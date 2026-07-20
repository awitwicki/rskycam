//! ZWO ASI camera driver. The proprietary SDK is embedded in the binary and
//! dlopen-ed at probe time, so the deployable stays a single file. All
//! unsafe FFI lives in this module; every SDK call's return code is checked.
mod ffi;

use std::ffi::{c_int, c_long};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use image::RgbImage;

use crate::camera::{Camera, CameraError, CameraInfo, CaptureParams, Frame};

const SDK_BYTES: &[u8] = include_bytes!("../../../assets/asi/libASICamera2.so");

/// Write the embedded SDK into a private, per-process directory and return
/// its path.
///
/// This deliberately does NOT reuse a previously-extracted file at a fixed,
/// predictable path: `Sdk::load` dlopens whatever we return here, so trusting
/// anything already on disk (even matching-length) at a world-writable path
/// like the shared system temp dir would let a local attacker pre-plant a
/// same-length malicious `.so` and have us load it as our own code, running
/// as the rskycam user. Extracting fresh, unconditionally, into a directory
/// only this process can write to closes that hole. Extraction re-runs each
/// probe (same per-process directory, idempotent) to handle supervisor retries
/// after probe/capture failures.
///
/// **Not safe to call concurrently within one process**: the target directory
/// is keyed by pid only. The supervisor is assumed to be the sole, sequential
/// caller. A second concurrent caller would race on the remove_dir_all +
/// create_dir sequence, causing one to fail with AlreadyExists or similar.
pub(super) fn extract_so() -> std::io::Result<PathBuf> {
    let dir = sdk_dir();
    // Best-effort: clear a stale dir a dead process left behind under a
    // reused pid. Fine if it doesn't exist or removal fails for some other
    // reason — `create_private_dir` below will surface any real problem.
    let _ = std::fs::remove_dir_all(&dir);
    create_private_dir(&dir)?;
    let path = dir.join("libASICamera2.so");
    std::fs::write(&path, SDK_BYTES)?;
    Ok(path)
}

fn sdk_dir() -> PathBuf {
    std::env::temp_dir().join(format!("rskycam-asi-{}", std::process::id()))
}

// 0700 so no other local user can plant a file in this directory before we
// dlopen out of it (see the comment on `extract_so`).
#[cfg(unix)]
fn create_private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir(dir)
}

pub(super) struct Sdk {
    lib: libloading::Library,
}

impl Sdk {
    pub(super) fn load() -> Result<Sdk, CameraError> {
        let path = extract_so()
            .map_err(|e| CameraError::Unavailable(format!("extracting ASI SDK: {e}")))?;
        // SAFETY: the library is the vendored ZWO SDK we embedded ourselves.
        let lib = unsafe { libloading::Library::new(&path) }
            .map_err(|e| CameraError::Unavailable(format!("loading ASI SDK: {e}")))?;
        Ok(Sdk { lib })
    }

    /// Resolve an SDK symbol. Lookup happens per call — at one frame per
    /// interval the cost is irrelevant and it keeps lifetimes trivial.
    pub(super) unsafe fn sym<T>(
        &self,
        name: &[u8],
    ) -> Result<libloading::Symbol<'_, T>, CameraError> {
        self.lib.get(name).map_err(|e| {
            CameraError::Unavailable(format!(
                "ASI symbol {:?}: {e}",
                String::from_utf8_lossy(name)
            ))
        })
    }
}

/// Expand one mono byte per pixel into an RGB image (R=G=B). Safe as long as
/// `buf` holds at least `w * h` bytes — `capture()` below sizes its download
/// buffer to exactly `width * height` before calling this, so the indexing
/// here can never run past the end of `buf`.
pub(super) fn mono_to_rgb(buf: &[u8], w: u32, h: u32) -> RgbImage {
    RgbImage::from_fn(w, h, |x, y| {
        let v = buf[(y * w + x) as usize];
        image::Rgb([v, v, v])
    })
}

pub(super) fn round_roi(req_w: u32, req_h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let w = (req_w.min(max_w) / 8 * 8).max(8);
    let h = (req_h.min(max_h) / 2 * 2).max(2);
    (w, h)
}

