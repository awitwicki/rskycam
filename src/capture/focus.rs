//! Star detection and HFD (half-flux diameter) for the focus aid.
//! Pure functions — no camera, no clock, no I/O.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use bytes::Bytes;
use image::RgbImage;
use serde::Serialize;

use crate::camera::{CameraError, Frame};

/// Star window is 64x64 px — matches the star.png the UI zooms into.
#[allow(dead_code)]
pub const WINDOW: u32 = 64;

#[allow(dead_code)]
pub struct StarMetrics {
    pub hfd: Option<f64>,
    pub star_x: u32,
    pub star_y: u32,
    pub peak: u8,
    pub saturated: bool,
}

/// Rec.601 luma as f32, one entry per pixel, row-major.
#[allow(dead_code)]
fn luma(img: &RgbImage) -> Vec<f32> {
    img.pixels()
        .map(|p| 0.299 * p.0[0] as f32 + 0.587 * p.0[1] as f32 + 0.114 * p.0[2] as f32)
        .collect()
}

/// 3x3 box mean at (x,y), edge-clamped — one lone hot pixel can't win argmax.
#[allow(dead_code)]
fn smoothed(l: &[f32], w: u32, h: u32, x: u32, y: u32) -> f32 {
    let mut sum = 0.0;
    for dy in -1i64..=1 {
        for dx in -1i64..=1 {
            let sx = (x as i64 + dx).clamp(0, w as i64 - 1) as u32;
            let sy = (y as i64 + dy).clamp(0, h as i64 - 1) as u32;
            sum += l[(sy * w + sx) as usize];
        }
    }
    sum / 9.0
}

/// Brightest star: argmax of smoothed luma, optionally restricted to the
/// manual mask circle `(cx, cy, r)`. Returns (metrics, 64x64 window image).
#[allow(dead_code)]
pub fn measure_star(img: &RgbImage, mask: Option<(f64, f64, f64)>) -> (StarMetrics, RgbImage) {
    let (w, h) = img.dimensions();
    let l = luma(img);

    let mut best = (0u32, 0u32, f32::MIN);
    for y in 0..h {
        for x in 0..w {
            if let Some((cx, cy, r)) = mask {
                let (dx, dy) = (x as f64 - cx, y as f64 - cy);
                if dx * dx + dy * dy > r * r {
                    continue;
                }
            }
            let v = smoothed(&l, w, h, x, y);
            if v > best.2 {
                best = (x, y, v);
            }
        }
    }
    let (px, py, _) = best;

    // 64x64 window centered on the peak, clamped inside the frame.
    let x0 = (px.saturating_sub(WINDOW / 2)).min(w.saturating_sub(WINDOW));
    let y0 = (py.saturating_sub(WINDOW / 2)).min(h.saturating_sub(WINDOW));
    let window = image::imageops::crop_imm(img, x0, y0, WINDOW.min(w), WINDOW.min(h)).to_image();

    // Background = median of the window's 2px border ring.
    let wl = luma(&window);
    let (ww, wh) = window.dimensions();
    let mut border: Vec<f32> = Vec::new();
    for y in 0..wh {
        for x in 0..ww {
            if x < 2 || y < 2 || x >= ww - 2 || y >= wh - 2 {
                border.push(wl[(y * ww + x) as usize]);
            }
        }
    }
    border.sort_by(|a, b| a.total_cmp(b));
    let bg = border[border.len() / 2];

    // Flux-weighted centroid, then HFD = 2 * sum(f*r) / sum(f).
    // Ignore near-background flux so noise doesn't drag the metric.
    // Collect thresholded flux values once, reuse for both centroid and radius calculations.
    let mut flux_map: Vec<(u32, u32, f64)> = Vec::new();
    let mut fsum = 0.0f64;
    for y in 0..wh {
        for x in 0..ww {
            let f = (wl[(y * ww + x) as usize] - bg).max(0.0) as f64;
            if f >= 4.0 {
                flux_map.push((x, y, f));
                fsum += f;
            }
        }
    }
    let peak = img.get_pixel(px, py).0.into_iter().max().unwrap_or(0);
    let hfd = if fsum > 0.0 {
        // Compute centroid from collected flux values.
        let (mut cx, mut cy) = (0.0f64, 0.0f64);
        for &(x, y, f) in &flux_map {
            cx += f * x as f64;
            cy += f * y as f64;
        }
        let (cx, cy) = (cx / fsum, cy / fsum);
        // Compute HFD radius sum from the same flux collection.
        let mut rsum = 0.0f64;
        for &(x, y, f) in &flux_map {
            rsum += f * ((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt();
        }
        Some(2.0 * rsum / fsum)
    } else {
        None
    };

    (
        StarMetrics {
            hfd,
            star_x: px,
            star_y: py,
            peak,
            saturated: peak >= 250,
        },
        window,
    )
}

#[allow(dead_code)]
pub fn encode_png(img: &RgbImage) -> Result<Vec<u8>, CameraError> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| CameraError::Capture(e.to_string()))?;
    Ok(buf.into_inner())
}

