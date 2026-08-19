//! Star-trail circle-center detection: every trail pixel's brightness
//! gradient points (anti)radially at the common center, so voting along
//! gradient lines piles up at the celestial pole's image position.
//!
//! `find_pole` is consumed by `web::nights::detect_pole`
//! (`POST /api/nights/{date}/detect-pole`).

use image::GrayImage;

pub struct PoleEstimate {
    pub x: f64,
    pub y: f64,
    pub confidence: f64,
}

/// Mask-circle boundary (image-space px) whose edge gradients must not vote.
pub struct MaskExclusion {
    pub cx: f64,
    pub cy: f64,
    pub r: f64,
}

struct GradPixel {
    x: f64,
    y: f64,
    dx: f64, // unit gradient direction
    dy: f64,
}

/// One interior pixel's raw Sobel response — magnitude not yet thresholded.
struct GradCandidate {
    x: u32,
    y: u32,
    gx: f64,
    gy: f64,
    m: f64,
}

/// The Sobel convolution over every interior pixel with a non-trivial
/// magnitude (`m > 1.0`), plus those magnitudes sorted ascending for
/// percentile lookups. This is the expensive O(w×h) sweep; a caller that
/// needs more than one percentile threshold over the *same* image (e.g.
/// separate position vs. confidence thresholds) should call this once and
/// reuse the result via `threshold_gradients` rather than re-running the
/// convolution per threshold.
fn sobel_gradients(img: &GrayImage) -> (Vec<GradCandidate>, Vec<f64>) {
    let (w, h) = img.dimensions();
    let mut mags = Vec::new();
    let mut all = Vec::new();
    let px = |x: u32, y: u32| f64::from(img.get_pixel(x, y).0[0]);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let gx = px(x + 1, y - 1) + 2.0 * px(x + 1, y) + px(x + 1, y + 1)
                - px(x - 1, y - 1)
                - 2.0 * px(x - 1, y)
                - px(x - 1, y + 1);
            let gy = px(x - 1, y + 1) + 2.0 * px(x, y + 1) + px(x + 1, y + 1)
                - px(x - 1, y - 1)
                - 2.0 * px(x, y - 1)
                - px(x + 1, y - 1);
            let m = gx.hypot(gy);
            if m > 1.0 {
                mags.push(m);
                all.push(GradCandidate { x, y, gx, gy, m });
            }
        }
    }
    mags.sort_by(|a, b| a.total_cmp(b));
    (all, mags)
}

/// Filters precomputed gradient candidates (from `sobel_gradients`) to
/// those above the `percentile`th magnitude percentile (with an absolute
/// floor so flat noise contributes nothing), minus mask-edge pixels.
fn threshold_gradients(
    candidates: &[GradCandidate],
    sorted_mags: &[f64],
    mask: Option<&MaskExclusion>,
    scale: f64,
    percentile: usize,
    floor: f64,
) -> Vec<GradPixel> {
    let thr = sorted_mags
        .get(sorted_mags.len().saturating_mul(percentile) / 100)
        .copied()
        .unwrap_or(f64::MAX)
        .max(floor);
    candidates
        .iter()
        .filter(|c| {
            c.m >= thr
                && mask.is_none_or(|mc| {
                    let d = (f64::from(c.x) - mc.cx * scale).hypot(f64::from(c.y) - mc.cy * scale);
                    (d - mc.r * scale).abs() > 4.0
                })
        })
        .map(|c| GradPixel {
            x: f64::from(c.x),
            y: f64::from(c.y),
            dx: c.gx / c.m,
            dy: c.gy / c.m,
        })
        .collect()
}

/// Sobel gradients above the 97th magnitude percentile (with an absolute
/// floor so flat noise contributes nothing), minus mask-edge pixels.
/// Convenience wrapper for the common single-threshold case; see
/// `sobel_gradients`/`threshold_gradients` for reusing the convolution
/// across multiple thresholds on the same image.
fn strong_gradients(img: &GrayImage, mask: Option<&MaskExclusion>, scale: f64) -> Vec<GradPixel> {
    let (candidates, mags) = sobel_gradients(img);
    threshold_gradients(&candidates, &mags, mask, scale, 97, 60.0)
}

