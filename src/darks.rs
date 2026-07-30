//! Dark-frame hot-pixel correction: a library of "dark" frames (lens
//! covered, no light) at a fixed exposure/gain sweep, subtracted from real
//! frames to remove hot pixels. See
//! docs/superpowers/specs/2026-07-29-dark-frame-correction-design.md.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use image::{ImageEncoder, RgbImage};
use serde::{Deserialize, Serialize};

use crate::settings::{CameraDriver, DarkFrameSettings};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DarkEntry {
    pub exposure_us: u64,
    pub gain: f64,
    pub file: String,
    pub captured_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DarksLibrary {
    pub entries: Vec<DarkEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DarksProgress {
    pub current: u32,
    pub total: u32,
}

/// Directory holding one (driver, width, height) dark library. Libraries for
/// different combinations coexist on disk; switching driver/resolution just
/// means a different (or no) directory matches.
pub fn library_dir(data_dir: &Path, driver: CameraDriver, width: u32, height: u32) -> PathBuf {
    let driver_name = match driver {
        CameraDriver::Asi => "asi",
        CameraDriver::Rpicam => "rpicam",
        CameraDriver::Mock => "mock",
    };
    data_dir
        .join("darks")
        .join(format!("{driver_name}_{width}x{height}"))
}

fn manifest_path(dir: &Path) -> PathBuf {
    dir.join("manifest.json")
}

/// Loads the manifest for a library directory. Returns an empty library —
/// not an error — when the directory or manifest doesn't exist yet (no
/// sweep has been run for this driver/resolution) or is unreadable/corrupt.
pub fn load_manifest(dir: &Path) -> DarksLibrary {
    fs::read_to_string(manifest_path(dir))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save_manifest(dir: &Path, lib: &DarksLibrary) -> anyhow::Result<()> {
    fs::create_dir_all(dir)?;
    let tmp = manifest_path(dir).with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(lib)?)?;
    fs::rename(&tmp, manifest_path(dir))?;
    Ok(())
}

/// The fixed exposure sweep, in microseconds: 0.5s, 2s, 8s, 20s, 60s.
pub const SWEEP_EXPOSURES_US: [u64; 5] = [500_000, 2_000_000, 8_000_000, 20_000_000, 60_000_000];

/// The 5x3 = 15 (exposure_us, gain) combinations for a sweep: every fixed
/// exposure crossed with {min, mid, max} of the currently-configured gain
/// range. Computed fresh each time a sweep starts (not stored), since gain
/// is a raw driver-specific unit — a literal number wouldn't mean the same
/// thing on rpicam (~1-16 analogue multiplier) vs. ASI (~0-600 raw units).
pub fn sweep_targets(gain_min: f64, gain_max: f64) -> Vec<(u64, f64)> {
    let gains = [gain_min, (gain_min + gain_max) / 2.0, gain_max];
    let mut targets = Vec::with_capacity(SWEEP_EXPOSURES_US.len() * gains.len());
    for &exposure_us in &SWEEP_EXPOSURES_US {
        for &gain in &gains {
            targets.push((exposure_us, gain));
        }
    }
    targets
}

/// Trim a sweep to the points that can ever actually be SUBTRACTED, given
/// the configured apply thresholds and the configured exposure ceiling:
/// exposures are capped at `exposure_us_cap` (frames can never expose
/// longer, so a longer dark would just soak sweep time), then any point
/// below `min_exposure_us_to_apply`/`min_gain_to_apply` is dropped — the
/// apply gate would never let it match a real frame. Sorted and deduped
/// (capping typically collapses several exposures onto the cap). May return
/// an empty list when the thresholds exclude everything — the caller
/// decides what a sensible fallback is.
pub fn appliable_targets(
    targets: Vec<(u64, f64)>,
    settings: &DarkFrameSettings,
    exposure_us_cap: u64,
) -> Vec<(u64, f64)> {
    let mut out: Vec<(u64, f64)> = targets
        .into_iter()
        .map(|(exposure_us, gain)| (exposure_us.min(exposure_us_cap), gain))
        .filter(|&(exposure_us, gain)| {
            exposure_us >= settings.min_exposure_us_to_apply && gain >= settings.min_gain_to_apply
        })
        .collect();
    out.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    out.dedup();
    out
}

/// How many frames to capture and median-stack for one sweep point. A
/// single dark stamps its own random read noise (and any one-off cosmic-ray
/// hit) into every corrected light frame; a median stack keeps only the
/// repeatable fixed pattern. Sized by a per-point time budget (~2 minutes)
/// so short exposures get the full 10-frame stack while a 60 s dark doesn't
/// balloon the sweep — never fewer than 3, the minimum for a median to
/// reject a single outlier frame.
pub fn stack_count(exposure_us: u64) -> usize {
    // ~3.3 min per point: with the sweep trimmed to only appliable points
    // (typically 1-2 of them) a full 10-frame stack at a 20 s exposure is
    // affordable, and more samples directly improve how well the master
    // captures blinky (telegraph-noise) pixels.
    const BUDGET_US: u64 = 200_000_000;
    ((BUDGET_US / exposure_us.max(1)) as usize).clamp(3, 10)
}

/// Combine equally-sized frames into one master dark: per pixel/channel,
/// reject the single lowest and single highest sample and average the rest
/// (min/max-clipped mean — for a 3-frame stack this IS the median). One
/// outlier sample per pixel (a cosmic-ray hit, a read-noise spike in one
/// frame) is discarded outright instead of bleeding into the master, which
/// is the entire reason to stack rather than keep a single shot. Stacks of
/// fewer than 3 frames (captures failed mid-point) fall back to a plain
/// mean — no room to clip. Returns `None` for an empty stack or mismatched
/// dimensions (a mid-sweep resolution change; blending different sizes
/// would be garbage).
pub fn stack_master(frames: &[RgbImage]) -> Option<RgbImage> {
    let first = frames.first()?;
    let (w, h) = first.dimensions();
    if frames.iter().any(|f| f.dimensions() != (w, h)) {
        return None;
    }
    let len = (w * h * 3) as usize;
    let mut sum = vec![0u32; len];
    let mut min = vec![255u8; len];
    let mut max = vec![0u8; len];
    for f in frames {
        for (i, &v) in f.as_raw().iter().enumerate() {
            sum[i] += u32::from(v);
            if v < min[i] {
                min[i] = v;
            }
            if v > max[i] {
                max[i] = v;
            }
        }
    }
    let n = frames.len() as u32;
    let out: Vec<u8> = if n >= 3 {
        let kept = n - 2;
        sum.iter()
            .zip(&min)
            .zip(&max)
            .map(|((&s, &lo), &hi)| {
                let clipped = s - u32::from(lo) - u32::from(hi);
                ((clipped + kept / 2) / kept) as u8
            })
            .collect()
    } else {
        sum.iter().map(|&s| ((s + n / 2) / n) as u8).collect()
    };
    RgbImage::from_raw(w, h, out)
}

/// Relative distance over (exposure_us, gain): each axis is normalized by
/// its own magnitude before summing, so the two axes stay comparable
/// despite very different scales (exposure in microseconds up to
/// 60,000,000; gain up to ~600 on the ASI driver). A raw-difference sum
/// would let exposure completely dominate the match and ignore gain.
fn distance(entry: &DarkEntry, exposure_us: u64, gain: f64) -> f64 {
    let exp_a = exposure_us as f64;
    let exp_b = entry.exposure_us as f64;
    let exp_term = (exp_a - exp_b).abs() / exp_a.max(exp_b).max(1.0);
    let gain_term = (gain - entry.gain).abs() / gain.max(entry.gain).max(1.0);
    exp_term + gain_term
}

/// Nearest dark by relative (exposure_us, gain) distance, used as-is (no
/// interpolation or pixel scaling). `None` for an empty library.
pub fn nearest_match(entries: &[DarkEntry], exposure_us: u64, gain: f64) -> Option<&DarkEntry> {
    entries.iter().min_by(|a, b| {
        distance(a, exposure_us, gain)
            .partial_cmp(&distance(b, exposure_us, gain))
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

/// Per-pixel saturating subtraction, clamped at 0. A dimension mismatch
/// (a stale library slipping through) is treated as "no dark available" —
/// callers only reach this after already matching on the (driver, width,
/// height) library key, so a mismatch here indicates a corrupt file rather
/// than the normal case.
pub fn subtract_dark(img: &mut RgbImage, dark: &RgbImage) {
    if img.dimensions() != dark.dimensions() {
        return;
    }
    for (px, dpx) in img.pixels_mut().zip(dark.pixels()) {
        for c in 0..3 {
            px.0[c] = px.0[c].saturating_sub(dpx.0[c]);
        }
    }
}

/// Hot-pixel cutoff for a master dark: its median luma (the ordinary
/// dark-current pedestal) plus this margin. Pixels above it are treated as
/// defective and REPAIRED (replaced by neighbors) rather than merely
/// subtracted — see `repair_hot_pixels`.
const HOT_PIXEL_MARGIN: u8 = 25;

fn luma(px: &image::Rgb<u8>) -> u8 {
    // Same Rec.601 weighting as camera::mean_brightness, in integer math.
    ((u32::from(px.0[0]) * 299 + u32::from(px.0[1]) * 587 + u32::from(px.0[2]) * 114) / 1000) as u8
}

fn median_luma(img: &RgbImage) -> u8 {
    let mut hist = [0u32; 256];
    for px in img.pixels() {
        hist[luma(px) as usize] += 1;
    }
    let half = (img.width() * img.height()).div_ceil(2);
    let mut seen = 0u32;
    for (v, &count) in hist.iter().enumerate() {
        seen += count;
        if seen >= half {
            return v as u8;
        }
    }
    255
}

/// Replace every pixel the master dark marks as hot (well above the dark's
/// own pedestal) with the median of its non-hot 8-neighbors in the light
/// frame. Subtraction alone underestimates blinking (telegraph-noise)
/// pixels: the stack's clipped mean averages a sometimes-on defect down to
/// a fraction of its lit brightness, so subtracting leaves a bright
/// residual at exactly the same position every frame. Replacement removes
/// the defect completely regardless of how bright it happened to be in
/// this particular frame — the standard hot-pixel treatment in allsky
/// stacks. Pixels whose neighbors are all hot too are left as-is (a
/// defective cluster has no clean data to borrow).
pub fn repair_hot_pixels(img: &mut RgbImage, dark: &RgbImage) {
    if img.dimensions() != dark.dimensions() {
        return;
    }
    let cutoff = median_luma(dark).saturating_add(HOT_PIXEL_MARGIN);
    let (w, h) = img.dimensions();
    let hot: Vec<bool> = dark.pixels().map(|p| luma(p) >= cutoff).collect();
    let src = img.clone(); // repairs read the original, not partial repairs
    let mut neighbor_vals: [Vec<u8>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for y in 0..h {
        for x in 0..w {
            if !hot[(y * w + x) as usize] {
                continue;
            }
            for v in &mut neighbor_vals {
                v.clear();
            }
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                        continue;
                    }
                    if hot[(ny as u64 * u64::from(w) + nx as u64) as usize] {
                        continue;
                    }
                    let np = src.get_pixel(nx as u32, ny as u32);
                    for (vals, &v) in neighbor_vals.iter_mut().zip(&np.0) {
                        vals.push(v);
                    }
                }
            }
            if neighbor_vals[0].is_empty() {
                continue; // fully hot neighborhood — nothing clean to copy
            }
            let px = img.get_pixel_mut(x, y);
            for c in 0..3 {
                neighbor_vals[c].sort_unstable();
                px.0[c] = neighbor_vals[c][neighbor_vals[c].len() / 2];
            }
        }
    }
}

/// The gate: darks only apply when enabled and both thresholds are
/// strictly exceeded. Below either threshold hot pixels aren't visible
/// enough to be worth the nearest-match imprecision.
pub fn should_apply(settings: &DarkFrameSettings, exposure_us: u64, gain: f64) -> bool {
    settings.enabled
        && gain > settings.min_gain_to_apply
        && exposure_us > settings.min_exposure_us_to_apply
}

fn encode_png(img: &RgbImage) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    image::codecs::png::PngEncoder::new(&mut buf).write_image(
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(buf)
}

fn load_png(path: &Path) -> anyhow::Result<RgbImage> {
    Ok(image::open(path)?.to_rgb8())
}

/// Subtracts the nearest-matching dark from `img` in place, if darks are
/// enabled, the gate thresholds are met, and a library exists for
/// `(driver, width, height)`. A no-op in every other case (disabled, below
/// threshold, no library, no readable file on disk) — the light frame
/// always passes through unless every condition is met.
#[allow(clippy::too_many_arguments)]
pub fn apply_if_available(
    img: &mut RgbImage,
    data_dir: &Path,
    driver: CameraDriver,
    width: u32,
    height: u32,
    settings: &DarkFrameSettings,
    exposure_us: u64,
    gain: f64,
) {
    if !should_apply(settings, exposure_us, gain) {
        return;
    }
    let dir = library_dir(data_dir, driver, width, height);
    let lib = load_manifest(&dir);
    let Some(entry) = nearest_match(&lib.entries, exposure_us, gain) else {
        return;
    };
    let Ok(dark) = load_png(&dir.join(&entry.file)) else {
        return;
    };
    subtract_dark(img, &dark);
    repair_hot_pixels(img, &dark);
}

/// Writes one captured dark image to the library directory and updates the
/// on-disk manifest, creating both if this is the first capture for this
/// (driver, width, height). Re-capturing the same (exposure_us, gain) pair
/// (e.g. retrying a step) replaces the existing entry rather than
/// duplicating it.
#[allow(clippy::too_many_arguments)]
pub fn add_entry(
    data_dir: &Path,
    driver: CameraDriver,
    width: u32,
    height: u32,
    exposure_us: u64,
    gain: f64,
    img: &RgbImage,
    captured_at: DateTime<Utc>,
) -> anyhow::Result<()> {
    let dir = library_dir(data_dir, driver, width, height);
    fs::create_dir_all(&dir)?;
    let mut lib = load_manifest(&dir);
    let file = format!("dark-{exposure_us}-{}.png", (gain * 1000.0).round() as i64);
    fs::write(dir.join(&file), encode_png(img)?)?;
    lib.entries.retain(|e| e.file != file);
    lib.entries.push(DarkEntry {
        exposure_us,
        gain,
        file,
        captured_at,
    });
    save_manifest(&dir, &lib)
}

/// Deletes the entire library for a (driver, width, height) combination.
/// Not an error if it doesn't exist.
pub fn clear_library(
    data_dir: &Path,
    driver: CameraDriver,
    width: u32,
    height: u32,
) -> anyhow::Result<()> {
    let dir = library_dir(data_dir, driver, width, height);
    match fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;
    use tempfile::TempDir;

    fn settings(enabled: bool) -> DarkFrameSettings {
        DarkFrameSettings {
            enabled,
            min_gain_to_apply: 15.0,
            min_exposure_us_to_apply: 10_000_000,
        }
    }

    #[test]
    fn appliable_targets_keeps_only_points_the_apply_gate_can_use() {
        // The real deployed scenario: ASI gain range 1..100, apply thresholds
        // gain>=15 / exp>=10s, configured exposure ceiling 20s. All five
        // sweep exposures cap/collapse onto 20s, gain 1 drops out — leaving
        // exactly the two darks that can ever be subtracted, including the
        // (20s, max gain) one the whole feature exists for.
        let settings = DarkFrameSettings {
            enabled: true,
            min_gain_to_apply: 15.0,
            min_exposure_us_to_apply: 10_000_000,
        };
        let got = appliable_targets(sweep_targets(1.0, 100.0), &settings, 20_000_000);
        assert_eq!(got, vec![(20_000_000, 50.5), (20_000_000, 100.0)]);
    }

    #[test]
    fn appliable_targets_is_empty_when_thresholds_exclude_the_whole_grid() {
        let settings = DarkFrameSettings {
            enabled: true,
            min_gain_to_apply: 500.0, // above every swept gain
            min_exposure_us_to_apply: 10_000_000,
        };
        assert!(appliable_targets(sweep_targets(1.0, 100.0), &settings, 20_000_000).is_empty());
    }

    #[test]
    fn stack_count_fits_the_time_budget_and_clamps_to_3_10() {
        assert_eq!(stack_count(500_000), 10); // 0.5s — budget allows far more, capped
        assert_eq!(stack_count(20_000_000), 10); // 20s → 200/20, right at the cap
        assert_eq!(stack_count(60_000_000), 3); // 60s → 3 by budget, also the floor
    }

    #[test]
    fn repair_replaces_hot_pixels_with_neighbor_median_even_when_blinking_brighter() {
        // Master dark: flat pedestal 30 with one hot pixel at (2,2).
        let mut dark = RgbImage::from_pixel(5, 5, Rgb([30, 30, 30]));
        dark.put_pixel(2, 2, Rgb([120, 120, 120]));

        // Light frame: uniform sky 80, but the defect is currently blinking
        // FULL brightness (250) — much brighter than the master's 120, so
        // subtraction alone would leave a bright 130 residual.
        let mut light = RgbImage::from_pixel(5, 5, Rgb([80, 80, 80]));
        light.put_pixel(2, 2, Rgb([250, 250, 250]));

        subtract_dark(&mut light, &dark);
        repair_hot_pixels(&mut light, &dark);

        // Non-hot pixels: plain subtraction (80 - 30).
        assert_eq!(light.get_pixel(0, 0).0, [50, 50, 50]);
        // The hot pixel matches its neighbors instead of showing a residual.
        assert_eq!(light.get_pixel(2, 2).0, [50, 50, 50]);
    }

    #[test]
    fn repair_leaves_frames_alone_when_the_dark_has_no_hot_pixels() {
        let dark = RgbImage::from_pixel(4, 4, Rgb([35, 35, 35])); // pure pedestal
        let mut light = RgbImage::from_pixel(4, 4, Rgb([90, 90, 90]));
        let before = light.clone();
        repair_hot_pixels(&mut light, &dark);
        assert_eq!(light, before);
    }

    #[test]
    fn repair_skips_pixels_whose_whole_neighborhood_is_hot() {
        // Every dark pixel is far above its own median? No — median adapts.
        // Make a 3x3 fully-hot cluster inside a larger pedestal so the
        // cluster's center has no clean neighbor to borrow from.
        let mut dark = RgbImage::from_pixel(7, 7, Rgb([20, 20, 20]));
        for y in 2..5 {
            for x in 2..5 {
                dark.put_pixel(x, y, Rgb([200, 200, 200]));
            }
        }
        let mut light = RgbImage::from_pixel(7, 7, Rgb([220, 220, 220]));
        subtract_dark(&mut light, &dark);
        let center_after_subtract = light.get_pixel(3, 3).0;
        repair_hot_pixels(&mut light, &dark);
        // Edge-of-cluster pixels have clean neighbors and get repaired to
        // the surrounding value; the center has none and stays subtracted.
        assert_eq!(light.get_pixel(2, 2).0, [200, 200, 200]); // 220-20 neighbors
        assert_eq!(light.get_pixel(3, 3).0, center_after_subtract);
    }

    #[test]
    fn stack_master_rejects_a_single_outlier_sample() {
        // Nine frames agree on 10; one carries a cosmic-ray-style 255 spike.
        // The min/max clip must drop the spike (and one 10 as the min),
        // leaving the master at exactly 10 — a plain mean would read ~34.
        let mut frames = vec![RgbImage::from_pixel(4, 3, Rgb([10, 10, 10])); 9];
        frames.push(RgbImage::from_pixel(4, 3, Rgb([255, 255, 255])));
        let master = stack_master(&frames).unwrap();
        assert!(master.pixels().all(|p| p.0 == [10, 10, 10]));
    }

    #[test]
    fn stack_master_of_three_is_the_median() {
        let frames = vec![
            RgbImage::from_pixel(2, 2, Rgb([5, 5, 5])),
            RgbImage::from_pixel(2, 2, Rgb([20, 20, 20])),
            RgbImage::from_pixel(2, 2, Rgb([200, 200, 200])),
        ];
        let master = stack_master(&frames).unwrap();
        assert!(master.pixels().all(|p| p.0 == [20, 20, 20]));
    }

    #[test]
    fn stack_master_of_fewer_than_three_falls_back_to_the_mean() {
        let frames = vec![
            RgbImage::from_pixel(2, 2, Rgb([10, 10, 10])),
            RgbImage::from_pixel(2, 2, Rgb([20, 20, 20])),
        ];
        let master = stack_master(&frames).unwrap();
        assert!(master.pixels().all(|p| p.0 == [15, 15, 15]));
    }

    #[test]
    fn stack_master_rejects_empty_or_mismatched_stacks() {
        assert!(stack_master(&[]).is_none());
        let mismatched = vec![
            RgbImage::from_pixel(2, 2, Rgb([10, 10, 10])),
            RgbImage::from_pixel(3, 2, Rgb([10, 10, 10])),
            RgbImage::from_pixel(2, 2, Rgb([10, 10, 10])),
        ];
        assert!(stack_master(&mismatched).is_none());
    }

    #[test]
    fn sweep_targets_covers_five_exposures_and_three_gains() {
        let targets = sweep_targets(1.0, 16.0);
        assert_eq!(targets.len(), 15);
        assert!(targets.contains(&(500_000, 1.0)));
        assert!(targets.contains(&(500_000, 8.5))); // midpoint
        assert!(targets.contains(&(500_000, 16.0)));
        assert!(targets.contains(&(60_000_000, 16.0)));
    }

    #[test]
    fn nearest_match_prefers_closest_relative_distance() {
        let entries = vec![
            DarkEntry {
                exposure_us: 500_000,
                gain: 1.0,
                file: "a.png".into(),
                captured_at: Utc::now(),
            },
            DarkEntry {
                exposure_us: 60_000_000,
                gain: 16.0,
                file: "b.png".into(),
                captured_at: Utc::now(),
            },
        ];
        let m = nearest_match(&entries, 55_000_000, 15.0).unwrap();
        assert_eq!(m.file, "b.png");
    }

    #[test]
    fn nearest_match_does_not_let_exposure_scale_swamp_gain() {
        // A dark at the light frame's exact exposure but very different
        // gain must lose to one at a slightly-off exposure but matching
        // gain — proving the distance is relative per-axis, not a raw sum
        // (which would be dominated by microsecond-scale exposure diffs).
        let entries = vec![
            DarkEntry {
                exposure_us: 20_000_000,
                gain: 1.0,
                file: "wrong-gain.png".into(),
                captured_at: Utc::now(),
            },
            DarkEntry {
                exposure_us: 8_000_000,
                gain: 16.0,
                file: "right-gain.png".into(),
                captured_at: Utc::now(),
            },
        ];
        let m = nearest_match(&entries, 20_000_000, 16.0).unwrap();
        assert_eq!(m.file, "right-gain.png");
    }

    #[test]
    fn nearest_match_is_none_for_an_empty_library() {
        assert!(nearest_match(&[], 1_000_000, 8.0).is_none());
    }

    #[test]
    fn subtract_dark_saturates_at_zero_and_skips_size_mismatch() {
        let mut img = RgbImage::from_pixel(4, 4, Rgb([10, 5, 200]));
        let dark = RgbImage::from_pixel(4, 4, Rgb([50, 2, 100]));
        subtract_dark(&mut img, &dark);
        assert_eq!(img.get_pixel(0, 0).0, [0, 3, 100]); // 10-50 clamps to 0

        let mut img2 = RgbImage::from_pixel(4, 4, Rgb([10, 10, 10]));
        let wrong_size = RgbImage::from_pixel(2, 2, Rgb([5, 5, 5]));
        subtract_dark(&mut img2, &wrong_size); // size mismatch -> no-op
        assert_eq!(img2.get_pixel(0, 0).0, [10, 10, 10]);
    }

    #[test]
    fn should_apply_gates_on_enabled_and_both_thresholds() {
        let s = settings(true);
        assert!(should_apply(&s, 20_000_000, 16.0)); // above both
        assert!(!should_apply(&s, 20_000_000, 15.0)); // gain not strictly above
        assert!(!should_apply(&s, 10_000_000, 16.0)); // exposure not strictly above
        assert!(!should_apply(&settings(false), 60_000_000, 100.0)); // disabled
    }

    #[test]
    fn manifest_round_trips_through_disk() {
        let dir = TempDir::new().unwrap();
        let lib = DarksLibrary {
            entries: vec![DarkEntry {
                exposure_us: 2_000_000,
                gain: 8.0,
                file: "dark-2000000-8000.png".into(),
                captured_at: Utc::now(),
            }],
        };
        save_manifest(dir.path(), &lib).unwrap();
        let loaded = load_manifest(dir.path());
        assert_eq!(loaded, lib);
    }

    #[test]
    fn load_manifest_is_empty_when_no_sweep_has_run() {
        let dir = TempDir::new().unwrap();
        let loaded = load_manifest(&dir.path().join("nonexistent"));
        assert!(loaded.entries.is_empty());
    }

    #[test]
    fn add_entry_writes_a_png_and_updates_the_manifest() {
        let dir = TempDir::new().unwrap();
        let img = RgbImage::from_pixel(8, 8, Rgb([3, 4, 5]));
        add_entry(
            dir.path(),
            CameraDriver::Mock,
            1280,
            960,
            2_000_000,
            8.0,
            &img,
            Utc::now(),
        )
        .unwrap();
        let lib_dir = library_dir(dir.path(), CameraDriver::Mock, 1280, 960);
        let lib = load_manifest(&lib_dir);
        assert_eq!(lib.entries.len(), 1);
        assert_eq!(lib.entries[0].exposure_us, 2_000_000);
        assert!(lib_dir.join(&lib.entries[0].file).exists());
    }

    #[test]
    fn add_entry_replaces_rather_than_duplicates_the_same_exposure_and_gain() {
        let dir = TempDir::new().unwrap();
        let img = RgbImage::from_pixel(4, 4, Rgb([1, 1, 1]));
        add_entry(
            dir.path(),
            CameraDriver::Mock,
            100,
            100,
            1_000_000,
            4.0,
            &img,
            Utc::now(),
        )
        .unwrap();
        add_entry(
            dir.path(),
            CameraDriver::Mock,
            100,
            100,
            1_000_000,
            4.0,
            &img,
            Utc::now(),
        )
        .unwrap();
        let lib = load_manifest(&library_dir(dir.path(), CameraDriver::Mock, 100, 100));
        assert_eq!(lib.entries.len(), 1);
    }

    #[test]
    fn clear_library_removes_the_directory_and_is_a_noop_when_absent() {
        let dir = TempDir::new().unwrap();
        let img = RgbImage::from_pixel(4, 4, Rgb([1, 1, 1]));
        add_entry(
            dir.path(),
            CameraDriver::Mock,
            50,
            50,
            1_000_000,
            4.0,
            &img,
            Utc::now(),
        )
        .unwrap();
        let lib_dir = library_dir(dir.path(), CameraDriver::Mock, 50, 50);
        assert!(lib_dir.exists());
        clear_library(dir.path(), CameraDriver::Mock, 50, 50).unwrap();
        assert!(!lib_dir.exists());
        clear_library(dir.path(), CameraDriver::Mock, 50, 50).unwrap(); // no error second time
    }

    #[test]
    fn apply_if_available_subtracts_the_matching_dark_when_gated_open() {
        let dir = TempDir::new().unwrap();
        let dark = RgbImage::from_pixel(6, 6, Rgb([40, 40, 40]));
        add_entry(
            dir.path(),
            CameraDriver::Asi,
            6,
            6,
            20_000_000,
            16.0,
            &dark,
            Utc::now(),
        )
        .unwrap();
        let s = settings(true);

        let mut light = RgbImage::from_pixel(6, 6, Rgb([100, 100, 100]));
        apply_if_available(
            &mut light,
            dir.path(),
            CameraDriver::Asi,
            6,
            6,
            &s,
            20_000_000,
            16.0,
        );
        assert_eq!(light.get_pixel(0, 0).0, [60, 60, 60]); // 100 - 40

        // Below threshold: gate closed, frame untouched even though a match exists.
        let mut untouched = RgbImage::from_pixel(6, 6, Rgb([100, 100, 100]));
        apply_if_available(
            &mut untouched,
            dir.path(),
            CameraDriver::Asi,
            6,
            6,
            &s,
            1_000_000,
            1.0,
        );
        assert_eq!(untouched.get_pixel(0, 0).0, [100, 100, 100]);
    }

    #[test]
    fn apply_if_available_is_a_noop_disabled_or_without_a_library() {
        let dir = TempDir::new().unwrap();
        let mut img = RgbImage::from_pixel(4, 4, Rgb([9, 9, 9]));
        // No library at all for this (driver, width, height).
        apply_if_available(
            &mut img,
            dir.path(),
            CameraDriver::Rpicam,
            4,
            4,
            &settings(true),
            20_000_000,
            16.0,
        );
        assert_eq!(img.get_pixel(0, 0).0, [9, 9, 9]);
    }
}
