use crate::settings::MeterPolygon;
use image::RgbImage;
use std::sync::atomic::{AtomicBool, Ordering};

/// What auto-exposure measured, and over how much of the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeterResult {
    pub mean: f64,
    pub area_pct: f64,
}

/// Is a point inside the polygon? Even-odd crossing number, with edges
/// half-open in y (`y0 <= p < y1`). Half-open is what makes the rule total:
/// a point level with a shared vertex is counted exactly once, so adjacent
/// polygons never double-count and never leave a seam between them.
fn contains(points: &[crate::settings::MeterPoint], px: f64, py: f64) -> bool {
    let n = points.len();
    let mut inside = false;
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        // When this differs, py lies in the half-open span of the edge — and
        // a.y != b.y is guaranteed, so the division below is safe.
        if (a.y <= py) != (b.y <= py) {
            let t = (py - a.y) / (b.y - a.y);
            if px < a.x + t * (b.x - a.x) {
                inside = !inside;
            }
        }
    }
    inside
}

/// A rasterised metering mask for one frame size.
///
/// `included` empty means "the whole frame" — both the no-polygons case and
/// the degenerate-polygons fallback. Nothing here ever converts a 0..1
/// fraction into a pixel index; it converts pixel centres into fractions, so
/// a coordinate of exactly 1.0 cannot produce an out-of-range index.
pub struct MeterMask {
    w: u32,
    h: u32,
    included: Vec<bool>,
    included_count: u64,
    // Set once `mean` has already warned about a frame-size mismatch, so a
    // cache bug that resurfaces the fallback logs once per mask instance
    // instead of once per frame.
    warned_mismatch: AtomicBool,
}

impl MeterMask {
    fn whole(w: u32, h: u32) -> MeterMask {
        MeterMask {
            w,
            h,
            included: Vec::new(),
            included_count: u64::from(w) * u64::from(h),
            warned_mismatch: AtomicBool::new(false),
        }
    }

    /// Test-only: production code never inspects coverage directly, it only
    /// consumes the `area_pct` that `mean` derives from it.
    #[allow(dead_code)]
    pub fn included_count(&self) -> u64 {
        self.included_count
    }

    pub fn build(polys: &[MeterPolygon], w: u32, h: u32) -> MeterMask {
        if polys.is_empty() || w == 0 || h == 0 {
            return MeterMask::whole(w, h);
        }
        let (wu, hu) = (w as usize, h as usize);
        let mut included = vec![false; wu * hu];
        let mut count: u64 = 0;
        for y in 0..h {
            let fy = (f64::from(y) + 0.5) / f64::from(h);
            for x in 0..w {
                let fx = (f64::from(x) + 0.5) / f64::from(w);
                if polys.iter().any(|p| contains(&p.points, fx, fy)) {
                    included[y as usize * wu + x as usize] = true;
                    count += 1;
                }
            }
        }
        if count == 0 {
            // Metering a mean of 0 would read the scene as pitch black and
            // rail auto-exposure to maximum exposure and gain until morning.
            tracing::warn!(
                "metering mask covers no pixels at {w}x{h} — metering the whole frame instead"
            );
            return MeterMask::whole(w, h);
        }
        MeterMask {
            w,
            h,
            included,
            included_count: count,
            warned_mismatch: AtomicBool::new(false),
        }
    }

    pub fn mean(&self, img: &RgbImage) -> MeterResult {
        let total = u64::from(img.width()) * u64::from(img.height());
        if total == 0 {
            return MeterResult {
                mean: 0.0,
                area_pct: 0.0,
            };
        }
        // A mask built for another frame size must never index this image.
        // MeterCache is what's supposed to make this unreachable — if it
        // fires, the cache has a bug, and silently metering the whole frame
        // forever would hide it. Warn, but only once per mask instance:
        // `mean` runs on every capture.
        if img.width() != self.w || img.height() != self.h {
            if !self.warned_mismatch.swap(true, Ordering::Relaxed) {
                tracing::warn!(
                    "metering mask built for {}x{} used on a {}x{} frame — metering the whole \
                     frame instead; this should be unreachable, the cache has a bug",
                    self.w,
                    self.h,
                    img.width(),
                    img.height()
                );
            }
            return MeterResult {
                mean: crate::camera::mean_brightness(img),
                area_pct: 100.0,
            };
        }
        if self.included.is_empty() {
            return MeterResult {
                mean: crate::camera::mean_brightness(img),
                area_pct: 100.0,
            };
        }
        let mut sum = 0.0;
        for (x, y, p) in img.enumerate_pixels() {
            if self.included[y as usize * self.w as usize + x as usize] {
                sum += 0.299 * f64::from(p.0[0])
                    + 0.587 * f64::from(p.0[1])
                    + 0.114 * f64::from(p.0[2]);
            }
        }
        MeterResult {
            mean: sum / self.included_count as f64,
            area_pct: self.included_count as f64 * 100.0 / total as f64,
        }
    }
}