/// Vote along each gradient line into a grid covering `3×` the frame
/// (origin at (-w, -h)), one cell per `cell` px. Returns the accumulator
/// plus its dimensions.
fn vote(pixels: &[GradPixel], w: f64, h: f64, cell: f64) -> (Vec<u32>, usize, usize) {
    let aw = (3.0 * w / cell).ceil() as usize;
    let ah = (3.0 * h / cell).ceil() as usize;
    let mut acc = vec![0u32; aw * ah];
    let span = (3.0 * (w + h)) / cell; // longer than any accumulator diagonal
    for p in pixels {
        let (cx, cy) = ((p.x + w) / cell, (p.y + h) / cell);
        let (dx, dy) = (p.dx, p.dy);
        let mut t = -span;
        while t <= span {
            let ix = (cx + t * dx) as isize;
            let iy = (cy + t * dy) as isize;
            if ix >= 0 && iy >= 0 && (ix as usize) < aw && (iy as usize) < ah {
                acc[iy as usize * aw + ix as usize] += 1;
            }
            t += 1.0;
        }
    }
    (acc, aw, ah)
}

/// Peak of the accumulator smoothed with a `k×k` window (integral image).
fn smoothed_peak(acc: &[u32], aw: usize, ah: usize, k: usize) -> (usize, usize, f64, f64, f64) {
    let mut integral = vec![0u64; (aw + 1) * (ah + 1)];
    for y in 0..ah {
        for x in 0..aw {
            integral[(y + 1) * (aw + 1) + x + 1] = u64::from(acc[y * aw + x])
                + integral[y * (aw + 1) + x + 1]
                + integral[(y + 1) * (aw + 1) + x]
                - integral[y * (aw + 1) + x];
        }
    }
    let sum = |x: usize, y: usize| -> u64 {
        let (x2, y2) = ((x + k).min(aw), (y + k).min(ah));
        integral[y2 * (aw + 1) + x2] + integral[y * (aw + 1) + x]
            - integral[y * (aw + 1) + x2]
            - integral[y2 * (aw + 1) + x]
    };
    let mut best = (0usize, 0usize, 0u64);
    let mut samples = Vec::new();
    for y in 0..ah.saturating_sub(k) {
        for x in 0..aw.saturating_sub(k) {
            let s = sum(x, y);
            if s > best.2 {
                best = (x, y, s);
            }
        }
    }
    // Background statistics from a deterministic sparse sample grid, excluding
    // any sample whose window overlaps the detected peak's own window (it
    // would otherwise contaminate the "background" floor with the signal
    // itself and suppress the z-score for genuine detections).
    for y in (0..ah.saturating_sub(k)).step_by(13) {
        for x in (0..aw.saturating_sub(k)).step_by(13) {
            if x.abs_diff(best.0) < k && y.abs_diff(best.1) < k {
                continue;
            }
            samples.push(sum(x, y) as f64);
        }
    }
    let mean = samples.iter().sum::<f64>() / samples.len().max(1) as f64;
    let var = samples.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / samples.len().max(1) as f64;
    (best.0, best.1, best.2 as f64, mean, var.sqrt())
}