/// Focus session state shared between the web layer and the capture loop.
/// `last_activity` is a std Mutex touched only from sync code paths.
pub struct FocusShared {
    enabled: AtomicBool,
    exposure_us: AtomicU64,
    gain_bits: AtomicU64,
    timeout_ms: AtomicU64,
    last_activity: Mutex<Instant>,
    /// Wakes the capture loop out of its interval/day-pause sleep so
    /// enabling focus takes effect immediately, not one interval later.
    pub(crate) wake: tokio::sync::Notify,
}

impl FocusShared {
    pub fn new() -> Self {
        FocusShared {
            enabled: AtomicBool::new(false),
            exposure_us: AtomicU64::new(1_000_000),
            gain_bits: AtomicU64::new(1.0f64.to_bits()),
            timeout_ms: AtomicU64::new(60_000),
            last_activity: Mutex::new(Instant::now()),
            wake: tokio::sync::Notify::new(),
        }
    }
    pub fn enable(&self, exposure_us: u64) {
        self.exposure_us.store(exposure_us, Ordering::Relaxed);
        self.bump();
        self.enabled.store(true, Ordering::Relaxed);
        self.wake.notify_one();
    }
    pub fn disable(&self) {
        self.enabled.store(false, Ordering::Relaxed);
    }
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }
    pub fn exposure_us(&self) -> u64 {
        self.exposure_us.load(Ordering::Relaxed)
    }
    /// Set independently from `enable` — the web layer applies it before
    /// or alongside enabling, but it never toggles enabled/bump/wake itself.
    pub fn set_gain(&self, gain: f64) {
        self.gain_bits.store(gain.to_bits(), Ordering::Relaxed);
    }
    pub fn gain(&self) -> f64 {
        f64::from_bits(self.gain_bits.load(Ordering::Relaxed))
    }
    #[allow(dead_code)]
    pub fn set_timeout_ms(&self, ms: u64) {
        self.timeout_ms.store(ms, Ordering::Relaxed);
    }
    /// A viewer is watching: refresh the auto-exit deadline.
    #[allow(dead_code)]
    pub fn bump(&self) {
        *self.last_activity.lock().expect("not poisoned") = Instant::now();
    }
    /// Should the loop capture a focus frame this iteration?
    /// Auto-disables (and logs) when no viewer bumped within the timeout.
    pub fn active(&self) -> bool {
        if !self.enabled() {
            return false;
        }
        let stale = self.last_activity.lock().expect("not poisoned").elapsed()
            > std::time::Duration::from_millis(self.timeout_ms.load(Ordering::Relaxed));
        if stale {
            self.disable();
            tracing::info!("focus mode auto-exited (no viewer)");
            return false;
        }
        true
    }
}

