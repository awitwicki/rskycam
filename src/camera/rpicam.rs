use std::path::PathBuf;
use std::process::Command;

use chrono::Utc;

use crate::camera::{Camera, CameraError, CameraInfo, CaptureParams, Frame};

/// CSI camera via the `rpicam-still` CLI (libcamera). One subprocess per
/// frame — at all-sky cadence (seconds and up) this is simpler and more
/// robust than holding libcamera open.
pub struct RpiCamera {
    binary: PathBuf,
    width: u32,
    height: u32,
}

pub fn build_args(p: CaptureParams, width: u32, height: u32) -> Vec<String> {
    // App must outlive the shutter; +5 s margin covers sensor setup and encode.
    let timeout_ms = p.exposure_us / 1000 + 5000;

    [
        "-n",          // no preview
        "--immediate", // skip AE/AWB settle loop — we control exposure
        "--denoise",
        "off",
        "--awbgains",
        "1.6,1.6", // fixed neutral-ish WB (NoIR has no meaningful AWB)
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([
        "-t".into(),
        timeout_ms.to_string(),
        "--shutter".into(),
        p.exposure_us.to_string(),
        "--gain".into(),
        format!("{}", p.gain),
        "--width".into(),
        width.to_string(),
        "--height".into(),
        height.to_string(),
        "-o".into(),
        "-".into(),
    ])
    .collect()
}

impl RpiCamera {
    pub fn with_binary(binary: PathBuf, width: u32, height: u32) -> Self {
        RpiCamera {
            binary,
            width,
            height,
        }
    }

    /// Verify rpicam-still exists and capture at the given resolution.
    pub fn probe_with_size(width: u32, height: u32) -> Result<Self, CameraError> {
        Self::probe_binary("rpicam-still".into(), width, height)
    }

    /// Verify the given binary exists and answers --version.
    pub fn probe_binary(binary: PathBuf, width: u32, height: u32) -> Result<Self, CameraError> {
        let out = Command::new(&binary)
            .arg("--version")
            .output()
            .map_err(|e| CameraError::Unavailable(format!("{:?} not found: {e}", binary)))?;
        if !out.status.success() {
            return Err(CameraError::Unavailable(format!(
                "{:?} --version failed",
                binary
            )));
        }
        Ok(RpiCamera::with_binary(binary, width, height))
    }
}

impl Camera for RpiCamera {
    fn info(&self) -> CameraInfo {
        CameraInfo {
            model: "Raspberry Pi CSI (rpicam)".into(),
            width: self.width,
            height: self.height,
            max_width: 3280,
            max_height: 2464,
            min_exposure_us: 32,
            max_exposure_us: 10_000_000,
            min_gain: 1.0,
            max_gain: 16.0,
        }
    }

    fn capture(&mut self, p: CaptureParams) -> Result<Frame, CameraError> {
        let out = Command::new(&self.binary)
            .args(build_args(p, self.width, self.height))
            .output()
            .map_err(|e| CameraError::Capture(format!("spawn rpicam-still: {e}")))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let err = err.trim();
            let err = err.chars().take(300).collect::<String>();
            return Err(CameraError::Capture(format!(
                "rpicam-still exited {}: {}",
                out.status, err
            )));
        }
        let img = image::load_from_memory(&out.stdout)
            .map_err(|e| CameraError::Capture(format!("decode jpeg: {e}")))?
            .to_rgb8();
        Ok(Frame {
            image: img,
            timestamp: Utc::now(),
            exposure_us: p.exposure_us,
            gain: p.gain,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{Camera, CaptureParams};

    #[test]
    fn build_args_maps_params_to_rpicam_flags() {
        let args = build_args(
            CaptureParams {
                exposure_us: 5_000_000,
                gain: 8.0,
            },
            1640,
            1232,
        );
        let joined = args.join(" ");
        assert!(joined.contains("--shutter 5000000"));
        assert!(joined.contains("--gain 8"));
        assert!(joined.contains("--width 1640"));
        assert!(joined.contains("--height 1232"));
        assert!(joined.contains("--immediate"));
        assert!(joined.contains("-n"));
        assert!(joined.ends_with("-o -"));
        // Timeout derived from exposure: 5_000_000 / 1000 + 5000 = 10000
        assert!(args.windows(2).any(|w| w == ["-t", "10000"]));
    }

    #[test]
    fn capture_decodes_stdout_jpeg_from_the_fake_binary() {
        let bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/fake-rpicam-still");
        let mut cam = RpiCamera::with_binary(bin, 64, 48);
        let f = cam
            .capture(CaptureParams {
                exposure_us: 100_000,
                gain: 2.0,
            })
            .unwrap();
        assert_eq!((f.image.width(), f.image.height()), (64, 48));
        assert_eq!(f.exposure_us, 100_000);
    }

    #[test]
    fn probe_fails_cleanly_when_binary_is_missing() {
        let cam = RpiCamera::with_binary("/nonexistent/rpicam-still".into(), 64, 48);
        let err = {
            let mut c = cam;
            c.capture(CaptureParams {
                exposure_us: 1,
                gain: 1.0,
            })
        };
        assert!(matches!(err, Err(crate::camera::CameraError::Capture(_))));
    }

    #[test]
    fn probe_succeeds_against_the_fake_binary() {
        let bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/fake-rpicam-still");
        let cam = RpiCamera::probe_binary(bin, 64, 48).unwrap();
        let info = cam.info();
        assert!(info.model.contains("rpicam"));
    }

    #[test]
    fn probe_reports_unavailable_for_missing_binary() {
        let err = RpiCamera::probe_binary("/nonexistent/rpicam-still".into(), 64, 48);
        assert!(matches!(
            err,
            Err(crate::camera::CameraError::Unavailable(_))
        ));
    }
}
