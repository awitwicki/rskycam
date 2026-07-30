use std::path::{Component, Path, PathBuf};

use anyhow::Context;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::settings::ProcessingSettings;
use crate::web::AppState;

/// Thumbnails are square crops of this side length, in pixels. The frame
/// grid renders tiles well above 100px on most viewports (up to ~265px on a
/// 1920px-wide screen), so a 100px source was visibly blurry when upscaled;
/// 200px covers typical grid tile sizes without over-fetching for a
/// 1000+-frame night.
const THUMBNAIL_SIZE: u32 = 200;

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ArtifactState {
    #[serde(rename_all = "camelCase")]
    Ready {
        url: String,
        size_bytes: u64,
    },
    // Emitted once background artifact generation lands in Phase 3.
    Generating,
    Error {
        message: String,
    },
    // Enabled in settings but not produced yet (generation lands in Phase 3).
    Pending,
    // Turned off in settings.
    Disabled,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NightSummary {
    pub date: String,
    pub frame_count: u64,
    pub frames_size_bytes: u64,
    pub total_size_bytes: u64,
    pub thumbnail_url: String,
    pub keogram: ArtifactState,
    pub startrails: ArtifactState,
    pub timelapse_day: ArtifactState,
    pub timelapse_night: ArtifactState,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameInfo {
    pub timestamp: String,
    pub url: String,
    pub thumb_url: String,
    pub exposure_us: u64,
    pub gain: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NightDetail {
    pub date: String,
    pub frame_count: u64,
    pub frames_size_bytes: u64,
    pub total_size_bytes: u64,
    pub thumbnail_url: String,
    pub keogram: ArtifactState,
    pub startrails: ArtifactState,
    pub timelapse_day: ArtifactState,
    pub timelapse_night: ArtifactState,
    pub frames: Vec<FrameInfo>,
}

#[derive(Deserialize)]
struct FrameLine {
    timestamp: String,
    file: String,
    #[serde(rename = "exposureUs")]
    exposure_us: u64,
    gain: f64,
}

fn artifact(
    night_dir: &Path,
    date: &str,
    file: &str,
    enabled: bool,
    progress: Option<&crate::processing::status::ArtifactProgress>,
) -> ArtifactState {
    if !enabled {
        return ArtifactState::Disabled;
    }
    match progress {
        Some(crate::processing::status::ArtifactProgress::Error { message }) => {
            ArtifactState::Error {
                message: message.clone(),
            }
        }
        Some(crate::processing::status::ArtifactProgress::Generating) => ArtifactState::Generating,
        None => match std::fs::metadata(night_dir.join(file)) {
            Ok(meta) => ArtifactState::Ready {
                url: format!("/api/files/{date}/{file}"),
                size_bytes: meta.len(),
            },
            Err(_) => ArtifactState::Pending,
        },
    }
}

/// Recursively sums the size of every regular file under `dir`. Best-effort:
/// a missing or unreadable directory/entry contributes 0 rather than
/// erroring the whole request.
fn dir_size_bytes(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| {
            let path = e.path();
            if path.is_dir() {
                dir_size_bytes(&path)
            } else {
                std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
            }
        })
        .sum()
}

/// Newest-first (frames.jsonl is append order, i.e. oldest-first on disk).
fn read_frames(night_dir: &Path, date: &str) -> Vec<FrameInfo> {
    let Ok(raw) = std::fs::read_to_string(night_dir.join("frames.jsonl")) else {
        return Vec::new();
    };
    raw.lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<FrameLine>(l).ok())
        .map(|f| FrameInfo {
            url: format!("/api/files/{date}/frames/{}", f.file),
            thumb_url: format!("/api/files/{date}/frames/{}?thumb=1", f.file),
            timestamp: f.timestamp,
            exposure_us: f.exposure_us,
            gain: f.gain,
        })
        .collect()
}

fn summary(night_dir: &Path, date: &str, processing: &ProcessingSettings) -> NightSummary {
    let frames = read_frames(night_dir, date);
    let st: crate::processing::status::NightProcessingStatus =
        crate::processing::status::load(night_dir);
    NightSummary {
        date: date.to_string(),
        frame_count: frames.len() as u64,
        frames_size_bytes: dir_size_bytes(&night_dir.join("frames")),
        total_size_bytes: dir_size_bytes(night_dir),
        // Full image, not the small thumbnail: this is the one "cover" image
        // per night (not per-frame), so bandwidth isn't a concern, and it's
        // rendered noticeably larger than THUMBNAIL_SIZE — a thumbnail there
        // just looks blurry from upscaling.
        thumbnail_url: frames.first().map(|f| f.url.clone()).unwrap_or_default(),
        keogram: artifact(
            night_dir,
            date,
            "keogram.jpg",
            processing.keogram,
            st.keogram.as_ref(),
        ),
        startrails: artifact(
            night_dir,
            date,
            "startrails.jpg",
            processing.startrails,
            st.startrails.as_ref(),
        ),
        timelapse_day: artifact(
            night_dir,
            date,
            "timelapse-day.mp4",
            processing.timelapse_day,
            st.timelapse_day.as_ref(),
        ),
        timelapse_night: artifact(
            night_dir,
            date,
            "timelapse-night.mp4",
            processing.timelapse_night,
            st.timelapse_night.as_ref(),
        ),
    }
}

fn is_date(s: &str) -> bool {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

pub async fn get_nights(State(state): State<AppState>) -> Json<Vec<NightSummary>> {
    let processing = state.cfg.read().await.settings.processing.clone();
    let images = state.data_dir.join("images");
    // Summaries now walk each night's directory tree to size it up, which is
    // real blocking disk I/O for a night with hundreds/thousands of frames —
    // must not run inline on the async runtime.
    let summaries = tokio::task::spawn_blocking(move || {
        let mut dates: Vec<String> = std::fs::read_dir(&images)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .filter(|n| is_date(n))
                    .collect()
            })
            .unwrap_or_default();
        dates.sort_unstable_by(|a, b| b.cmp(a)); // newest first
        dates
            .iter()
            .map(|d| summary(&images.join(d), d, &processing))
            .collect::<Vec<_>>()
    })
    .await
    .expect("nights listing is panic-free by design");
    Json(summaries)
}