pub fn find_pole(img: &GrayImage, mask: Option<&MaskExclusion>) -> PoleEstimate {
    let (fw, fh) = img.dimensions();
    // Coarse pass on a ≤640 px copy.
    let scale = (640.0 / f64::from(fw.max(fh))).min(1.0);
    let small = if scale < 1.0 {
        image::imageops::resize(
            img,
            (f64::from(fw) * scale) as u32,
            (f64::from(fh) * scale) as u32,
            image::imageops::FilterType::Triangle,
        )
    } else {
        img.clone()
    };
    let (sw, sh) = (f64::from(small.width()), f64::from(small.height()));
    let cell = 2.0;
    let k = 5;
    // One Sobel sweep over the small image, reused below at two different
    // percentile thresholds (position vs. confidence) so the O(w×h)
    // convolution itself only runs once.
    let (candidates, mags) = sobel_gradients(&small);
    let pixels = threshold_gradients(&candidates, &mags, mask, scale, 97, 60.0);
    let (acc, aw, ah) = vote(&pixels, sw, sh, cell);
    let (bx, by, _, _, _) = smoothed_peak(&acc, aw, ah, k);
    // Window center, accumulator → small-image → full-res coordinates.
    let coarse_x = ((bx as f64 + k as f64 / 2.0) * cell - sw) / scale;
    let coarse_y = ((by as f64 + k as f64 / 2.0) * cell - sh) / scale;

    // Confidence: a second, more permissive vote pass over the same
    // candidates. The strict percentile used for `pixels` above keeps only
    // the cleanest edge pixels, which is what position-finding wants, but
    // it thins the accumulator enough that a genuine trail peak and a
    // random-noise peak both look statistically unremarkable (z-scores ~4
    // vs ~15 — too close to threshold reliably). Admitting more of the
    // trail's weaker edge pixels via a lower percentile widens that gap
    // (~20+ vs ~4) without touching the position pipeline.
    let confidence_pixels = threshold_gradients(&candidates, &mags, mask, scale, 85, 60.0);
    let (confidence_acc, caw, cah) = vote(&confidence_pixels, sw, sh, cell);
    let (_, _, peak, mean, sd) = smoothed_peak(&confidence_acc, caw, cah, k);
    let z = if sd > 0.0 { (peak - mean) / sd } else { 0.0 };
    let confidence = (z / 25.0).clamp(0.0, 1.0);

    // Refinement: full-res vote restricted to a window around the coarse
    // hit. `win` must be wide enough to still reach the true center when
    // the coarse pass is off (a far off-frame pole is inherently a poorly
    // conditioned estimate from partial, non-circular trail arcs — the
    // coarse hit can land 100+ px away) but not so wide that it re-admits
    // enough distant pixels to reconstruct that same coarse bias locally.
    let win = 140.0;
    let fine = strong_gradients(img, mask, 1.0);
    let mut facc = vec![0u32; (2.0 * win) as usize * (2.0 * win) as usize];
    let faw = (2.0 * win) as usize;
    for p in &fine {
        // Distance from the window center to the pixel's gradient line.
        let (rx, ry) = (coarse_x - p.x, coarse_y - p.y);
        let cross = rx * p.dy - ry * p.dx;
        if cross.abs() > win * 1.5 {
            continue; // line misses the window entirely
        }
        let t0 = rx * p.dx + ry * p.dy; // closest-approach parameter
        let mut t = t0 - win * 1.5;
        while t <= t0 + win * 1.5 {
            let ix = (p.x + t * p.dx - coarse_x + win) as isize;
            let iy = (p.y + t * p.dy - coarse_y + win) as isize;
            if ix >= 0 && iy >= 0 && (ix as usize) < faw && (iy as usize) < faw {
                facc[iy as usize * faw + ix as usize] += 1;
            }
            t += 1.0;
        }
    }
    let (fx, fy, _, _, _) = smoothed_peak(&facc, faw, faw, 7);
    PoleEstimate {
        x: coarse_x - win + fx as f64 + 3.5,
        y: coarse_y - win + fy as f64 + 3.5,
        confidence,
    }
}

/// Test-only synthetic-startrails builder, `pub(crate)` so the endpoint
/// tests (Task 4) can seed a night with it too.
#[cfg(test)]
pub(crate) mod tests_support {
    use image::{GrayImage, Luma};