/// Wall-clock budget for one snap: the exposure itself plus 5 s of
/// download/scheduling slack (same rule as the rpicam subprocess timeout).
fn snap_deadline(exposure_us: u64) -> Duration {
    Duration::from_micros(exposure_us) + Duration::from_secs(5)
}

pub struct AsiCamera {
    sdk: Sdk,
    id: c_int,
    model: String,
    width: u32,
    height: u32,
    sensor_max_w: u32,
    sensor_max_h: u32,
    min_exposure_us: u64,
    max_exposure_us: u64,
    min_gain: f64,
    max_gain: f64,
}

/// Post-open configuration results, threaded out of the fallible
/// `configure` step so a failure can close the camera before returning.
struct Configured {
    w: u32,
    h: u32,
    exp: (u64, u64),
    gain: (f64, f64),
}

impl AsiCamera {
    pub fn probe_with_size(width: u32, height: u32) -> Result<AsiCamera, CameraError> {
        let sdk = Sdk::load()?;
        unsafe {
            let count: libloading::Symbol<unsafe extern "C" fn() -> c_int> =
                sdk.sym(b"ASIGetNumOfConnectedCameras")?;
            if count() < 1 {
                return Err(CameraError::Unavailable("no ASI camera on USB".into()));
            }
            let get_prop: libloading::Symbol<
                unsafe extern "C" fn(*mut ffi::AsiCameraInfo, c_int) -> c_int,
            > = sdk.sym(b"ASIGetCameraProperty")?;
            let mut info = std::mem::zeroed::<ffi::AsiCameraInfo>();
            check(get_prop(&mut info, 0), "ASIGetCameraProperty")?;
            let id = info.camera_id;
            let model = std::ffi::CStr::from_ptr(info.name.as_ptr())
                .to_string_lossy()
                .into_owned();

            let open: libloading::Symbol<unsafe extern "C" fn(c_int) -> c_int> =
                sdk.sym(b"ASIOpenCamera")?;
            check(open(id), "ASIOpenCamera")?;

            // The camera is now OPEN. Every early return past this point must
            // close it first, or the device stays claimed (a leaked handle
            // makes the next probe fail until the process restarts). The
            // post-open configuration is fallible, so run it and, on any
            // error, close before propagating.
            let configure = || -> Result<Configured, CameraError> {
                let init: libloading::Symbol<unsafe extern "C" fn(c_int) -> c_int> =
                    sdk.sym(b"ASIInitCamera")?;
                check(init(id), "ASIInitCamera")?;

                // Exposure/gain limits from the control caps table.
                let ncontrols: libloading::Symbol<
                    unsafe extern "C" fn(c_int, *mut c_int) -> c_int,
                > = sdk.sym(b"ASIGetNumOfControls")?;
                let caps_fn: libloading::Symbol<
                    unsafe extern "C" fn(c_int, c_int, *mut ffi::AsiControlCaps) -> c_int,
                > = sdk.sym(b"ASIGetControlCaps")?;
                let mut n = 0;
                check(ncontrols(id, &mut n), "ASIGetNumOfControls")?;
                let (mut exp, mut gain) = ((32u64, 10_000_000u64), (0f64, 100f64));
                for i in 0..n {
                    let mut caps = std::mem::zeroed::<ffi::AsiControlCaps>();
                    if caps_fn(id, i, &mut caps) != 0 {
                        continue;
                    }
                    match caps.control_type {
                        ffi::ASI_EXPOSURE => exp = (caps.min_value as u64, caps.max_value as u64),
                        ffi::ASI_GAIN => gain = (caps.min_value as f64, caps.max_value as f64),
                        _ => {}
                    }
                }

                let (w, h) =
                    round_roi(width, height, info.max_width as u32, info.max_height as u32);
                let set_roi: libloading::Symbol<
                    unsafe extern "C" fn(c_int, c_int, c_int, c_int, c_int) -> c_int,
                > = sdk.sym(b"ASISetROIFormat")?;
                check(
                    set_roi(id, w as c_int, h as c_int, 1, ffi::ASI_IMG_RAW8),
                    "ASISetROIFormat",
                )?;
                Ok(Configured { w, h, exp, gain })
            };
            let Configured { w, h, exp, gain } = match configure() {
                Ok(v) => v,
                Err(e) => {
                    if let Ok(close) =
                        sdk.sym::<unsafe extern "C" fn(c_int) -> c_int>(b"ASICloseCamera")
                    {
                        let _ = close(id);
                    }
                    return Err(e);
                }
            };

            tracing::info!("ASI camera ready: {model} {w}x{h} (RAW8, bin1)");
            Ok(AsiCamera {
                sdk,
                id,
                model,
                width: w,
                height: h,
                sensor_max_w: info.max_width as u32,
                sensor_max_h: info.max_height as u32,
                min_exposure_us: exp.0,
                max_exposure_us: exp.1,
                min_gain: gain.0,
                max_gain: gain.1,
            })
        }
    }
}

