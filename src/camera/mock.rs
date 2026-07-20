use chrono::Utc;
use image::{Rgb, RgbImage};

use crate::camera::{Camera, CameraError, CameraInfo, CaptureParams, Frame};
use crate::overlay::astro;

const W: u32 = 1280;
const H: u32 = 960;
const STAR_COUNT: u64 = 350;

/// The sky is never pure black (light pollution / airglow); this floor also
/// guarantees the exposure response is measurable at astronomical night.
const NIGHT_SKYGLOW: f64 = 14.0;

/// Deterministic synthetic sky: fixed pseudo-random star catalog projected
/// through the default lens for Kyiv, twilight background from sun altitude,
/// pixel values scaled by exposure·gain so auto-exposure has a real signal.
pub struct MockCamera;

impl MockCamera {
    pub fn new() -> Self {
        MockCamera
    }
}

impl Default for MockCamera {
    fn default() -> Self {
        Self::new()
    }
}

/// Tiny deterministic PRNG (splitmix64) — no rand dependency in the hot path.
fn splitmix(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn unit(seed: u64) -> f64 {
    (splitmix(seed) >> 11) as f64 / (1u64 << 53) as f64
}

impl Camera for MockCamera {
    fn info(&self) -> CameraInfo {
        CameraInfo {
            model: "Mock synthetic sky".into(),
            width: W,
            height: H,
            max_width: W,
            max_height: H,
            min_exposure_us: 32,
            max_exposure_us: 30_000_000,
            min_gain: 1.0,
            max_gain: 16.0,
        }
    }

    fn capture(&mut self, p: CaptureParams) -> Result<Frame, CameraError> {
        let now = Utc::now();
        let defaults = crate::settings::Settings::default();
        let cal = defaults.overlay.calibration;
        let (lat, lon) = (
            defaults.location.latitude_deg,
            defaults.location.longitude_deg,
        );
        // Exposure response: 5s @ gain 8 ≈ neutral 1.0.
        let scale = (p.exposure_us as f64 * p.gain / 40_000_000.0).clamp(0.001, 20.0);

        let sun = astro::sun_equatorial(now);
        let sun_alt = astro::altitude_of(now, sun.ra_deg, sun.dec_deg, lat, lon);
        // Background sky: pitch black at astro night, bright at day.
        let base = ((sun_alt + 18.0) / 36.0).clamp(0.0, 1.0) * 180.0 + NIGHT_SKYGLOW;
        let bg = (base * scale).clamp(0.0, 235.0) as u8;
        let mut img = RgbImage::from_pixel(W, H, Rgb([bg, bg, bg / 2 + 40 * (bg > 0) as u8]));

        let lst = astro::lst_deg(now, lon);
        for i in 0..STAR_COUNT {
            let ra = unit(i * 2 + 1) * 360.0;
            let dec = unit(i * 2 + 2) * 180.0 - 90.0;
            let aa = astro::ra_dec_to_alt_az(ra, dec, lat, lst);
            if aa.alt_deg < 0.0 {
                continue;
            }
            let pt = astro::alt_az_to_image(aa.alt_deg, aa.az_deg, &cal);
            let (x, y) = (pt.x.round() as i64, pt.y.round() as i64);
            if !(0..W as i64).contains(&x) || !(0..H as i64).contains(&y) {
                continue;
            }
            let mag = 60.0 + unit(i + 777) * 195.0;
            let v = (mag * scale).clamp(0.0, 255.0) as u8;
            let px = img.get_pixel_mut(x as u32, y as u32);
            *px = Rgb([px.0[0].max(v), px.0[1].max(v), px.0[2].max(v)]);
        }

        Ok(Frame {
            image: img,
            timestamp: now,
            exposure_us: p.exposure_us,
            gain: p.gain,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{mean_brightness, Camera, CaptureParams};

    #[test]
    fn frames_are_deterministic_and_sized() {
        let mut cam = MockCamera::new();
        let p = CaptureParams {
            exposure_us: 5_000_000,
            gain: 8.0,
        };
        let a = cam.capture(p).unwrap();
        let b = cam.capture(p).unwrap();
        assert_eq!((a.image.width(), a.image.height()), (1280, 960));
        // same params ⇒ statistically identical output (same star field)
        assert!((mean_brightness(&a.image) - mean_brightness(&b.image)).abs() < 1.0);
    }

    #[test]
    fn brightness_scales_with_exposure_and_gain() {
        let mut cam = MockCamera::new();
        let dim = cam
            .capture(CaptureParams {
                exposure_us: 100_000,
                gain: 1.0,
            })
            .unwrap();
        let bright = cam
            .capture(CaptureParams {
                exposure_us: 8_000_000,
                gain: 8.0,
            })
            .unwrap();
        assert!(mean_brightness(&bright.image) > mean_brightness(&dim.image) + 5.0);
    }

    #[test]
    fn info_reports_bounds_used_by_auto_exposure() {
        let info = MockCamera::new().info();
        assert_eq!((info.width, info.height), (1280, 960));
        assert!(info.min_exposure_us < info.max_exposure_us);
        assert!(info.min_gain < info.max_gain);
    }
}