    /// Concentric arcs (0.75 of each circle, ~1.5 px thick) on a dark sky —
    /// structurally what a startrails image is.
    pub(crate) fn synthetic_trails(w: u32, h: u32, cx: f64, cy: f64) -> GrayImage {
        let mut img = GrayImage::from_pixel(w, h, Luma([10u8]));
        for k in 1..90 {
            let r = 14.0 * k as f64;
            let steps = (r * 5.0) as usize;
            for i in 0..steps {
                let a = 0.2 + 1.5 * std::f64::consts::PI * i as f64 / steps as f64;
                for (dx, dy) in [(0.0, 0.0), (0.6, 0.0), (0.0, 0.6)] {
                    let px = (cx + r * a.cos() + dx).round() as i64;
                    let py = (cy + r * a.sin() + dy).round() as i64;
                    if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h {
                        img.put_pixel(px as u32, py as u32, Luma([230u8]));
                    }
                }
            }
        }
        img
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::synthetic_trails;
    use super::*;
    use image::{GrayImage, Luma};

    #[test]
    fn finds_an_in_frame_pole() {
        let img = synthetic_trails(1280, 960, 900.0, 300.0);
        let e = find_pole(&img, None);
        let d = ((e.x - 900.0).powi(2) + (e.y - 300.0).powi(2)).sqrt();
        assert!(d <= 4.0, "off by {d} px (found {}, {})", e.x, e.y);
        assert!(e.confidence > 0.8, "confidence {}", e.confidence);
    }

    #[test]
    fn finds_a_pole_outside_the_frame() {
        let img = synthetic_trails(1280, 960, 1500.0, -200.0);
        let e = find_pole(&img, None);
        let d = ((e.x - 1500.0).powi(2) + (e.y + 200.0).powi(2)).sqrt();
        assert!(d <= 15.0, "off by {d} px (found {}, {})", e.x, e.y);
    }

    #[test]
    fn noise_yields_low_confidence() {
        // Deterministic LCG noise — no rand dependency.
        let mut s: u64 = 42;
        let mut img = GrayImage::new(1280, 960);
        for p in img.pixels_mut() {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *p = Luma([(s >> 33) as u8]);
        }
        let e = find_pole(&img, None);
        assert!(e.confidence < 0.2, "confidence {}", e.confidence);
    }

    #[test]
    fn mask_edge_is_excluded_from_voting() {
        // Trails + a bright mask-boundary ring centered elsewhere; with the
        // exclusion the ring must not hijack the peak.
        let mut img = synthetic_trails(1280, 960, 900.0, 300.0);
        let (mx, my, mr) = (640.0, 480.0, 420.0);
        for i in 0..12_000 {
            let a = i as f64 * std::f64::consts::TAU / 12_000.0;
            let px = (mx + mr * a.cos()) as i64;
            let py = (my + mr * a.sin()) as i64;
            if px >= 0 && py >= 0 && (px as u32) < 1280 && (py as u32) < 960 {
                img.put_pixel(px as u32, py as u32, Luma([255u8]));
            }
        }
        let e = find_pole(
            &img,
            Some(&MaskExclusion {
                cx: mx,
                cy: my,
                r: mr,
            }),
        );
        let d = ((e.x - 900.0).powi(2) + (e.y - 300.0).powi(2)).sqrt();
        assert!(d <= 6.0, "off by {d} px (found {}, {})", e.x, e.y);
    }

    #[test]
    fn finds_the_pole_in_a_real_startrails() {
        let bytes = std::fs::read("tests/fixtures/startrails-2026-08-12.jpg").unwrap();
        let img = image::load_from_memory(&bytes).unwrap().to_luma8();
        let e = find_pole(&img, None);
        let d = ((e.x - 882.5).powi(2) + (e.y - 218.5).powi(2)).sqrt();
        assert!(d <= 12.0, "off by {d} px (found {}, {})", e.x, e.y);
        assert!(e.confidence > 0.4, "confidence {}", e.confidence);
    }
}
