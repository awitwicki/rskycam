use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Local, Timelike, Utc};
use futures::stream::{Stream, StreamExt};
use serde::{Deserialize, Deserializer, Serialize};
use tokio_stream::wrappers::ReceiverStream;

use crate::capture::{CaptureStatus, FrameMeta, LatestFrame};
use crate::overlay::{astro, geometry};
use crate::sensors::SensorStatus;
use crate::settings::{CropRect, LensCalibration, OverlayLayers, Settings};
use crate::system::SystemStatus;
use crate::web::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AstroStatus {
    pub sun_alt_deg: f64,
    pub moon_alt_deg: f64,
    pub moon_phase_pct: f64,
    pub moon_waxing: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub capture: CaptureStatus,
    pub sensor: SensorStatus,
    pub system: SystemStatus,
    pub astro: AstroStatus,
    pub camera: Option<crate::capture::CameraCaps>,
    pub darks_progress: Option<crate::darks::DarksProgress>,
}

fn astro_status(s: &Settings, now: DateTime<Utc>) -> AstroStatus {
    let (lat, lon) = (s.location.latitude_deg, s.location.longitude_deg);
    let sun = astro::sun_equatorial(now);
    let moon = astro::moon_equatorial(now);
    let ill = astro::moon_illumination(now);
    AstroStatus {
        sun_alt_deg: astro::altitude_of(now, sun.ra_deg, sun.dec_deg, lat, lon),
        moon_alt_deg: astro::altitude_of(now, moon.ra_deg, moon.dec_deg, lat, lon),
        moon_phase_pct: ill.pct,
        moon_waxing: ill.waxing,
    }
}

async fn build_status(state: &AppState) -> Status {
    let s = state.cfg.read().await.settings.clone();
    let capture = state.capture_status.borrow().clone();
    let data_dir = state.data_dir.clone();
    let sensor_enabled = s.sensor.enabled;
    // File I/O, the vcgencmd subprocess and I2C probing are blocking work —
    // keep them off the async workers (this runs per SSE tick).
    let (sensor, system) = tokio::task::spawn_blocking(move || {
        (
            crate::sensors::read_sensor(sensor_enabled),
            crate::system::read_system_status(&data_dir),
        )
    })
    .await
    .expect("status readers are panic-free by design");
    Status {
        capture,
        sensor,
        system,
        astro: astro_status(&s, Utc::now()),
        camera: state.camera_caps.borrow().clone(),
        darks_progress: *state.darks_progress.borrow(),
    }
}

pub async fn get_status(State(state): State<AppState>) -> Json<Status> {
    Json(build_status(&state).await)
}

#[derive(Deserialize)]
pub struct LatestQuery {
    #[serde(default)]
    raw: Option<u8>,
}