pub async fn get_night(
    State(state): State<AppState>,
    AxumPath(date): AxumPath<String>,
) -> Response {
    let night_dir = state.data_dir.join("images").join(&date);
    if !is_date(&date) || !night_dir.is_dir() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let processing = state.cfg.read().await.settings.processing.clone();
    let detail = tokio::task::spawn_blocking(move || {
        let s = summary(&night_dir, &date, &processing);
        let frames = read_frames(&night_dir, &date);
        NightDetail {
            date: s.date,
            frame_count: s.frame_count,
            frames_size_bytes: s.frames_size_bytes,
            total_size_bytes: s.total_size_bytes,
            thumbnail_url: s.thumbnail_url,
            keogram: s.keogram,
            startrails: s.startrails,
            timelapse_day: s.timelapse_day,
            timelapse_night: s.timelapse_night,
            frames,
        }
    })
    .await
    .expect("night detail read is panic-free by design");
    Json(detail).into_response()
}

pub async fn rebuild_night(
    State(state): State<AppState>,
    AxumPath(date): AxumPath<String>,
) -> StatusCode {
    let night_dir = state.data_dir.join("images").join(&date);
    let Ok(parsed) = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d") else {
        return StatusCode::NOT_FOUND;
    };
    if !night_dir.is_dir() {
        return StatusCode::NOT_FOUND;
    }
    match state
        .processing
        .commands
        .try_send(crate::processing::Command::Rebuild { date: parsed })
    {
        Ok(()) => StatusCode::ACCEPTED,
        Err(e) => {
            tracing::warn!("rebuild for {date} rejected: {e}");
            StatusCode::SERVICE_UNAVAILABLE // queue full — try again shortly
        }
    }
}