fn check(rc: c_int, what: &str) -> Result<(), CameraError> {
    if rc == 0 {
        Ok(())
    } else {
        Err(CameraError::Unavailable(format!(
            "{what}: {}",
            ffi::err_name(rc)
        )))
    }
}

impl Camera for AsiCamera {
    fn info(&self) -> CameraInfo {
        CameraInfo {
            model: self.model.clone(),
            width: self.width,
            height: self.height,
            max_width: self.sensor_max_w,
            max_height: self.sensor_max_h,
            min_exposure_us: self.min_exposure_us,
            max_exposure_us: self.max_exposure_us,
            min_gain: self.min_gain,
            max_gain: self.max_gain,
        }
    }

    fn capture(&mut self, p: CaptureParams) -> Result<Frame, CameraError> {
        let cap = |rc: c_int, what: &str| -> Result<(), CameraError> {
            if rc == 0 {
                Ok(())
            } else {
                Err(CameraError::Capture(format!(
                    "{what}: {}",
                    ffi::err_name(rc)
                )))
            }
        };
        unsafe {
            let set_ctl: libloading::Symbol<
                unsafe extern "C" fn(c_int, c_int, c_long, c_int) -> c_int,
            > = self.sdk.sym(b"ASISetControlValue")?;
            let exposure_us = p
                .exposure_us
                .clamp(self.min_exposure_us, self.max_exposure_us);
            let gain = p.gain.clamp(self.min_gain, self.max_gain);
            cap(
                set_ctl(self.id, ffi::ASI_EXPOSURE, exposure_us as c_long, 0),
                "set exposure",
            )?;
            cap(
                set_ctl(self.id, ffi::ASI_GAIN, gain.round() as c_long, 0),
                "set gain",
            )?;

            let start: libloading::Symbol<unsafe extern "C" fn(c_int, c_int) -> c_int> =
                self.sdk.sym(b"ASIStartExposure")?;
            cap(start(self.id, 0), "ASIStartExposure")?;

            let status: libloading::Symbol<unsafe extern "C" fn(c_int, *mut c_int) -> c_int> =
                self.sdk.sym(b"ASIGetExpStatus")?;
            let deadline = Instant::now() + snap_deadline(exposure_us);
            let mut st = ffi::ASI_EXP_WORKING;
            while st == ffi::ASI_EXP_WORKING || st == ffi::ASI_EXP_IDLE {
                if Instant::now() > deadline {
                    let stop: libloading::Symbol<unsafe extern "C" fn(c_int) -> c_int> =
                        self.sdk.sym(b"ASIStopExposure")?;
                    let rc = stop(self.id);
                    if rc != 0 {
                        tracing::warn!("ASIStopExposure: {}", ffi::err_name(rc));
                    }
                    return Err(CameraError::Capture("exposure timed out".into()));
                }
                std::thread::sleep(Duration::from_millis(20));
                cap(status(self.id, &mut st), "ASIGetExpStatus")?;
            }
            if st != ffi::ASI_EXP_SUCCESS {
                return Err(CameraError::Capture(format!(
                    "exposure failed (status {st})"
                )));
            }

            let get_data: libloading::Symbol<
                unsafe extern "C" fn(c_int, *mut u8, c_long) -> c_int,
            > = self.sdk.sym(b"ASIGetDataAfterExp")?;
            let len = (self.width * self.height) as usize;
            let mut buf = vec![0u8; len];
            cap(
                get_data(self.id, buf.as_mut_ptr(), len as c_long),
                "ASIGetDataAfterExp",
            )?;

            Ok(Frame {
                image: mono_to_rgb(&buf, self.width, self.height),
                timestamp: chrono::Utc::now(),
                exposure_us,
                gain,
            })
        }
    }
}