/// Holds the rasterised mask between frames.
///
/// Rasterising ~1.2 M pixels costs real time on a Pi, and the polygons change
/// perhaps once a month — so the mask is built once and reused until either
/// the polygon list or the frame size changes.
pub struct MeterCache {
    mask: Option<MeterMask>,
    key: Option<(u32, u32, Vec<MeterPolygon>)>,
    builds: u32,
}

impl Default for MeterCache {
    fn default() -> Self {
        Self::new()
    }
}

impl MeterCache {
    pub fn new() -> MeterCache {
        MeterCache {
            mask: None,
            key: None,
            builds: 0,
        }
    }

    /// How many times the mask has been rasterised. Tests only.
    #[allow(dead_code)]
    pub fn builds(&self) -> u32 {
        self.builds
    }

    pub fn measure(&mut self, img: &RgbImage, polys: &[MeterPolygon]) -> MeterResult {
        let (w, h) = (img.width(), img.height());
        let stale = match &self.key {
            Some((kw, kh, kp)) => *kw != w || *kh != h || kp.as_slice() != polys,
            None => true,
        };
        if stale {
            self.mask = Some(MeterMask::build(polys, w, h));
            self.key = Some((w, h, polys.to_vec()));
            self.builds += 1;
        }
        self.mask.as_ref().expect("mask built above").mean(img)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{MeterPoint, MeterPolygon};
    use image::{Rgb, RgbImage};

    fn poly(pts: &[(f64, f64)]) -> MeterPolygon {
        MeterPolygon {
            points: pts.iter().map(|&(x, y)| MeterPoint { x, y }).collect(),
        }
    }

    /// Two-tone image: left half dark, right half bright.
    fn split_image(w: u32, h: u32, left: u8, right: u8) -> RgbImage {
        RgbImage::from_fn(w, h, |x, _| {
            let v = if x < w / 2 { left } else { right };
            Rgb([v, v, v])
        })
    }

    #[test]
    fn a_mask_over_one_half_meters_only_that_half() {
        let img = split_image(100, 100, 20, 200);
        // Right half only.
        let mask = MeterMask::build(
            &[poly(&[(0.5, 0.0), (1.0, 0.0), (1.0, 1.0), (0.5, 1.0)])],
            100,
            100,
        );
        let r = mask.mean(&img);
        assert!(
            (r.mean - 200.0).abs() < 1.0,
            "metered {} not the bright half",
            r.mean
        );
        assert!((r.area_pct - 50.0).abs() < 1.0, "area {}", r.area_pct);
    }

    #[test]
    fn an_empty_polygon_list_reproduces_the_old_whole_frame_mean() {
        // The guarantee for every existing install: with no mask configured,
        // auto-exposure must see the exact number it saw before this feature.
        let img = split_image(64, 48, 20, 200);
        let mask = MeterMask::build(&[], 64, 48);
        let r = mask.mean(&img);
        assert_eq!(r.mean, crate::camera::mean_brightness(&img));
        assert_eq!(r.area_pct, 100.0);
    }

    #[test]
    fn the_masked_path_weights_channels_like_mean_brightness() {
        // A genuinely coloured (non-grey) image: with R != G != B, any set
        // of weights that merely sums to 1.0 gives a different answer,
        // unlike a grey image where every weighting agrees — which is why
        // every other test in this file (all greyscale images) cannot pin
        // the weights. A full-frame polygon gives a non-empty `included`,
        // so `mean()` takes the masked summation branch (the one with the
        // literal 0.299/0.587/0.114 weights), not the whole-frame
        // delegation branch (empty `included`) that just calls
        // `mean_brightness` directly and so would match it trivially no
        // matter what the masked-branch weights were.
        let img = RgbImage::from_pixel(10, 10, Rgb([200, 100, 50]));
        let mask = MeterMask::build(
            &[poly(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)])],
            10,
            10,
        );
        assert_eq!(mask.mean(&img).mean, crate::camera::mean_brightness(&img));
    }

    #[test]
    fn concave_and_multi_region_masks_select_the_right_pixels() {
        // An L: the left band plus the bottom strip, so the top-right
        // quadrant falls OUTSIDE the region.
        let l_shape = poly(&[
            (0.0, 0.0),
            (0.4, 0.0),
            (0.4, 0.6),
            (1.0, 0.6),
            (1.0, 1.0),
            (0.0, 1.0),
        ]);
        let img = RgbImage::from_fn(100, 100, |x, y| {
            let v = if x >= 50 && y < 50 { 255 } else { 10 };
            Rgb([v, v, v])
        });
        let mask = MeterMask::build(&[l_shape], 100, 100);
        assert!(
            mask.mean(&img).mean < 20.0,
            "the bright top-right must be outside the L, got {}",
            mask.mean(&img).mean
        );

        // Two disjoint squares, 10% of the frame each.
        let two = MeterMask::build(
            &[
                poly(&[(0.0, 0.0), (0.2, 0.0), (0.2, 0.5), (0.0, 0.5)]),
                poly(&[(0.8, 0.5), (1.0, 0.5), (1.0, 1.0), (0.8, 1.0)]),
            ],
            100,
            100,
        );
        assert!((two.mean(&img).area_pct - 20.0).abs() < 1.0);
    }

    #[test]
    fn two_regions_sharing_an_edge_count_each_pixel_once() {
        // Half-open edges exist for exactly this: a seam must be neither
        // double-counted nor dropped, so two adjacent regions together cover
        // the frame exactly. 101 is odd, so pixel centre 50 —
        // (50 + 0.5) / 101 == 0.5 exactly — lands ON the seam; at an even
        // size no centre is ever exactly on a half/half split, so the
        // half-open rule's boundary condition is never actually exercised.
        let n: u32 = 101;
        let total = u64::from(n) * u64::from(n);

        // Vertical seam at x = 0.5.
        let left = poly(&[(0.0, 0.0), (0.5, 0.0), (0.5, 1.0), (0.0, 1.0)]);
        let right = poly(&[(0.5, 0.0), (1.0, 0.0), (1.0, 1.0), (0.5, 1.0)]);
        let vertical = MeterMask::build(&[left, right], n, n);
        assert_eq!(
            vertical.included_count(),
            total,
            "vertical seam lost or doubled"
        );
        assert!((vertical.mean(&split_image(n, n, 20, 200)).area_pct - 100.0).abs() < 0.01);

        // Horizontal seam at y = 0.5 — the orientation the half-open rule
        // (`(a.y <= py) != (b.y <= py)`) actually governs. The vertical seam
        // above only exercises horizontal (y = 0 / y = 1) frame edges, which
        // no pixel centre ever sits on; this is the case that does.
        let top = poly(&[(0.0, 0.0), (1.0, 0.0), (1.0, 0.5), (0.0, 0.5)]);
        let bottom = poly(&[(0.0, 0.5), (1.0, 0.5), (1.0, 1.0), (0.0, 1.0)]);
        let horizontal = MeterMask::build(&[top, bottom], n, n);
        assert_eq!(
            horizontal.included_count(),
            total,
            "horizontal seam lost or doubled"
        );
    }

    #[test]
    fn a_full_frame_polygon_includes_the_first_and_last_row_and_column() {
        // Coordinates of exactly 0.0 and 1.0 are what the editor produces when
        // a vertex is dragged to the frame edge and clamped. Because the
        // backend only ever maps pixel centres to fractions (never a fraction
        // to an index) this must cover every pixel — no dropped edge column,
        // no out-of-range index.
        let mask = MeterMask::build(
            &[poly(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)])],
            37, // deliberately not a round number
            23,
        );
        assert_eq!(mask.included_count(), 37 * 23);
        let img = RgbImage::from_pixel(37, 23, Rgb([120, 120, 120]));
        let r = mask.mean(&img);
        assert!((r.mean - 120.0).abs() < 0.6);
        assert!((r.area_pct - 100.0).abs() < 0.01);
    }

    #[test]
    fn a_self_intersecting_polygon_still_meters_a_finite_mean() {
        // One vertex dragged across the opposite edge makes a bowtie. Even-odd
        // gives it a hole; that is fine and expected. What must NOT happen is
        // zero coverage (which would silently switch metering back to the whole
        // frame) or a non-finite mean.
        let bowtie = poly(&[(0.1, 0.1), (0.9, 0.9), (0.9, 0.1), (0.1, 0.9)]);
        let mask = MeterMask::build(&[bowtie], 100, 100);
        assert!(mask.included_count() > 0, "bowtie collapsed to nothing");
        let r = mask.mean(&RgbImage::from_pixel(100, 100, Rgb([80, 80, 80])));
        assert!(r.mean.is_finite() && (r.mean - 80.0).abs() < 0.6);
        assert!(r.area_pct > 0.0 && r.area_pct < 100.0);
    }

    #[test]
    fn degenerate_polygons_fall_back_to_the_whole_frame() {
        let img = split_image(100, 100, 20, 200);
        let whole = crate::camera::mean_brightness(&img);

        // Three collinear points: passes the >=3-point check in sanitize, but
        // encloses zero area. Metering its "mean" would be 0 and would rail
        // auto-exposure to maximum exposure and gain until morning.
        let collinear = MeterMask::build(&[poly(&[(0.1, 0.5), (0.5, 0.5), (0.9, 0.5)])], 100, 100);
        assert_eq!(collinear.mean(&img).mean, whole);
        assert_eq!(collinear.mean(&img).area_pct, 100.0);

        // A region far too small to contain any pixel centre at this size.
        let tiny = MeterMask::build(
            &[poly(&[
                (0.5000, 0.5000),
                (0.5001, 0.5000),
                (0.5001, 0.5001),
            ])],
            100,
            100,
        );
        assert_eq!(tiny.mean(&img).mean, whole);
    }

    #[test]
    fn a_mask_over_the_sky_exposes_for_the_sky_not_the_sunlit_wall() {
        use crate::camera::CaptureParams;
        use crate::capture::auto_exposure::{self, ExposureLimits};

        const LIM: ExposureLimits = ExposureLimits {
            min_exposure_us: 32,
            max_exposure_us: 10_000_000,
            min_gain: 1.0,
            max_gain: 16.0,
        };
        const TARGET: f64 = 100.0;

        // Top half sky, bottom half a sunlit wall 20x brighter. Both respond
        // linearly to exposure x gain and clip at 255, like a real sensor.
        fn scene(p: CaptureParams) -> (RgbImage, f64) {
            let k = TARGET / (2_000_000.0 * 4.0);
            let light = p.exposure_us as f64 * p.gain;
            let sky = (light * k).min(255.0);
            let wall = (light * k * 20.0).min(255.0);
            let img = RgbImage::from_fn(64, 64, |_, y| {
                let v = if y < 32 { sky } else { wall } as u8;
                Rgb([v, v, v])
            });
            (img, sky)
        }

        fn settle(mask: &MeterMask) -> f64 {
            let mut cur = CaptureParams {
                exposure_us: 2_000_000,
                gain: 4.0,
            };
            let mut sky = 0.0;
            for _ in 0..40 {
                let (img, s) = scene(cur);
                sky = s;
                cur = auto_exposure::next_params(mask.mean(&img).mean, TARGET, cur, &LIM);
            }
            sky
        }

        // Unmasked: the wall dominates the average, so the loop stops the
        // whole frame down and leaves the sky badly underexposed.
        let unmasked_sky = settle(&MeterMask::build(&[], 64, 64));
        assert!(
            unmasked_sky < 40.0,
            "without a mask the sky should end up underexposed, got {unmasked_sky}"
        );

        // Masked to the sky half: the loop exposes for the sky itself.
        let sky_only = MeterMask::build(
            &[poly(&[(0.0, 0.0), (1.0, 0.0), (1.0, 0.5), (0.0, 0.5)])],
            64,
            64,
        );
        let masked_sky = settle(&sky_only);
        assert!(
            auto_exposure::converged(masked_sky, TARGET),
            "with a sky mask the sky should reach the target, got {masked_sky}"
        );
    }

    #[test]
    fn the_cache_rebuilds_only_when_the_mask_or_the_frame_size_changes() {
        let polys = vec![poly(&[(0.0, 0.0), (0.5, 0.0), (0.5, 1.0), (0.0, 1.0)])];
        let img = split_image(100, 100, 20, 200);
        let mut cache = MeterCache::new();

        cache.measure(&img, &polys);
        assert_eq!(cache.builds(), 1);
        cache.measure(&img, &polys);
        cache.measure(&img, &polys);
        assert_eq!(cache.builds(), 1, "nothing changed — must not rebuild");

        // A different polygon list rebuilds.
        let other = vec![poly(&[(0.5, 0.0), (1.0, 0.0), (1.0, 1.0), (0.5, 1.0)])];
        let r = cache.measure(&img, &other);
        assert_eq!(cache.builds(), 2);
        assert!(
            (r.mean - 200.0).abs() < 1.0,
            "rebuilt mask metered the wrong half"
        );
    }

    #[test]
    fn a_resolution_change_rebuilds_the_cache_for_the_new_size() {
        // Review Focus 5: the user changes captureWidth/Height in Settings
        // while a mask is cached. Fractions stay valid, but the raster does
        // not — it must be rebuilt, never indexed at the old size.
        let polys = vec![poly(&[(0.0, 0.0), (0.5, 0.0), (0.5, 1.0), (0.0, 1.0)])];
        let mut cache = MeterCache::new();

        let small = split_image(100, 100, 20, 200);
        assert!((cache.measure(&small, &polys).area_pct - 50.0).abs() < 1.0);
        assert_eq!(cache.builds(), 1);

        let large = split_image(320, 240, 20, 200);
        let r = cache.measure(&large, &polys);
        assert_eq!(cache.builds(), 2, "new frame size must rebuild");
        assert!(
            (r.area_pct - 50.0).abs() < 1.0,
            "fractions must survive the resize"
        );
        assert!((r.mean - 20.0).abs() < 1.0, "still the dark half");
    }
}