pub async fn latest_jpg(State(state): State<AppState>, Query(q): Query<LatestQuery>) -> Response {
    let Some(latest) = state.latest.borrow().clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let body = if q.raw == Some(1) {
        latest.raw_jpeg.clone()
    } else {
        latest.jpeg.clone()
    };
    (
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response()
}

pub fn frame_event_json(meta: &FrameMeta) -> String {
    serde_json::json!({
        "imageUrl": format!("/api/latest.jpg?ts={}", meta.timestamp),
        "meta": meta,
    })
    .to_string()
}

/// One SSE session per client: a dedicated task pushes `frame` events on every
/// new capture (plus the current frame immediately on connect) and a `status`
/// event every 2.5s, exiting cleanly when the client disconnects (send fails).
pub async fn events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Event>(16);
    let mut latest = state.latest.clone();

    tokio::spawn(async move {
        let current: Option<Arc<LatestFrame>> = latest.borrow().clone();
        if let Some(l) = current {
            let ev = Event::default()
                .event("frame")
                .data(frame_event_json(&l.meta));
            if tx.send(ev).await.is_err() {
                return;
            }
        }

        let mut tick = tokio::time::interval(Duration::from_millis(2500));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await; // interval's first tick fires immediately; consume it so status is every 2.5s, not instant

        loop {
            tokio::select! {
                changed = latest.changed() => {
                    if changed.is_err() {
                        break; // capture task's sender dropped
                    }
                    let meta = latest.borrow().as_ref().map(|l: &Arc<LatestFrame>| l.meta.clone());
                    if let Some(m) = meta {
                        let ev = Event::default().event("frame").data(frame_event_json(&m));
                        if tx.send(ev).await.is_err() {
                            break; // client disconnected
                        }
                    }
                }
                _ = tick.tick() => {
                    let status = build_status(&state).await;
                    let Ok(payload) = serde_json::to_string(&status) else {
                        continue;
                    };
                    let ev = Event::default().event("status").data(payload);
                    if tx.send(ev).await.is_err() {
                        break; // client disconnected
                    }
                }
            }
        }
    });

    Sse::new(ReceiverStream::new(rx).map(Ok)).keep_alive(KeepAlive::default())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LightgraphData {
    pub start_iso: String,
    pub step_minutes: u32,
    pub sun_alt_deg: Vec<f64>,
}

pub async fn get_lightgraph(State(state): State<AppState>) -> Json<LightgraphData> {
    let s = state.cfg.read().await.settings.clone();
    let now = Local::now();
    let mut start = now
        .with_hour(12)
        .and_then(|t| t.with_minute(0))
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(now);
    if now < start {
        start -= chrono::Duration::days(1);
    }
    let step_minutes = 10u32;
    let mut sun_alt_deg = Vec::with_capacity(144);
    for i in 0..144 {
        let t =
            (start + chrono::Duration::minutes(i as i64 * step_minutes as i64)).with_timezone(&Utc);
        let sun = astro::sun_equatorial(t);
        sun_alt_deg.push(astro::altitude_of(
            t,
            sun.ra_deg,
            sun.dec_deg,
            s.location.latitude_deg,
            s.location.longitude_deg,
        ));
    }
    Json(LightgraphData {
        start_iso: start.to_rfc3339(),
        step_minutes,
        sun_alt_deg,
    })
}

/// `crop` distinguishes absent (settings crop) from explicit null (sensor space).
fn double_option<'de, D>(d: D) -> Result<Option<Option<CropRect>>, D::Error>
where
    D: Deserializer<'de>,
{
    Deserialize::deserialize(d).map(Some)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayRequest {
    pub time: Option<String>,
    pub calibration: Option<LensCalibration>,
    pub layers: Option<OverlayLayers>,
    pub grid_opacity: Option<f64>,
    #[serde(default, deserialize_with = "double_option")]
    pub crop: Option<Option<CropRect>>,
}

pub async fn post_overlay(
    State(state): State<AppState>,
    Json(req): Json<OverlayRequest>,
) -> Json<geometry::OverlayGeometry> {
    let s = state.cfg.read().await.settings.clone();
    let latest = state.latest.borrow().clone();
    let (w, h) = latest
        .as_ref()
        .map(|l| (l.raw_width, l.raw_height))
        .unwrap_or((1280, 960));
    let time = req
        .time
        .as_deref()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);
    let calibration = req.calibration.unwrap_or(s.overlay.calibration);
    let layers = req.layers.unwrap_or(s.overlay.layers);
    let native_width = state
        .camera_caps
        .borrow()
        .as_ref()
        .map(|c| c.max_width)
        .unwrap_or(w);
    let mut geo = geometry::build_overlay_geometry(&geometry::BuildOptions {
        time,
        location: &s.location,
        calibration: &calibration,
        layers: &layers,
        grid_opacity: Some(req.grid_opacity.unwrap_or(s.overlay.grid_opacity)),
        image_width: w,
        image_height: h,
        native_width,
        mask: geometry::MaskCircle::from_image(&s.image),
    });
    let sensor = crate::sensors::read_sensor(s.sensor.enabled);
    let ctx = geometry::TextContext {
        local_time: time
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
        exposure_us: latest.as_ref().map(|l| l.meta.exposure_us),
        gain: latest.as_ref().map(|l| l.meta.gain),
        sensor_temp_c: sensor.reading.map(|r| r.temperature_c),
    };
    geometry::append_text_fields(&mut geo, &s.overlay.text_fields, &ctx);
    let crop = match req.crop {
        None => s.image.crop, // absent → settings
        Some(c) => c,         // explicit null or rect
    };
    Json(match crop {
        Some(c) => geometry::crop_geometry(geo, &c),
        None => geo,
    })
}

pub async fn get_settings(State(state): State<AppState>) -> Json<Settings> {
    Json(state.cfg.read().await.settings.clone())
}

pub async fn put_settings(State(state): State<AppState>, Json(new): Json<Settings>) -> Response {
    // Persist first, off the write lock and off the async runtime (blocking
    // file I/O); adopt in memory only once the new settings are on disk.
    let mut candidate = state.cfg.read().await.clone();
    candidate.settings = new;
    candidate.settings.sanitize();
    let store = state.store.clone();
    let to_save = candidate.clone();
    let saved = tokio::task::spawn_blocking(move || store.save(&to_save)).await;
    if !matches!(saved, Ok(Ok(()))) {
        tracing::error!("persisting settings failed");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    // Adopt only our field so a concurrent password change can't be clobbered
    // by our (possibly stale) snapshot of the rest of the config.
    state.cfg.write().await.settings = candidate.settings;
    StatusCode::NO_CONTENT.into_response()
}

pub async fn start_darks_capture(State(state): State<AppState>) -> Response {
    if state.capture_status.borrow().state == crate::capture::CaptureState::CameraUnavailable {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "camera is currently unavailable",
        )
            .into_response();
    }
    if state.darks_progress.borrow().is_some() {
        return (StatusCode::CONFLICT, "a darks sweep is already running").into_response();
    }
    match state.darks_cmd.try_send(()) {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(_) => (StatusCode::CONFLICT, "a darks sweep is already running").into_response(),
    }
}

/// The dimensions the dark library is keyed by: the *actual* captured frame
/// size (pre-crop, pre-mask), which is what the sweep writes under and what
/// `process_frame` looks up. Mock always renders its own fixed size and ASI
/// rounds the requested ROI to valid boundaries, so this routinely differs
/// from the configured `capture_width`/`capture_height` — those are only the
/// fallback for the window before the very first frame has arrived.
fn darks_library_dims(state: &AppState, s: &crate::settings::Settings) -> (u32, u32) {
    state
        .latest
        .borrow()
        .as_ref()
        .map(|l| (l.raw_width, l.raw_height))
        .unwrap_or((s.camera.capture_width, s.camera.capture_height))
}

pub async fn get_darks(State(state): State<AppState>) -> Json<crate::darks::DarksLibrary> {
    let s = state.cfg.read().await.settings.clone();
    let data_dir = state.data_dir.clone();
    let (width, height) = darks_library_dims(&state, &s);
    let lib = tokio::task::spawn_blocking(move || {
        let dir = crate::darks::library_dir(&data_dir, s.camera.driver, width, height);
        crate::darks::load_manifest(&dir)
    })
    .await
    .expect("darks manifest read is panic-free by design");
    Json(lib)
}

pub async fn delete_darks(State(state): State<AppState>) -> Response {
    let s = state.cfg.read().await.settings.clone();
    let data_dir = state.data_dir.clone();
    let (width, height) = darks_library_dims(&state, &s);
    let result = tokio::task::spawn_blocking(move || {
        crate::darks::clear_library(&data_dir, s.camera.driver, width, height)
    })
    .await;
    match result {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use crate::web::testing::{harness, login_cookie};

    async fn get_json(app: &axum::Router, cookie: &str, uri: &str) -> serde_json::Value {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{uri}");
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn status_matches_the_wire_contract() {
        let h = harness();
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;
        let v = get_json(&app, &cookie, "/api/status").await;
        assert_eq!(v["capture"]["state"], "idle");
        assert!(v["astro"]["sunAltDeg"].is_number());
        assert!(v["astro"]["moonPhasePct"].is_number());
        assert_eq!(v["sensor"]["state"], "not_detected"); // enabled by default, no hardware in tests
        assert!(v["sensor"]["reading"].is_null());
        assert!(v["system"]["ramTotalMb"].is_number());
    }

    #[tokio::test]
    async fn status_includes_camera_caps_when_present() {
        let h = crate::web::testing::harness();
        h.caps_tx
            .send(Some(crate::capture::CameraCaps {
                model: "ZWO ASI120MM Mini".into(),
                max_width: 1280,
                max_height: 960,
            }))
            .unwrap();
        let app = crate::web::router(h.state.clone());
        let cookie = crate::web::testing::login_cookie(&app).await;
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/status")
                    .header(axum::http::header::COOKIE, &cookie)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(
            &http_body_util::BodyExt::collect(res.into_body())
                .await
                .unwrap()
                .to_bytes(),
        )
        .unwrap();
        assert_eq!(v["camera"]["maxWidth"], 1280);
        assert_eq!(v["camera"]["model"], "ZWO ASI120MM Mini");
    }

    #[tokio::test]
    async fn latest_jpg_404_before_first_frame_then_serves_processed_and_raw() {
        let h = harness();
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;
        let none = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/latest.jpg")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(none.status(), StatusCode::NOT_FOUND);

        // publish a frame through the watch channel
        use crate::camera::{Camera, CaptureParams};
        let frame = crate::camera::mock::MockCamera::new()
            .capture(CaptureParams {
                exposure_us: 1_000_000,
                gain: 4.0,
            })
            .unwrap();
        let mut s = h.state.cfg.read().await.settings.clone();
        s.image.crop = Some(crate::settings::CropRect {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 480.0,
        });
        let (latest, _) = crate::capture::process_frame(
            &frame,
            &s,
            &h.state.data_dir,
            crate::settings::CameraDriver::Mock,
            true,
            None,
            1280,
        )
        .unwrap();
        h.latest_tx.send(Some(std::sync::Arc::new(latest))).unwrap();

        for (uri, w) in [("/api/latest.jpg", 640u32), ("/api/latest.jpg?raw=1", 1280)] {
            let res = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .header(header::COOKIE, &cookie)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK);
            assert_eq!(res.headers()[header::CONTENT_TYPE], "image/jpeg");
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(image::load_from_memory(&bytes).unwrap().width(), w);
        }
    }

    #[tokio::test]
    async fn overlay_uses_settings_and_honors_request_overrides() {
        let h = harness();
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;
        let post = |body: serde_json::Value| {
            let app = app.clone();
            let cookie = cookie.clone();
            async move {
                let res = app
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri("/api/overlay")
                            .header(header::COOKIE, cookie)
                            .header(header::CONTENT_TYPE, "application/json")
                            .body(Body::from(body.to_string()))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(res.status(), StatusCode::OK);
                let bytes = res.into_body().collect().await.unwrap().to_bytes();
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
            }
        };
        let g = post(serde_json::json!({})).await;
        assert!(g["polylines"].as_array().unwrap().len() > 10); // default layers all on
        assert_eq!(g["polylines"][0]["opacity"], 0.45);
        assert_eq!(g["imageWidth"], 1280); // no frame yet → default sensor dims
        let custom = post(serde_json::json!({
            "gridOpacity": 0.8,
            "layers": {"cardinal": false, "altAzGrid": true, "raDecGrid": false},
            "crop": null
        }))
        .await;
        assert_eq!(custom["polylines"][0]["opacity"], 0.8);
        assert!(custom["labels"]
            .as_array()
            .unwrap()
            .iter()
            .all(|l| l["layer"] != "cardinal"));
        let rect = post(serde_json::json!({
            "crop": {"x": 100.0, "y": 50.0, "width": 700.0, "height": 800.0}
        }))
        .await;
        assert_eq!(rect["imageWidth"], 700);
        assert_eq!(rect["imageHeight"], 800);

        // calibration override: the altAz horizon ring (polylines[0], per
        // alt_az_grid_has_3_circles_and_8_radials in overlay/geometry.rs) sits
        // at a radius from the frame center that scales with focalLengthMm.
        let max_radius_from_center = |geo: &serde_json::Value| -> f64 {
            let cx = geo["imageWidth"].as_f64().unwrap() / 2.0;
            let cy = geo["imageHeight"].as_f64().unwrap() / 2.0;
            geo["polylines"][0]["points"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    let x = p[0].as_f64().unwrap();
                    let y = p[1].as_f64().unwrap();
                    ((x - cx).powi(2) + (y - cy).powi(2)).sqrt()
                })
                .fold(0.0_f64, f64::max)
        };
        let narrow = post(serde_json::json!({
            "calibration": {
                "lensType": "fisheye", "focalLengthMm": 1.0, "pixelSizeUm": 3.75,
                "pointingAzDeg": 0.0, "pointingAltDeg": 90.0, "rollDeg": 0.0, "flip": false,
                "centerOffsetXPx": 0.0, "centerOffsetYPx": 0.0
            },
            "crop": null
        }))
        .await;
        let wide = post(serde_json::json!({
            "calibration": {
                "lensType": "fisheye", "focalLengthMm": 2.0, "pixelSizeUm": 3.75,
                "pointingAzDeg": 0.0, "pointingAltDeg": 90.0, "rollDeg": 0.0, "flip": false,
                "centerOffsetXPx": 0.0, "centerOffsetYPx": 0.0
            },
            "crop": null
        }))
        .await;
        assert_ne!(
            max_radius_from_center(&narrow),
            max_radius_from_center(&wide),
            "geometry must respond to a calibration override in the request body"
        );
    }

    #[tokio::test]
    async fn settings_roundtrip_and_lightgraph_shape() {
        let h = harness();
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;
        let mut s = get_json(&app, &cookie, "/api/settings").await;
        assert_eq!(s["location"]["latitudeDeg"], 50.45);
        s["location"]["latitudeDeg"] = serde_json::json!(48.85);
        let put = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/settings")
                    .header(header::COOKIE, &cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(s.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put.status(), StatusCode::NO_CONTENT);
        let again = get_json(&app, &cookie, "/api/settings").await;
        assert_eq!(again["location"]["latitudeDeg"], 48.85);

        let lg = get_json(&app, &cookie, "/api/lightgraph").await;
        assert_eq!(lg["stepMinutes"], 10);
        assert_eq!(lg["sunAltDeg"].as_array().unwrap().len(), 144);
    }

    #[tokio::test]
    async fn events_stream_delivers_the_current_frame_immediately_on_connect() {
        let h = harness();
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;

        use crate::camera::{Camera, CaptureParams};
        let frame = crate::camera::mock::MockCamera::new()
            .capture(CaptureParams {
                exposure_us: 1_000_000,
                gain: 4.0,
            })
            .unwrap();
        let s = h.state.cfg.read().await.settings.clone();
        let (latest, _) = crate::capture::process_frame(
            &frame,
            &s,
            &h.state.data_dir,
            crate::settings::CameraDriver::Mock,
            true,
            None,
            1280,
        )
        .unwrap();
        h.latest_tx.send(Some(std::sync::Arc::new(latest))).unwrap();

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/api/events")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "text/event-stream");

        let mut body = res.into_body().into_data_stream();
        use futures::StreamExt;
        let first = tokio::time::timeout(std::time::Duration::from_secs(2), body.next())
            .await
            .expect("no SSE data within 2s")
            .expect("stream ended before any event")
            .unwrap();
        let text = String::from_utf8(first.to_vec()).unwrap();
        assert!(text.contains("event: frame"), "got: {text}");
        assert!(text.contains("imageUrl"), "got: {text}");
    }

    #[tokio::test]
    async fn darks_capture_start_lists_and_clears() {
        let h = crate::web::testing::harness();
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;

        let start = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/darks/capture")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(start.status(), StatusCode::ACCEPTED);

        // A second request while a sweep is (simulated as) already running is rejected.
        h.darks_progress_tx
            .send(Some(crate::darks::DarksProgress {
                current: 1,
                total: 15,
            }))
            .unwrap();
        let again = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/darks/capture")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(again.status(), StatusCode::CONFLICT);
        h.darks_progress_tx.send(None).unwrap();

        let listed = get_json(&app, &cookie, "/api/darks").await;
        assert!(listed["entries"].as_array().unwrap().is_empty());

        let del = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/darks")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del.status(), StatusCode::NO_CONTENT);
    }

    /// The dark library is keyed by the dimensions the camera *actually*
    /// captures at, not the configured `captureWidth`/`captureHeight` — those
    /// routinely differ (mock always renders 1280x960; ASI rounds the ROI to
    /// valid boundaries). Seeding a library at the frame's dimensions and
    /// finding it through the API proves the handler follows the frame:
    /// keying by the configured values (1640x1232 by default) would find
    /// nothing.
    #[tokio::test]
    async fn darks_list_and_delete_key_off_the_captured_frame_dimensions() {
        let h = crate::web::testing::harness();
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;

        // The frames below come from MockCamera, so key the library by that
        // driver (the handlers take the driver from settings).
        let configured = {
            let s = &mut h.state.cfg.write().await.settings;
            s.camera.driver = crate::settings::CameraDriver::Mock;
            (s.camera.capture_width, s.camera.capture_height)
        };
        assert_ne!(
            configured,
            (1280, 960),
            "test needs configured != captured dimensions to be meaningful"
        );

        // Seed a library at the *captured* dimensions, as the sweep would.
        crate::darks::add_entry(
            &h.state.data_dir,
            crate::settings::CameraDriver::Mock,
            1280,
            960,
            20_000_000,
            16.0,
            &image::RgbImage::from_pixel(4, 4, image::Rgb([1, 2, 3])),
            chrono::Utc::now(),
        )
        .unwrap();

        // Before any frame has arrived we fall back to the configured size,
        // so the seeded library is (correctly) not visible yet.
        let empty = get_json(&app, &cookie, "/api/darks").await;
        assert!(empty["entries"].as_array().unwrap().is_empty());

        // Publish a frame captured at 1280x960 (what mock really renders).
        use crate::camera::{Camera, CaptureParams};
        let frame = crate::camera::mock::MockCamera::new()
            .capture(CaptureParams {
                exposure_us: 1_000_000,
                gain: 4.0,
            })
            .unwrap();
        let s = h.state.cfg.read().await.settings.clone();
        let (latest, _) = crate::capture::process_frame(
            &frame,
            &s,
            &h.state.data_dir,
            crate::settings::CameraDriver::Mock,
            true,
            None,
            1280,
        )
        .unwrap();
        assert_eq!((latest.raw_width, latest.raw_height), (1280, 960));
        h.latest_tx.send(Some(std::sync::Arc::new(latest))).unwrap();

        let listed = get_json(&app, &cookie, "/api/darks").await;
        let entries = listed["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1, "got: {listed}");
        assert_eq!(entries[0]["exposureUs"], 20_000_000u64);

        // DELETE must clear that same library, not the configured-size one.
        let del = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/darks")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del.status(), StatusCode::NO_CONTENT);
        assert!(!crate::darks::library_dir(
            &h.state.data_dir,
            crate::settings::CameraDriver::Mock,
            1280,
            960
        )
        .exists());
        let after = get_json(&app, &cookie, "/api/darks").await;
        assert!(after["entries"].as_array().unwrap().is_empty());
    }

    /// A sweep request while the camera is down is rejected outright rather
    /// than queued and silently dropped by the supervisor.
    #[tokio::test]
    async fn darks_capture_is_rejected_while_the_camera_is_unavailable() {
        let h = crate::web::testing::harness();
        h.status_tx
            .send(crate::capture::CaptureStatus {
                state: crate::capture::CaptureState::CameraUnavailable,
                message: Some("injected".into()),
                last_frame: None,
            })
            .unwrap();
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/darks/capture")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn status_includes_darks_progress() {
        let h = crate::web::testing::harness();
        h.darks_progress_tx
            .send(Some(crate::darks::DarksProgress {
                current: 3,
                total: 15,
            }))
            .unwrap();
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;
        let v = get_json(&app, &cookie, "/api/status").await;
        assert_eq!(v["darksProgress"]["current"], 3);
        assert_eq!(v["darksProgress"]["total"], 15);
    }

    #[test]
    fn frame_event_payload_shape() {
        let meta = crate::capture::FrameMeta {
            timestamp: "2026-07-15T22:00:00Z".into(),
            exposure_us: 30_000_000,
            gain: 8.0,
            is_night: true,
        };
        let v: serde_json::Value = serde_json::from_str(&super::frame_event_json(&meta)).unwrap();
        assert!(v["imageUrl"]
            .as_str()
            .unwrap()
            .starts_with("/api/latest.jpg?ts="));
        assert_eq!(v["meta"]["exposureUs"], 30_000_000);
        assert_eq!(v["meta"]["isNight"], true);
    }
}
