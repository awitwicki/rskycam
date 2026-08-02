use chrono::Utc;
use image::{Rgb, RgbImage};

use crate::camera::{Camera, CameraError, CameraInfo, CaptureParams, Frame};
use crate::overlay::astro;
use crate::settings::{LensCalibration, LensType};

const W: u32 = 1280;
const H: u32 = 960;
const STAR_COUNT: u64 = 350;

/// Calibration for the synthetic sky, independent of `Settings::default()`
/// (which holds the imx219 *physical* lens values). Mirrors the frontend
/// mock's calibration in `frontend/src/api/mock/mockApi.ts` (`defaultSettings`):
/// the old ~620 px image circle on this 1280×960 mock frame
/// (fPx = 1480/3.75 ≈ 394.7 px → horizon at fPx·π/2 ≈ 620 px).
const MOCK_CALIBRATION: LensCalibration = LensCalibration {
    lens_type: LensType::Fisheye,
    focal_length_mm: 1.48,
    pixel_size_um: 3.75,
    pointing_az_deg: 0.0,
    pointing_alt_deg: 90.0,
    roll_deg: 0.0,
    flip: false,
    center_offset_x_px: 0.0,
    center_offset_y_px: 0.0,
};

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

/// Gaussian blob with configurable blending.
/// If `additive` is true, adds the Gaussian to existing pixels (so it's visible at any background level).
/// If `additive` is false, uses max-blend (the Gaussian only brightens existing pixels).
fn draw_star(img: &mut RgbImage, cx: f64, cy: f64, amp: f64, sigma: f64, additive: bool) {
    let r = (3.0 * sigma).ceil() as i64;
    for dy in -r..=r {
        for dx in -r..=r {
            let (x, y) = (cx as i64 + dx, cy as i64 + dy);
            if !(0..img.width() as i64).contains(&x) || !(0..img.height() as i64).contains(&y) {
                continue;
            }
            let d2 = (dx * dx + dy * dy) as f64;
            let v = (amp * (-d2 / (2.0 * sigma * sigma)).exp()).clamp(0.0, 255.0) as u8;
            let px = img.get_pixel_mut(x as u32, y as u32);
            if additive {
                *px = Rgb([
                    (px.0[0] as u16 + v as u16).min(255) as u8,
                    (px.0[1] as u16 + v as u16).min(255) as u8,
                    (px.0[2] as u16 + v as u16).min(255) as u8,
                ]);
            } else {
                *px = Rgb([px.0[0].max(v), px.0[1].max(v), px.0[2].max(v)]);
            }
        }
    }
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
        let cal = MOCK_CALIBRATION;
        let view = astro::LensView {
            frame_width: W,
            frame_height: H,
            native_width: W,
        };
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

        // Focus "breathing": sigma drifts 1.2..2.0 over a 3-minute cycle so the
        // focus page has something to show without hardware.
        let phase = (now.timestamp().rem_euclid(180)) as f64 / 180.0;
        let sigma = 1.6 + 0.4 * (std::f64::consts::TAU * phase).sin();

        let lst = astro::lst_deg(now, lon);
        for i in 0..STAR_COUNT {
            let ra = unit(i * 2 + 1) * 360.0;
            let dec = unit(i * 2 + 2) * 180.0 - 90.0;
            let aa = astro::ra_dec_to_alt_az(ra, dec, lat, lst);
            if aa.alt_deg < 0.0 {
                continue;
            }
            let pt = astro::alt_az_to_image(aa.alt_deg, aa.az_deg, &cal, &view);
            let (x, y) = (pt.x.round() as i64, pt.y.round() as i64);
            if !(0..W as i64).contains(&x) || !(0..H as i64).contains(&y) {
                continue;
            }
            let mag = 60.0 + unit(i + 777) * 195.0;
            draw_star(&mut img, pt.x, pt.y, mag * scale, sigma, false);
        }

        // Guarantee one bright star near the crop center for the focus page.
        // Use fixed amplitude (not scaled by day/night) and additive blending so it's
        // always visible and measurable at any time of day.
        draw_star(
            &mut img,
            W as f64 / 2.0 + 90.0,
            H as f64 / 2.0 + 60.0,
            200.0,
            sigma,
            true,
        );

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

    #[test]
    fn mock_stars_are_measurable_blobs_not_single_pixels() {
        let mut cam = MockCamera::new();
        let f = cam
            .capture(CaptureParams {
                exposure_us: 5_000_000,
                gain: 8.0,
            })
            .unwrap();
        let (m, _) = crate::capture::focus::measure_star(&f.image, None);
        let hfd = m.hfd.expect("mock sky must contain a measurable star");
        assert!(hfd > 1.0 && hfd < 30.0, "hfd = {hfd}");
        assert!(m.peak > 100, "peak = {}", m.peak);
    }

    #[test]
    fn draw_star_hfd_grows_with_sigma() {
        let mut sharp = RgbImage::from_pixel(200, 200, Rgb([10, 10, 10]));
        let mut soft = RgbImage::from_pixel(200, 200, Rgb([10, 10, 10]));
        draw_star(&mut sharp, 100.0, 100.0, 200.0, 1.0, true);
        draw_star(&mut soft, 100.0, 100.0, 200.0, 2.5, true);
        let h1 = crate::capture::focus::measure_star(&sharp, None)
            .0
            .hfd
            .unwrap();
        let h2 = crate::capture::focus::measure_star(&soft, None)
            .0
            .hfd
            .unwrap();
        assert!(h1 < h2, "{h1} !< {h2}");
    }
}
