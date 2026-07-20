use std::path::{Component, Path, PathBuf};

use axum::extract::{Path as AxumPath, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::settings::ProcessingSettings;
use crate::web::AppState;

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ArtifactState {
    Ready { url: String },
    // Emitted once background artifact generation lands in Phase 3.
    Generating,
    Error { message: String },
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
    pub thumbnail_url: String,
    pub keogram: ArtifactState,
    pub startrails: ArtifactState,
    pub timelapse: ArtifactState,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameInfo {
    pub timestamp: String,
    pub url: String,
    pub exposure_us: u64,
    pub gain: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NightDetail {
    pub date: String,
    pub frame_count: u64,
    pub thumbnail_url: String,
    pub keogram: ArtifactState,
    pub startrails: ArtifactState,
    pub timelapse: ArtifactState,
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
        None => {
            if night_dir.join(file).is_file() {
                ArtifactState::Ready {
                    url: format!("/api/files/{date}/{file}"),
                }
            } else {
                ArtifactState::Pending
            }
        }
    }
}

fn read_frames(night_dir: &Path, date: &str) -> Vec<FrameInfo> {
    let Ok(raw) = std::fs::read_to_string(night_dir.join("frames.jsonl")) else {
        return Vec::new();
    };
    raw.lines()
        .filter_map(|l| serde_json::from_str::<FrameLine>(l).ok())
        .map(|f| FrameInfo {
            url: format!("/api/files/{date}/frames/{}", f.file),
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
        thumbnail_url: frames.last().map(|f| f.url.clone()).unwrap_or_default(),
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
        timelapse: artifact(
            night_dir,
            date,
            "timelapse.mp4",
            processing.timelapse,
            st.timelapse.as_ref(),
        ),
    }
}

fn is_date(s: &str) -> bool {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

pub async fn get_nights(State(state): State<AppState>) -> Json<Vec<NightSummary>> {
    let processing = state.cfg.read().await.settings.processing.clone();
    let images = state.data_dir.join("images");
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
    Json(
        dates
            .iter()
            .map(|d| summary(&images.join(d), d, &processing))
            .collect(),
    )
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
    let s = summary(&night_dir, &date, &processing);
    let frames = read_frames(&night_dir, &date);
    Json(NightDetail {
        date: s.date,
        frame_count: s.frame_count,
        thumbnail_url: s.thumbnail_url,
        keogram: s.keogram,
        startrails: s.startrails,
        timelapse: s.timelapse,
        frames,
    })
    .into_response()
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

pub async fn get_file(
    State(state): State<AppState>,
    AxumPath((date, path)): AxumPath<(String, String)>,
) -> Response {
    if !is_date(&date) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let rel = PathBuf::from(&path);
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let full = state.data_dir.join("images").join(&date).join(rel);
    let Ok(bytes) = std::fs::read(&full) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mime = match full.extension().and_then(|e| e.to_str()) {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("mp4") => "video/mp4",
        Some("png") => "image/png",
        _ => "application/octet-stream",
    };
    ([(header::CONTENT_TYPE, mime)], bytes).into_response()
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

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
        assert_eq!(v[0]["startrails"]["state"], "pending");
        assert_eq!(v[0]["timelapse"]["state"], "pending");
        assert!(v[0]["thumbnailUrl"]
            .as_str()
            .unwrap()
            .starts_with("/api/files/2026-07-14/frames/"));

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
        assert_eq!(d["frames"][0]["exposureUs"], 30_000_000);

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
        assert_eq!(d["timelapse"]["state"], "pending"); // still enabled, not generated
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
                timelapse: Some(crate::processing::status::ArtifactProgress::Error {
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
        assert_eq!(d["timelapse"]["state"], "error");
        assert_eq!(d["timelapse"]["message"], "no space left");
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
                if night.join("timelapse.mp4").is_file() && night.join("startrails.jpg").is_file() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("rebuild did not produce artifacts within 10s");
    }
}