impl Default for FocusShared {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FocusMeta {
    pub timestamp: String, // ISO 8601
    pub hfd: Option<f64>,
    pub star_x: u32,
    pub star_y: u32,
    pub peak: u8,
    pub saturated: bool,
    pub exposure_us: u64,
    pub gain: f64,
}

// Fields are read by the web layer's focus-mode endpoint (Task 3) and by
// this task's capture-loop tests; not yet read anywhere in the main binary.
#[allow(dead_code)]
pub struct FocusFrame {
    pub preview_jpeg: Bytes,
    pub star_png: Bytes,
    pub meta: FocusMeta,
}

/// Center-50% crop JPEG + star window PNG + metrics for one raw frame.
/// No mask/overlay/darks — the focus aid wants the sensor as-is.
pub fn focus_frame(
    frame: &Frame,
    mask: Option<(f64, f64, f64)>,
) -> Result<FocusFrame, CameraError> {
    // Full frame, uncropped -- the browser handles zoom/pan itself, so the
    // server no longer picks a fixed region for the viewer.
    let preview_jpeg = Bytes::from(crate::camera::encode_jpeg(&frame.image)?);
    let (m, window) = measure_star(&frame.image, mask);
    let star_png = Bytes::from(encode_png(&window)?);
    Ok(FocusFrame {
        preview_jpeg,
        star_png,
        meta: FocusMeta {
            timestamp: frame
                .timestamp
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            hfd: m.hfd,
            star_x: m.star_x,
            star_y: m.star_y,
            peak: m.peak,
            saturated: m.saturated,
            exposure_us: frame.exposure_us,
            gain: frame.gain,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    /// Additive Gaussian star on a flat background.
    fn gaussian_frame(w: u32, h: u32, cx: f64, cy: f64, amp: f64, sigma: f64, bg: u8) -> RgbImage {
        let mut img = RgbImage::from_pixel(w, h, Rgb([bg, bg, bg]));
        for (x, y, px) in img.enumerate_pixels_mut() {
            let d2 = (x as f64 - cx).powi(2) + (y as f64 - cy).powi(2);
            let v = (bg as f64 + amp * (-d2 / (2.0 * sigma * sigma)).exp()).min(255.0) as u8;
            *px = Rgb([v, v, v]);
        }
        img
    }

    #[test]
    fn hfd_of_a_gaussian_matches_the_analytic_value() {
        // For a 2D Gaussian, mean radial distance = sigma * sqrt(pi/2),
        // so HFD = 2 * sigma * 1.2533. sigma=2 -> ~5.0 px.
        let img = gaussian_frame(200, 200, 100.0, 100.0, 180.0, 2.0, 10);
        let (m, window) = measure_star(&img, None);
        assert_eq!((window.width(), window.height()), (64, 64));
        assert_eq!((m.star_x, m.star_y), (100, 100));
        let hfd = m.hfd.expect("star present");
        assert!((hfd - 5.0).abs() < 1.0, "hfd = {hfd}");
        assert!(!m.saturated);
    }

    #[test]
    fn hfd_shrinks_as_focus_improves() {
        let soft = gaussian_frame(200, 200, 100.0, 100.0, 180.0, 3.0, 10);
        let sharp = gaussian_frame(200, 200, 100.0, 100.0, 180.0, 1.0, 10);
        let hfd_soft = measure_star(&soft, None).0.hfd.unwrap();
        let hfd_sharp = measure_star(&sharp, None).0.hfd.unwrap();
        assert!(hfd_sharp < hfd_soft, "{hfd_sharp} !< {hfd_soft}");
    }

    #[test]
    fn a_blurred_blob_beats_a_brighter_single_hot_pixel() {
        let mut img = gaussian_frame(200, 200, 60.0, 60.0, 150.0, 2.0, 10);
        img.put_pixel(150, 150, Rgb([255, 255, 255])); // hot pixel
        let (m, _) = measure_star(&img, None);
        // 3x3 smoothing: blob keeps ~150, lone pixel drops to ~(255+8*10)/9 ~= 37
        assert_eq!((m.star_x, m.star_y), (60, 60));
    }

    #[test]
    fn saturated_peak_is_flagged() {
        let img = gaussian_frame(200, 200, 100.0, 100.0, 255.0, 2.0, 10);
        let (m, _) = measure_star(&img, None);
        assert!(m.saturated);
    }

    #[test]
    fn flat_frame_yields_no_hfd() {
        let img = RgbImage::from_pixel(100, 100, Rgb([12, 12, 12]));
        let (m, _) = measure_star(&img, None);
        assert!(m.hfd.is_none());
    }

    #[test]
    fn mask_circle_excludes_stars_outside_it() {
        // Bright star at (20,20) outside the circle, dimmer one at (100,100) inside.
        let mut img = gaussian_frame(200, 200, 100.0, 100.0, 120.0, 2.0, 10);
        let outside = gaussian_frame(200, 200, 20.0, 20.0, 250.0, 2.0, 10);
        for (x, y, px) in img.enumerate_pixels_mut() {
            let o = outside.get_pixel(x, y).0[0];
            let m = px.0[0].max(o);
            *px = Rgb([m, m, m]);
        }
        let (m, _) = measure_star(&img, Some((100.0, 100.0, 40.0)));
        assert_eq!((m.star_x, m.star_y), (100, 100));
    }

    #[test]
    fn star_window_clamps_at_frame_edges() {
        let img = gaussian_frame(80, 80, 2.0, 2.0, 200.0, 1.5, 10);
        let (m, window) = measure_star(&img, None);
        assert_eq!((window.width(), window.height()), (64, 64));
        assert!(m.hfd.is_some());
    }

    #[test]
    fn png_roundtrips() {
        let img = RgbImage::from_pixel(64, 64, Rgb([50, 90, 130]));
        let png = encode_png(&img).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        let back = image::load_from_memory(&png).unwrap().to_rgb8();
        assert_eq!(back.get_pixel(0, 0).0, [50, 90, 130]);
    }

    #[test]
    fn focus_frame_builds_full_frame_preview_and_metrics() {
        let img = gaussian_frame(200, 100, 100.0, 50.0, 200.0, 2.0, 10);
        let frame = crate::camera::Frame {
            image: img,
            timestamp: chrono::Utc::now(),
            exposure_us: 500_000,
            gain: 4.0,
        };
        let ff = focus_frame(&frame, None).unwrap();
        let preview = image::load_from_memory(&ff.preview_jpeg).unwrap();
        assert_eq!((preview.width(), preview.height()), (200, 100)); // full frame, uncropped
        assert!(ff.meta.hfd.is_some());
        assert_eq!(ff.meta.exposure_us, 500_000);
        assert!(!ff.meta.timestamp.is_empty());
    }

    #[test]
    fn gain_defaults_low_and_is_independently_settable() {
        let shared = FocusShared::new();
        // Defaults to the camera's minimum, not an inherited AE/manual
        // value -- a night-tuned high gain must never be the silent
        // default for a fresh focus session.
        assert_eq!(shared.gain(), 1.0);
        shared.set_gain(6.5);
        assert_eq!(shared.gain(), 6.5);
        // Independent of enable()/exposure_us -- setting one never
        // touches the other.
        shared.enable(250_000);
        assert_eq!(shared.gain(), 6.5);
        assert_eq!(shared.exposure_us(), 250_000);
    }
}
