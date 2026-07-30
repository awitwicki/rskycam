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