/// Permanently delete a night's directory (frames, artifacts, everything).
/// The date is validated to a strict `YYYY-MM-DD` first, so the joined path
/// can never escape `images/`. The UI guards this behind a confirmation.
pub async fn delete_night(
    State(state): State<AppState>,
    AxumPath(date): AxumPath<String>,
) -> StatusCode {
    if !is_date(&date) {
        return StatusCode::NOT_FOUND;
    }
    let night_dir = state.data_dir.join("images").join(&date);
    if !night_dir.is_dir() {
        return StatusCode::NOT_FOUND;
    }
    match tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&night_dir)).await {
        Ok(Ok(())) => {
            tracing::info!("deleted night {date}");
            StatusCode::NO_CONTENT
        }
        Ok(Err(e)) => {
            tracing::error!("deleting night {date}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
        Err(e) => {
            tracing::error!("delete task panicked for {date}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

#[derive(Deserialize)]
pub struct FileQuery {
    /// `?thumb=1` serves (generating and caching on first request) a small
    /// square crop instead of the full file — only meaningful for JPEGs.
    #[serde(default)]
    thumb: Option<u8>,
}

/// Center-crop `bytes` (a JPEG) to a square and shrink it to
/// `THUMBNAIL_SIZE`x`THUMBNAIL_SIZE`, re-encoded as JPEG.
fn make_thumbnail(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let img = image::load_from_memory(bytes)
        .context("decoding source image")?
        .to_rgb8();
    let (w, h) = img.dimensions();
    let side = w.min(h);
    let cropped =
        image::imageops::crop_imm(&img, (w - side) / 2, (h - side) / 2, side, side).to_image();
    let thumb = image::imageops::resize(
        &cropped,
        THUMBNAIL_SIZE,
        THUMBNAIL_SIZE,
        image::imageops::FilterType::Triangle,
    );
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 80)
        .encode_image(&thumb)
        .context("encoding thumbnail")?;
    Ok(buf)
}

/// The cached-thumbnail path for a frame file: a `thumbs<SIZE>/` subdirectory
/// alongside it, so it's removed for free whenever the containing night is
/// (retention, manual delete) — nothing extra to clean up. The size is
/// baked into the directory name so bumping `THUMBNAIL_SIZE` can't serve a
/// stale, wrong-size cached thumbnail — it just starts a fresh directory.
fn thumbnail_cache_path(full: &Path) -> anyhow::Result<PathBuf> {
    Ok(full
        .parent()
        .context("file has no parent directory")?
        .join(format!("thumbs{THUMBNAIL_SIZE}"))
        .join(full.file_name().context("file has no name")?))
}

/// Serves the cached thumbnail if present, otherwise generates one from the
/// full file, caches it to disk, and returns it. Blocking — call from
/// `spawn_blocking`.
fn load_or_make_thumbnail(full: &Path) -> anyhow::Result<Vec<u8>> {
    let cache = thumbnail_cache_path(full)?;
    if let Ok(cached) = std::fs::read(&cache) {
        return Ok(cached);
    }
    let source = std::fs::read(full).context("reading source image")?;
    let thumb = make_thumbnail(&source)?;
    if let Some(dir) = cache.parent() {
        std::fs::create_dir_all(dir).context("creating thumbnail cache dir")?;
    }
    std::fs::write(&cache, &thumb).context("writing thumbnail cache")?;
    Ok(thumb)
}

pub async fn get_file(
    State(state): State<AppState>,
    AxumPath((date, path)): AxumPath<(String, String)>,
    Query(q): Query<FileQuery>,
) -> Response {
    if !is_date(&date) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let rel = PathBuf::from(&path);
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let full = state.data_dir.join("images").join(&date).join(rel);
    let mime = match full.extension().and_then(|e| e.to_str()) {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("mp4") => "video/mp4",
        Some("png") => "image/png",
        _ => "application/octet-stream",
    };
    let want_thumb = q.thumb == Some(1) && mime == "image/jpeg";
    let read = tokio::task::spawn_blocking(move || {
        if want_thumb {
            load_or_make_thumbnail(&full)
        } else {
            std::fs::read(&full).context("reading file")
        }
    })
    .await;
    let Ok(Ok(bytes)) = read else {
        return StatusCode::NOT_FOUND.into_response();
    };
    ([(header::CONTENT_TYPE, mime)], bytes).into_response()
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use crate::web::nights::{make_thumbnail, THUMBNAIL_SIZE};
    use crate::web::testing::{harness, login_cookie};

    fn seed_night(data_dir: &std::path::Path, date: &str) {
        let night = data_dir.join("images").join(date);
        std::fs::create_dir_all(night.join("frames")).unwrap();
        for (i, name) in ["220000.jpg", "220100.jpg"].iter().enumerate() {
            let file = format!("20260714-{name}");
            let img = image::RgbImage::from_pixel(8, 6, image::Rgb([10, 10, 10]));
            img.save_with_format(night.join("frames").join(&file), image::ImageFormat::Jpeg)
                .unwrap();
            let line = serde_json::json!({
                "timestamp": format!("2026-07-14T22:0{i}:00Z"),
                "file": file, "exposureUs": 30_000_000, "gain": 8.0,
            });
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(night.join("frames.jsonl"))
                .unwrap();
            writeln!(f, "{line}").unwrap();
        }
        std::fs::write(night.join("keogram.jpg"), b"\xFF\xD8fake").unwrap();
    }

    #[tokio::test]
    async fn lists_nights_with_artifact_states_and_detail() {
        let h = harness();
        seed_night(&h.state.data_dir, "2026-07-14");
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/nights")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let v: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert_eq!(v[0]["date"], "2026-07-14");
        assert_eq!(v[0]["frameCount"], 2);
        // keogram file was seeded → ready; the others are enabled by default
        // but not generated yet → pending (not "disabled").
        assert_eq!(v[0]["keogram"]["state"], "ready");
        assert_eq!(v[0]["keogram"]["sizeBytes"], 6); // b"\xFF\xD8fake"
        assert_eq!(v[0]["startrails"]["state"], "pending");
        assert_eq!(v[0]["timelapseDay"]["state"], "pending");
        assert_eq!(v[0]["timelapseNight"]["state"], "pending");
        // Two seeded frames, non-zero bytes each; total includes keogram.jpg too.
        assert!(v[0]["framesSizeBytes"].as_u64().unwrap() > 0);
        assert!(
            v[0]["totalSizeBytes"].as_u64().unwrap() > v[0]["framesSizeBytes"].as_u64().unwrap()
        );
        assert!(v[0]["thumbnailUrl"]
            .as_str()
            .unwrap()
            .starts_with("/api/files/2026-07-14/frames/"));
        // The night's cover image is the full-resolution frame, not the
        // small per-frame grid thumbnail (it renders much larger than
        // THUMBNAIL_SIZE on the nights list, so a thumbnail there is blurry).
        assert!(!v[0]["thumbnailUrl"].as_str().unwrap().contains("thumb=1"));

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/nights/2026-07-14")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let d: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(d["frames"].as_array().unwrap().len(), 2);
        assert!(d["framesSizeBytes"].as_u64().unwrap() > 0);
        assert!(d["totalSizeBytes"].as_u64().unwrap() > d["framesSizeBytes"].as_u64().unwrap());
        assert_eq!(d["frames"][0]["exposureUs"], 30_000_000);
        // Newest first: 22:01 (seeded second) before 22:00 (seeded first).
        assert_eq!(d["frames"][0]["timestamp"], "2026-07-14T22:01:00Z");
        assert_eq!(d["frames"][1]["timestamp"], "2026-07-14T22:00:00Z");
        assert!(d["frames"][0]["thumbUrl"]
            .as_str()
            .unwrap()
            .contains("thumb=1"));

        let missing = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/nights/1999-01-01")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn disabled_processing_setting_reports_the_artifact_as_disabled() {
        let h = harness();
        seed_night(&h.state.data_dir, "2026-07-14");
        // Turn keogram off in settings; the seeded keogram.jpg must not make it "ready".
        h.state.cfg.write().await.settings.processing.keogram = false;
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/api/nights/2026-07-14")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let d: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(d["keogram"]["state"], "disabled"); // off in settings
        assert_eq!(d["timelapseDay"]["state"], "pending"); // still enabled, not generated
        assert_eq!(d["timelapseNight"]["state"], "pending"); // still enabled, not generated
    }

    #[tokio::test]
    async fn rebuild_known_202_unknown_404() {
        let h = harness();
        seed_night(&h.state.data_dir, "2026-07-14");
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;
        let ok = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/nights/2026-07-14/rebuild")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::ACCEPTED);
        let missing = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/nights/1999-01-01/rebuild")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_known_204_removes_dir_unknown_404() {
        let h = harness();
        seed_night(&h.state.data_dir, "2026-07-14");
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;
        let night = h.state.data_dir.join("images").join("2026-07-14");
        assert!(night.is_dir());

        let ok = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/nights/2026-07-14")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::NO_CONTENT);
        assert!(!night.exists(), "night dir was not removed");

        // A second delete (or an unknown date) is a 404.
        let gone = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/nights/2026-07-14")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(gone.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_requires_a_session() {
        let h = harness();
        seed_night(&h.state.data_dir, "2026-07-14");
        let app = crate::web::router(h.state.clone());
        let anon = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/nights/2026-07-14")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(anon.status(), StatusCode::UNAUTHORIZED);
        // The dir must still be there — an unauthenticated call deletes nothing.
        assert!(h.state.data_dir.join("images").join("2026-07-14").is_dir());
    }

    #[test]
    fn make_thumbnail_center_crops_to_square_and_resizes() {
        let img = image::RgbImage::from_pixel(400, 200, image::Rgb([10, 20, 30]));
        let mut buf = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 90)
            .encode_image(&img)
            .unwrap();
        let thumb_bytes = make_thumbnail(&buf).unwrap();
        let decoded = image::load_from_memory(&thumb_bytes).unwrap();
        assert_eq!(
            (decoded.width(), decoded.height()),
            (THUMBNAIL_SIZE, THUMBNAIL_SIZE)
        );
    }

    #[tokio::test]
    async fn thumbnails_are_cached_and_smaller_than_the_source() {
        let h = harness();
        let night = h.state.data_dir.join("images").join("2026-07-14");
        std::fs::create_dir_all(night.join("frames")).unwrap();
        let file = "20260714-220000.jpg";
        let img = image::RgbImage::from_pixel(400, 300, image::Rgb([120, 80, 40]));
        img.save_with_format(night.join("frames").join(file), image::ImageFormat::Jpeg)
            .unwrap();

        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;
        let get = |thumb: bool| {
            let app = app.clone();
            let cookie = cookie.clone();
            async move {
                let uri = if thumb {
                    format!("/api/files/2026-07-14/frames/{file}?thumb=1")
                } else {
                    format!("/api/files/2026-07-14/frames/{file}")
                };
                app.oneshot(
                    Request::builder()
                        .uri(uri)
                        .header(header::COOKIE, cookie)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
            }
        };

        let full = get(false).await;
        assert_eq!(full.status(), StatusCode::OK);
        let full_bytes = full.into_body().collect().await.unwrap().to_bytes();

        let thumb1 = get(true).await;
        assert_eq!(thumb1.status(), StatusCode::OK);
        assert_eq!(thumb1.headers()[header::CONTENT_TYPE], "image/jpeg");
        let thumb1_bytes = thumb1.into_body().collect().await.unwrap().to_bytes();
        assert!(thumb1_bytes.len() < full_bytes.len());
        let decoded = image::load_from_memory(&thumb1_bytes).unwrap();
        assert_eq!(
            (decoded.width(), decoded.height()),
            (THUMBNAIL_SIZE, THUMBNAIL_SIZE)
        );

        // Cached to disk after the first request.
        let cache_path = night
            .join("frames")
            .join(format!("thumbs{THUMBNAIL_SIZE}"))
            .join(file);
        assert!(cache_path.is_file());

        // Second request serves the exact same (cached) bytes.
        let thumb2 = get(true).await;
        let thumb2_bytes = thumb2.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(thumb1_bytes, thumb2_bytes);
    }

    #[tokio::test]
    async fn file_serving_is_guarded_and_traversal_safe() {
        let h = harness();
        seed_night(&h.state.data_dir, "2026-07-14");
        let app = crate::web::router(h.state.clone());
        let anon = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/files/2026-07-14/keogram.jpg")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(anon.status(), StatusCode::UNAUTHORIZED);
        let cookie = login_cookie(&app).await;
        let ok = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/files/2026-07-14/keogram.jpg")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(ok.headers()[header::CONTENT_TYPE], "image/jpeg");
        let evil = app
            .oneshot(
                Request::builder()
                    .uri("/api/files/2026-07-14/..%2F..%2Fconfig.toml")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(evil.status() == StatusCode::BAD_REQUEST || evil.status() == StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn processing_status_file_surfaces_generating_and_error() {
        let h = harness();
        seed_night(&h.state.data_dir, "2026-07-14");
        let night = h.state.data_dir.join("images").join("2026-07-14");
        crate::processing::status::save(
            &night,
            &crate::processing::status::NightProcessingStatus {
                startrails: Some(crate::processing::status::ArtifactProgress::Generating),
                timelapse_day: Some(crate::processing::status::ArtifactProgress::Error {
                    message: "no space left".into(),
                }),
                ..Default::default()
            },
        )
        .unwrap();
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/api/nights/2026-07-14")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let d: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(d["keogram"]["state"], "ready"); // file exists, no progress entry
        assert_eq!(d["startrails"]["state"], "generating");
        assert_eq!(d["timelapseDay"]["state"], "error");
        assert_eq!(d["timelapseDay"]["message"], "no space left");
        assert_eq!(d["timelapseNight"]["state"], "pending"); // untouched by this status entry
    }

    #[tokio::test]
    async fn rebuild_endpoint_regenerates_artifacts_through_the_processor() {
        let h = harness();
        seed_night(&h.state.data_dir, "2026-07-14");
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/nights/2026-07-14/rebuild")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);
        let night = h.state.data_dir.join("images").join("2026-07-14");
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if night.join("timelapse-night.mp4").is_file()
                    && night.join("startrails.jpg").is_file()
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("rebuild did not produce artifacts within 10s");
    }
}