impl Drop for AsiCamera {
    fn drop(&mut self) {
        unsafe {
            if let Ok(close) = self
                .sdk
                .sym::<unsafe extern "C" fn(c_int) -> c_int>(b"ASICloseCamera")
            {
                let rc = close(self.id);
                if rc != 0 {
                    tracing::warn!("ASICloseCamera: {}", ffi::err_name(rc));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `extract_so`'s target directory is keyed only by pid (see its doc
    // comment), not by thread — intentionally, since in production exactly
    // one `AsiCamera::probe_with_size` runs at a time. `cargo test` runs
    // tests in parallel threads within one process though, and both this
    // test and `probe_without_hardware_maps_to_unavailable_not_panic` reach
    // `extract_so` (the latter via `Sdk::load`), so without serializing them
    // here their concurrent `remove_dir_all` + `create_dir` calls race and
    // one loses with `AlreadyExists`. This lock only orders the two test
    // bodies; it doesn't change `extract_so` itself.
    static EXTRACT_SO_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn extracts_the_embedded_sdk_into_a_private_dir() {
        let _guard = EXTRACT_SO_TEST_LOCK.lock().unwrap();
        let p1 = extract_so().unwrap();
        assert!(p1.is_file());
        assert_eq!(
            std::fs::metadata(&p1).unwrap().len(),
            SDK_BYTES.len() as u64
        );

        let p2 = extract_so().unwrap(); // same per-process path, freshly rewritten
        assert_eq!(p1, p2);
        assert_eq!(std::fs::read(&p2).unwrap(), SDK_BYTES);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = p1.parent().unwrap();
            let mode = std::fs::metadata(dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
    }

    #[test]
    fn mono_buffer_replicates_into_all_rgb_channels() {
        let buf = [0u8, 128, 255, 7, 42, 200]; // 3x2
        let img = mono_to_rgb(&buf, 3, 2);
        assert_eq!((img.width(), img.height()), (3, 2));
        assert_eq!(img.get_pixel(1, 0), &image::Rgb([128, 128, 128]));
        assert_eq!(img.get_pixel(2, 1), &image::Rgb([200, 200, 200]));
    }

    #[test]
    fn roi_rounds_to_sdk_alignment_and_clamps_to_sensor() {
        // width % 8 == 0, height % 2 == 0, both clamped to the sensor max.
        assert_eq!(round_roi(1280, 960, 1280, 960), (1280, 960));
        assert_eq!(round_roi(1234, 961, 1280, 960), (1232, 960));
        assert_eq!(round_roi(4000, 4000, 1280, 960), (1280, 960));
        assert_eq!(round_roi(2, 1, 1280, 960), (8, 2)); // never zero
    }

    #[test]
    fn probe_without_hardware_maps_to_unavailable_not_panic() {
        // On the dev machine the aarch64 SDK cannot be dlopen-ed (wrong arch)
        // — and on a Pi without the camera enumeration returns 0. Both paths
        // must surface as CameraError::Unavailable so the capture supervisor
        // backs off and retries instead of dying.
        let _guard = EXTRACT_SO_TEST_LOCK.lock().unwrap();
        match AsiCamera::probe_with_size(1280, 960) {
            Err(CameraError::Unavailable(msg)) => {
                assert!(!msg.is_empty());
            }
            Err(other) => panic!("expected Unavailable, got: {other}"),
            Ok(_) => {
                // Only reachable on a machine with a real ASI camera attached
                // (the Pi). Treat as pass — the live task verifies this path.
            }
        }
    }

    #[test]
    fn exposure_deadline_mirrors_the_rpicam_rule() {
        assert_eq!(snap_deadline(1_000_000), Duration::from_millis(6_000));
        assert_eq!(snap_deadline(10_000_000), Duration::from_millis(15_000));
    }
}
