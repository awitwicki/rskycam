pub mod auto_exposure;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chrono::{DateTime, Local, NaiveDate, Utc};
use image::RgbImage;
use serde::Serialize;
use tokio::sync::{watch, RwLock};

use crate::camera::{
    apply_crop, apply_mask_circle, encode_jpeg, mean_brightness, mock::MockCamera,
    rpicam::RpiCamera, Camera, CameraError, CaptureParams, Frame,
};
use crate::overlay::astro;
use crate::overlay::geometry;
use crate::settings::{CameraDriver, ConfigFile, MaskMode, Settings};

pub const NIGHT_SUN_ALT_DEG: f64 = -6.0;

/// Delay between metering frames while auto-exposure is still converging —
/// short so the exposure settles in seconds rather than one capture interval
/// per step.
const METER_INTERVAL: Duration = Duration::from_secs(1);

/// Night = local noon-to-noon, dated by the evening.
pub fn night_date(local: DateTime<Local>) -> NaiveDate {
    if local.time() < chrono::NaiveTime::from_hms_opt(12, 0, 0).expect("valid time") {
        local.date_naive().pred_opt().expect("valid date")
    } else {
        local.date_naive()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameMeta {
    pub timestamp: String, // ISO 8601
    pub exposure_us: u64,
    pub gain: f64,
    pub is_night: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureState {
    Capturing,
    CameraUnavailable,
    Idle,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    pub state: CaptureState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_frame: Option<FrameMeta>,
}

pub struct LatestFrame {
    pub jpeg: Bytes,         // masked + cropped, clean — what the dashboard shows
    pub persist_jpeg: Bytes, // what goes to disk: overlay-baked when enabled, else == jpeg
    pub raw_jpeg: Bytes,     // full sensor frame (mask applied, no crop) for the editor
    pub raw_width: u32,
    pub raw_height: u32,
    pub meta: FrameMeta,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraCaps {
    pub model: String,
    pub max_width: u32,
    pub max_height: u32,
}

impl From<&crate::camera::CameraInfo> for CameraCaps {
    fn from(i: &crate::camera::CameraInfo) -> Self {
        CameraCaps {
            model: i.model.clone(),
            max_width: i.max_width,
            max_height: i.max_height,
        }
    }
}

pub struct CaptureChannels {
    pub latest: watch::Receiver<Option<Arc<LatestFrame>>>,
    pub status: watch::Receiver<CaptureStatus>,
    pub camera_caps: watch::Receiver<Option<CameraCaps>>,
}

pub fn is_night(now: DateTime<Utc>, lat: f64, lon: f64) -> bool {
    let sun = astro::sun_equatorial(now);
    astro::altitude_of(now, sun.ra_deg, sun.dec_deg, lat, lon) < NIGHT_SUN_ALT_DEG
}

/// Raw + processed JPEGs and metadata for one frame. Pure — unit-tested.
///
/// Returns the [`LatestFrame`] alongside the clean processed image (masked +
/// cropped, pre-bake) so callers that need pixel data (e.g. the keogram/
/// startrails tap) don't have to re-decode a JPEG.
pub fn process_frame(
    frame: &Frame,
    s: &Settings,
    is_night: bool,
    sensor_temp_c: Option<f64>,
) -> Result<(LatestFrame, RgbImage), CameraError> {
    let mut img = frame.image.clone();
    if s.image.mask_mode == MaskMode::Circle {
        apply_mask_circle(&mut img, &s.overlay.calibration);
    }
    let raw_jpeg = Bytes::from(encode_jpeg(&img)?);
    let (rw, rh) = (img.width(), img.height());
    let processed = match &s.image.crop {
        Some(c) => apply_crop(&img, c),
        None => img,
    };
    let jpeg = Bytes::from(encode_jpeg(&processed)?);

    let persist_jpeg = if s.overlay.bake_into_saved_frames {
        // Same geometry pipeline as GET/POST /api/overlay: build at raw
        // size, append text fields, then crop — so the baked overlay is
        // exactly what the browser preview shows (WYSIWYG).
        let mut geo = geometry::build_overlay_geometry(&geometry::BuildOptions {
            time: frame.timestamp,
            location: &s.location,
            calibration: &s.overlay.calibration,
            layers: &s.overlay.layers,
            grid_opacity: Some(s.overlay.grid_opacity),
            image_width: rw,
            image_height: rh,
        });
        let ctx = geometry::TextContext {
            local_time: frame
                .timestamp
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
            exposure_us: Some(frame.exposure_us),
            gain: Some(frame.gain),
            sensor_temp_c,
        };
        geometry::append_text_fields(&mut geo, &s.overlay.text_fields, &ctx);
        if let Some(c) = &s.image.crop {
            geo = geometry::crop_geometry(geo, c);
        }
        let mut baked = processed.clone();
        crate::overlay::bake::bake_overlay(&mut baked, &geo);
        Bytes::from(encode_jpeg(&baked)?)
    } else {
        jpeg.clone()
    };

    let meta = FrameMeta {
        timestamp: frame
            .timestamp
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        exposure_us: frame.exposure_us,
        gain: frame.gain,
        is_night,
    };
    Ok((
        LatestFrame {
            jpeg,
            persist_jpeg,
            raw_jpeg,
            raw_width: rw,
            raw_height: rh,
            meta,
        },
        processed,
    ))
}

fn make_camera(
    driver: CameraDriver,
    width: u32,
    height: u32,
) -> Result<Box<dyn Camera>, CameraError> {
    match driver {
        // The mock's synthetic sky is a fixed size; only the real camera
        // captures at the configured resolution.
        CameraDriver::Mock => Ok(Box::new(MockCamera::new())),
        CameraDriver::Rpicam => Ok(Box::new(RpiCamera::probe_with_size(width, height)?)),
        CameraDriver::Asi => Ok(Box::new(crate::camera::asi::AsiCamera::probe_with_size(
            width, height,
        )?)),
    }
}

fn persist_frame(data_dir: &Path, latest: &LatestFrame) -> anyhow::Result<String> {
    let date = night_date(Local::now()).to_string();
    let night_dir = data_dir.join("images").join(&date);
    let frames_dir = night_dir.join("frames");
    std::fs::create_dir_all(&frames_dir)?;
    let file = format!("{}.jpg", Local::now().format("%Y%m%d-%H%M%S"));
    std::fs::write(frames_dir.join(&file), &latest.persist_jpeg)?;
    let line = serde_json::json!({
        "timestamp": latest.meta.timestamp,
        "file": file,
        "exposureUs": latest.meta.exposure_us,
        "gain": latest.meta.gain,
    });
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(night_dir.join("frames.jsonl"))?;
    writeln!(f, "{line}")?;
    Ok(file)
}

/// Push a status update, carrying forward the previous `last_frame` (this
/// send never touches it — only a successfully processed frame does).
fn send_status(
    status_tx: &watch::Sender<CaptureStatus>,
    state: CaptureState,
    message: Option<String>,
) {
    let last_frame = status_tx.borrow().last_frame.clone();
    let _ = status_tx.send(CaptureStatus {
        state,
        message,
        last_frame,
    });
}

/// Spawn the supervised capture task. Camera build/capture errors — and
/// panics raised inside the camera driver during capture — set
/// camera_unavailable status and retry with exponential backoff; the
/// supervisor loop (and the web server it runs alongside) is unaffected.
pub fn spawn_capture(
    cfg: Arc<RwLock<ConfigFile>>,
    data_dir: PathBuf,
    tap: Option<tokio::sync::mpsc::Sender<crate::processing::NightFrame>>,
) -> CaptureChannels {
    spawn_capture_with(cfg, data_dir, tap, make_camera)
}

/// Same as [`spawn_capture`] but with the camera factory injected, so tests
/// can force build/capture errors and panics without touching real hardware.
fn spawn_capture_with<F>(
    cfg: Arc<RwLock<ConfigFile>>,
    data_dir: PathBuf,
    tap: Option<tokio::sync::mpsc::Sender<crate::processing::NightFrame>>,
    factory: F,
) -> CaptureChannels
where
    F: Fn(CameraDriver, u32, u32) -> Result<Box<dyn Camera>, CameraError> + Send + Sync + 'static,
{
    let (latest_tx, latest_rx) = watch::channel::<Option<Arc<LatestFrame>>>(None);
    let (status_tx, status_rx) = watch::channel(CaptureStatus {
        state: CaptureState::Idle,
        message: None,
        last_frame: None,
    });
    let (caps_tx, caps_rx) = watch::channel::<Option<CameraCaps>>(None);
    let factory = Arc::new(factory);

    tokio::spawn(async move {
        let mut params: Option<CaptureParams> = None;
        // The camera is tagged with the (driver, width, height) it was built
        // for, so a settings change to any of them forces a rebuild.
        let mut camera: Option<(CameraDriver, u32, u32, Box<dyn Camera>)> = None;
        let mut backoff = Duration::from_secs(1);

        loop {
            let s = cfg.read().await.settings.clone();
            let want = (
                s.camera.driver,
                s.camera.capture_width,
                s.camera.capture_height,
            );

            // (Re)create the camera when missing or when driver/resolution changed.
            if camera.as_ref().map(|(d, w, h, _)| (*d, *w, *h)) != Some(want) {
                camera = None;
            }
            if camera.is_none() {
                let (driver, width, height) = want;
                let factory = factory.clone();
                let built =
                    tokio::task::spawn_blocking(move || (*factory)(driver, width, height)).await;
                match built {
                    Ok(result) => match result {
                        Ok(c) => {
                            let _ = caps_tx.send(Some(CameraCaps::from(&c.info())));
                            camera = Some((driver, width, height, c));
                            backoff = Duration::from_secs(1);
                        }
                        Err(e) => {
                            send_status(
                                &status_tx,
                                CaptureState::CameraUnavailable,
                                Some(e.to_string()),
                            );
                            tokio::time::sleep(backoff).await;
                            backoff = (backoff * 2).min(Duration::from_secs(60));
                            continue;
                        }
                    },
                    Err(join_err) => {
                        send_status(
                            &status_tx,
                            CaptureState::CameraUnavailable,
                            Some(format!("camera factory panicked: {join_err}")),
                        );
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(Duration::from_secs(60));
                        continue;
                    }
                }
            }

            let night = is_night(
                Utc::now(),
                s.location.latitude_deg,
                s.location.longitude_deg,
            );
            if !night && !s.camera.capture_during_day {
                send_status(
                    &status_tx,
                    CaptureState::Idle,
                    Some("daytime — capture paused".into()),
                );
                tokio::time::sleep(Duration::from_secs(30)).await;
                continue;
            }

            let (_, _, _, cam) = camera.as_ref().expect("camera present");
            let info = cam.info();
            let lim = auto_exposure::ExposureLimits {
                min_exposure_us: s.camera.exposure_us_min.max(info.min_exposure_us),
                max_exposure_us: s.camera.exposure_us_max.min(info.max_exposure_us),
                min_gain: s.camera.gain_min.max(info.min_gain),
                max_gain: s.camera.gain_max.min(info.max_gain),
            };
            let p = if s.camera.auto_exposure {
                params.unwrap_or(CaptureParams {
                    exposure_us: s
                        .camera
                        .manual_exposure_us
                        .clamp(lim.min_exposure_us, lim.max_exposure_us),
                    gain: s.camera.manual_gain.clamp(lim.min_gain, lim.max_gain),
                })
            } else {
                CaptureParams {
                    exposure_us: s
                        .camera
                        .manual_exposure_us
                        .clamp(lim.min_exposure_us, lim.max_exposure_us),
                    gain: s.camera.manual_gain.clamp(lim.min_gain, lim.max_gain),
                }
            };

            // Capture AND the whole per-frame pipeline (brightness, mask/crop/
            // encode, disk persistence) run inside the SAME spawn_blocking
            // closure — none of that CPU/disk work belongs on the async
            // runtime. The camera moves into the closure and the closure
            // hands it straight back alongside the result, so there is
            // nothing left to reassemble. A panic anywhere in here (e.g. a
            // misbehaving driver) surfaces as a `JoinError` below instead of
            // taking down the supervisor task.
            let (driver, cam_w, cam_h, mut cam) = camera.take().expect("camera present");
            let join = tokio::task::spawn_blocking({
                let s = s.clone();
                let data_dir = data_dir.clone();
                let tap = tap.clone();
                move || {
                    let r = cam.capture(p).and_then(|frame| {
                        let mean = mean_brightness(&frame.image);
                        let taken = CaptureParams {
                            exposure_us: frame.exposure_us,
                            gain: frame.gain,
                        };
                        let wants_temp = s.overlay.bake_into_saved_frames
                            && s.sensor.enabled
                            && s.overlay
                                .text_fields
                                .iter()
                                .any(|f| f.kind == crate::settings::TextFieldKind::SensorTemp);
                        let temp = wants_temp
                            .then(|| crate::sensors::read_sensor(true).reading)
                            .flatten()
                            .map(|r| r.temperature_c);
                        let (latest, clean) = process_frame(&frame, &s, night, temp)?;
                        // Don't save blown-white / crushed-black frames that
                        // auto-exposure is still hunting through — they're
                        // useless in the gallery and would corrupt the keogram
                        // and star trails. Manual exposure always saves.
                        let keep = !s.camera.auto_exposure || !auto_exposure::is_clipped(mean);
                        if keep {
                            // persistence failure must not kill the frame publication
                            match persist_frame(&data_dir, &latest) {
                                Ok(file) => {
                                    // Tap AFTER persist: every frame the processor
                                    // sees is also on disk, so replay is authoritative.
                                    if let Some(tap) = &tap {
                                        let nf = crate::processing::NightFrame {
                                            date: night_date(Local::now()),
                                            file,
                                            image: clean,
                                            mean,
                                        };
                                        if let Err(e) = tap.try_send(nf) {
                                            tracing::warn!(
                                                "processing busy, frame dropped from artifacts: {e}"
                                            );
                                        }
                                    }
                                }
                                Err(e) => tracing::error!("persisting frame: {e:#}"),
                            }
                        }
                        Ok((latest, mean, taken))
                    });
                    (r, cam)
                }
            })
            .await;

            let (result, cam) = match join {
                Ok(pair) => pair,
                Err(join_err) => {
                    send_status(
                        &status_tx,
                        CaptureState::CameraUnavailable,
                        Some(format!("capture task panicked: {join_err}")),
                    );
                    camera = None; // the Box was consumed by the closure; re-probe next round
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                    continue;
                }
            };
            camera = Some((driver, cam_w, cam_h, cam));

            let (latest, mean, taken) = match result {
                Ok(v) => v,
                Err(e) => {
                    send_status(
                        &status_tx,
                        CaptureState::CameraUnavailable,
                        Some(e.to_string()),
                    );
                    camera = None; // re-probe next round
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                    continue;
                }
            };
            backoff = Duration::from_secs(1);

            // Converged when brightness is in band, or when the controller is
            // railed at a limit and can no longer improve (prevents an endless
            // fast-meter loop at extreme brightness). Manual exposure is always
            // "converged" so it keeps the configured interval.
            let converged = if s.camera.auto_exposure {
                let target = s.camera.target_brightness;
                let next = auto_exposure::next_params(mean, target, taken, &lim);
                let done = auto_exposure::converged(mean, target) || next == taken;
                params = Some(next);
                done
            } else {
                true
            };

            let meta = latest.meta.clone();
            let _ = latest_tx.send(Some(Arc::new(latest)));
            let _ = status_tx.send(CaptureStatus {
                state: CaptureState::Capturing,
                message: None,
                last_frame: Some(meta),
            });

            // While still hunting the exposure, re-meter after a short delay so
            // convergence takes seconds, not one full interval per step. Once
            // settled, fall back to the configured capture interval.
            let delay = if converged {
                Duration::from_secs(s.camera.interval_sec.max(1))
            } else {
                METER_INTERVAL
            };
            tokio::time::sleep(delay).await;
        }
    });

    CaptureChannels {
        latest: latest_rx,
        status: status_rx,
        camera_caps: caps_rx,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn night_date_buckets_noon_to_noon() {
        let evening = chrono::Local
            .with_ymd_and_hms(2026, 7, 15, 22, 0, 0)
            .unwrap();
        assert_eq!(night_date(evening).to_string(), "2026-07-15");
        let after_midnight = chrono::Local
            .with_ymd_and_hms(2026, 7, 16, 3, 30, 0)
            .unwrap();
        assert_eq!(night_date(after_midnight).to_string(), "2026-07-15");
        let noon = chrono::Local
            .with_ymd_and_hms(2026, 7, 16, 12, 0, 0)
            .unwrap();
        assert_eq!(night_date(noon).to_string(), "2026-07-16");
    }

    #[test]
    fn process_frame_masks_crops_and_encodes() {
        use crate::camera::{Camera, CaptureParams};
        let mut cam = crate::camera::mock::MockCamera::new();
        let frame = cam
            .capture(CaptureParams {
                exposure_us: 1_000_000,
                gain: 4.0,
            })
            .unwrap();
        let mut s = crate::settings::Settings::default();
        s.image.mask_mode = crate::settings::MaskMode::Circle;
        s.image.crop = Some(crate::settings::CropRect {
            x: 160.0,
            y: 120.0,
            width: 960.0,
            height: 720.0,
        });
        let (latest, _) = process_frame(&frame, &s, true, None).unwrap();
        assert!(latest.jpeg.starts_with(&[0xFF, 0xD8]));
        assert!(latest.raw_jpeg.starts_with(&[0xFF, 0xD8]));
        let processed = image::load_from_memory(&latest.jpeg).unwrap();
        assert_eq!((processed.width(), processed.height()), (960, 720));
        let raw = image::load_from_memory(&latest.raw_jpeg).unwrap();
        assert_eq!((raw.width(), raw.height()), (1280, 960));
        assert!(latest.meta.is_night);
    }

    #[test]
    fn baking_changes_the_persisted_jpeg_but_not_the_dashboard_jpeg() {
        use crate::camera::{Camera, CaptureParams};
        let mut cam = crate::camera::mock::MockCamera::new();
        let frame = cam
            .capture(CaptureParams {
                exposure_us: 1_000_000,
                gain: 4.0,
            })
            .unwrap();
        let mut s = crate::settings::Settings::default();
        s.overlay.bake_into_saved_frames = false;
        let (clean, _) = process_frame(&frame, &s, true, None).unwrap();
        assert_eq!(clean.persist_jpeg, clean.jpeg); // bake off → identical

        s.overlay.bake_into_saved_frames = true;
        let (baked, _) = process_frame(&frame, &s, true, Some(12.3)).unwrap();
        assert_eq!(baked.jpeg, clean.jpeg); // dashboard copy stays clean
        assert_ne!(baked.persist_jpeg, baked.jpeg); // persisted copy differs
                                                    // And it still decodes at the same size as the clean one.
        let img = image::load_from_memory(&baked.persist_jpeg).unwrap();
        let clean_img = image::load_from_memory(&clean.jpeg).unwrap();
        assert_eq!(img.width(), clean_img.width());
    }

    #[test]
    fn process_frame_returns_the_clean_processed_image() {
        use crate::camera::{Camera, CaptureParams};
        let mut cam = crate::camera::mock::MockCamera::new();
        let frame = cam
            .capture(CaptureParams {
                exposure_us: 1_000_000,
                gain: 4.0,
            })
            .unwrap();
        let mut s = crate::settings::Settings::default();
        s.overlay.bake_into_saved_frames = true; // baking must NOT leak into the returned image
        s.image.crop = Some(crate::settings::CropRect {
            x: 160.0,
            y: 120.0,
            width: 960.0,
            height: 720.0,
        });
        let (latest, clean) = process_frame(&frame, &s, true, None).unwrap();
        assert_eq!((clean.width(), clean.height()), (960, 720)); // cropped
                                                                 // The clean image re-encodes to exactly the dashboard jpeg.
        assert_eq!(
            crate::camera::encode_jpeg(&clean).unwrap(),
            latest.jpeg.to_vec()
        );
    }

    #[test]
    fn persist_frame_returns_the_filename_and_writes_persist_jpeg() {
        use crate::camera::{Camera, CaptureParams};
        let dir = tempfile::TempDir::new().unwrap();
        let mut cam = crate::camera::mock::MockCamera::new();
        let frame = cam
            .capture(CaptureParams {
                exposure_us: 1_000_000,
                gain: 4.0,
            })
            .unwrap();
        let mut s = crate::settings::Settings::default();
        s.overlay.bake_into_saved_frames = true;
        let (latest, _) = process_frame(&frame, &s, true, None).unwrap();
        let file = persist_frame(dir.path(), &latest).unwrap();
        assert!(file.ends_with(".jpg"));
        let date = night_date(chrono::Local::now()).to_string();
        let on_disk = std::fs::read(
            dir.path()
                .join("images")
                .join(&date)
                .join("frames")
                .join(&file),
        )
        .unwrap();
        assert_eq!(on_disk, latest.persist_jpeg.to_vec()); // baked copy is what's saved
    }

    #[tokio::test]
    async fn capture_loop_with_mock_camera_publishes_and_persists_frames() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut cfg = crate::settings::ConfigFile {
            version: 1,
            password_hash: "h".into(),
            settings: crate::settings::Settings::default(),
        };
        cfg.settings.camera.driver = crate::settings::CameraDriver::Mock;
        cfg.settings.camera.interval_sec = 1;
        cfg.settings.camera.capture_during_day = true; // test must not depend on wall clock
        let shared = std::sync::Arc::new(tokio::sync::RwLock::new(cfg));
        let mut ch = spawn_capture(shared, dir.path().to_path_buf(), None);

        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                ch.latest.changed().await.unwrap();
                if ch.latest.borrow().is_some() {
                    break;
                }
            }
        })
        .await
        .expect("no frame within 10s");

        assert_eq!(ch.status.borrow().state, CaptureState::Capturing);
        let date = night_date(chrono::Local::now()).to_string();
        let frames_dir = dir.path().join("images").join(&date).join("frames");
        assert!(frames_dir.read_dir().unwrap().count() >= 1);
        let jsonl =
            std::fs::read_to_string(dir.path().join("images").join(&date).join("frames.jsonl"))
                .unwrap();
        let first: serde_json::Value = serde_json::from_str(jsonl.lines().next().unwrap()).unwrap();
        assert!(first["exposureUs"].is_number());
        assert!(first["file"].as_str().unwrap().ends_with(".jpg"));
    }

    fn test_cfg() -> crate::settings::ConfigFile {
        let mut cfg = crate::settings::ConfigFile {
            version: 1,
            password_hash: "h".into(),
            settings: crate::settings::Settings::default(),
        };
        cfg.settings.camera.driver = crate::settings::CameraDriver::Mock;
        cfg.settings.camera.interval_sec = 1;
        cfg.settings.camera.capture_during_day = true; // test must not depend on wall clock
        cfg
    }

    #[tokio::test]
    async fn publishes_camera_caps_when_the_camera_builds() {
        let dir = tempfile::TempDir::new().unwrap();
        let shared = std::sync::Arc::new(tokio::sync::RwLock::new(test_cfg()));
        let mut ch = spawn_capture(shared, dir.path().to_path_buf(), None);
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                ch.camera_caps.changed().await.unwrap();
                if ch.camera_caps.borrow().is_some() {
                    break;
                }
            }
        })
        .await
        .expect("no camera caps within 10s");
        let caps = ch.camera_caps.borrow().clone().unwrap();
        assert_eq!((caps.max_width, caps.max_height), (1280, 960)); // mock sensor
        assert!(!caps.model.is_empty());
    }

    #[tokio::test]
    async fn capture_error_reports_camera_unavailable() {
        struct FailingCamera;
        impl Camera for FailingCamera {
            fn info(&self) -> crate::camera::CameraInfo {
                MockCamera::new().info()
            }
            fn capture(&mut self, _p: CaptureParams) -> Result<Frame, CameraError> {
                Err(CameraError::Capture("injected failure".into()))
            }
        }

        let dir = tempfile::TempDir::new().unwrap();
        let shared = std::sync::Arc::new(tokio::sync::RwLock::new(test_cfg()));
        let mut ch =
            spawn_capture_with(shared, dir.path().to_path_buf(), None, |_driver, _w, _h| {
                Ok(Box::new(FailingCamera) as Box<dyn Camera>)
            });

        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                ch.status.changed().await.unwrap();
                if ch.status.borrow().state == CaptureState::CameraUnavailable {
                    break;
                }
            }
        })
        .await
        .expect("no camera-unavailable status within 10s");

        let msg = ch.status.borrow().message.clone().unwrap();
        assert!(msg.contains("injected failure"), "message was: {msg}");
    }

    #[tokio::test]
    async fn capture_panic_keeps_the_supervisor_alive() {
        struct PanickyCamera;
        impl Camera for PanickyCamera {
            fn info(&self) -> crate::camera::CameraInfo {
                MockCamera::new().info()
            }
            fn capture(&mut self, _p: CaptureParams) -> Result<Frame, CameraError> {
                panic!("boom")
            }
        }

        // The panic below is deliberately injected to prove the supervisor
        // survives it; suppress the default backtrace print so test stderr
        // stays pristine, then restore the default hook afterwards.
        std::panic::set_hook(Box::new(|_| {}));

        let dir = tempfile::TempDir::new().unwrap();
        let shared = std::sync::Arc::new(tokio::sync::RwLock::new(test_cfg()));

        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut ch = spawn_capture_with(
            shared,
            dir.path().to_path_buf(),
            None,
            move |_driver, _w, _h| {
                if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    Ok(Box::new(PanickyCamera) as Box<dyn Camera>)
                } else {
                    Ok(Box::new(MockCamera::new()) as Box<dyn Camera>)
                }
            },
        );

        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                ch.status.changed().await.unwrap();
                if ch.status.borrow().state == CaptureState::CameraUnavailable {
                    break;
                }
            }
        })
        .await
        .expect("no camera-unavailable status within 15s (panic not reported)");

        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                ch.latest.changed().await.unwrap();
                if ch.latest.borrow().is_some() {
                    break;
                }
            }
        })
        .await
        .expect("supervisor did not recover after the panic within 15s");

        let _ = std::panic::take_hook();
    }

    #[tokio::test]
    async fn changing_capture_resolution_rebuilds_the_camera() {
        // The factory records the (width, height) it is asked to build at.
        let sizes = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(u32, u32)>::new()));
        let dir = tempfile::TempDir::new().unwrap();
        let shared = std::sync::Arc::new(tokio::sync::RwLock::new(test_cfg())); // default 1640x1232
        let seen = sizes.clone();
        let mut ch = spawn_capture_with(
            shared.clone(),
            dir.path().to_path_buf(),
            None,
            move |_d, w, h| {
                seen.lock().unwrap().push((w, h));
                Ok(Box::new(MockCamera::new()) as Box<dyn Camera>)
            },
        );

        // Wait for the first build (at the default resolution).
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                ch.latest.changed().await.unwrap();
                if ch.latest.borrow().is_some() {
                    break;
                }
            }
        })
        .await
        .expect("no frame within 10s");
        assert_eq!(sizes.lock().unwrap().first().copied(), Some((1640, 1232)));

        // Change the resolution; the loop must rebuild at the new size.
        {
            let mut cfg = shared.write().await;
            cfg.settings.camera.capture_width = 800;
            cfg.settings.camera.capture_height = 600;
        }
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                ch.latest.changed().await.unwrap();
                if sizes.lock().unwrap().contains(&(800, 600)) {
                    break;
                }
            }
        })
        .await
        .expect("camera was not rebuilt at the new resolution within 10s");
    }

    #[tokio::test]
    async fn capture_tap_delivers_persisted_frames() {
        let dir = tempfile::TempDir::new().unwrap();
        let shared = std::sync::Arc::new(tokio::sync::RwLock::new(test_cfg()));
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let _ch = spawn_capture(shared, dir.path().to_path_buf(), Some(tx));
        let nf = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
            .await
            .expect("no tapped frame within 10s")
            .expect("channel closed");
        // The tapped frame's file exists on disk (persist-then-tap invariant).
        let date = night_date(chrono::Local::now()).to_string();
        assert!(dir
            .path()
            .join("images")
            .join(&date)
            .join("frames")
            .join(&nf.file)
            .is_file());
        assert!(nf.image.width() > 0);
    }
}
