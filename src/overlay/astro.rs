use chrono::{DateTime, Utc};

use crate::settings::{LensCalibration, LensType};

const DEG: f64 = std::f64::consts::PI / 180.0;

pub fn julian_date(t: DateTime<Utc>) -> f64 {
    t.timestamp_millis() as f64 / 86_400_000.0 + 2_440_587.5
}

/// Greenwich mean sidereal time in degrees, [0, 360).
pub fn gmst_deg(jd: f64) -> f64 {
    norm360(280.460_618_37 + 360.985_647_366_29 * (jd - 2_451_545.0))
}

/// Local sidereal time in degrees; east longitude positive.
pub fn lst_deg(t: DateTime<Utc>, lon_deg: f64) -> f64 {
    norm360(gmst_deg(julian_date(t)) + lon_deg)
}

fn norm360(x: f64) -> f64 {
    (x % 360.0 + 360.0) % 360.0
}

pub struct AltAz {
    pub alt_deg: f64,
    pub az_deg: f64,
}

/// Azimuth measured from north, clockwise (east = 90°).
pub fn ra_dec_to_alt_az(ra_deg: f64, dec_deg: f64, lat_deg: f64, lst_deg: f64) -> AltAz {
    let ha = (lst_deg - ra_deg) * DEG;
    let dec = dec_deg * DEG;
    let lat = lat_deg * DEG;
    let sin_alt = dec.sin() * lat.sin() + dec.cos() * lat.cos() * ha.cos();
    let alt = sin_alt.clamp(-1.0, 1.0).asin();
    let az =
        (-dec.cos() * ha.sin()).atan2(dec.sin() * lat.cos() - dec.cos() * lat.sin() * ha.cos());
    AltAz {
        alt_deg: alt / DEG,
        az_deg: norm360(az / DEG),
    }
}

fn obliquity_rad(n: f64) -> f64 {
    (23.439 - 0.000_000_4 * n) * DEG
}

/// Low-precision solar ecliptic longitude (±0.01°), n = days since J2000.
fn sun_ecliptic_lon_deg(n: f64) -> f64 {
    let l = 280.46 + 0.985_647_4 * n;
    let g = (357.528 + 0.985_600_3 * n) * DEG;
    norm360(l + 1.915 * g.sin() + 0.02 * (2.0 * g).sin())
}

/// Low-precision lunar ecliptic coordinates (~1° accuracy).
fn moon_ecliptic(n: f64) -> (f64, f64) {
    let l = 218.316 + 13.176_396 * n;
    let m = (134.963 + 13.064_993 * n) * DEG;
    let f = (93.272 + 13.229_35 * n) * DEG;
    (norm360(l + 6.289 * m.sin()), 5.128 * f.sin())
}

pub struct Equatorial {
    pub ra_deg: f64,
    pub dec_deg: f64,
}

fn ecliptic_to_equatorial(lon_deg: f64, lat_deg: f64, n: f64) -> Equatorial {
    let lam = lon_deg * DEG;
    let beta = lat_deg * DEG;
    let eps = obliquity_rad(n);
    let ra = (lam.sin() * eps.cos() - beta.tan() * eps.sin()).atan2(lam.cos()) / DEG;
    let dec = (beta.sin() * eps.cos() + beta.cos() * eps.sin() * lam.sin()).asin() / DEG;
    Equatorial {
        ra_deg: norm360(ra),
        dec_deg: dec,
    }
}

pub fn sun_equatorial(t: DateTime<Utc>) -> Equatorial {
    let n = julian_date(t) - 2_451_545.0;
    ecliptic_to_equatorial(sun_ecliptic_lon_deg(n), 0.0, n)
}

pub fn moon_equatorial(t: DateTime<Utc>) -> Equatorial {
    let n = julian_date(t) - 2_451_545.0;
    let (lon, lat) = moon_ecliptic(n);
    ecliptic_to_equatorial(lon, lat, n)
}

/// Altitude of a body with fixed equatorial coordinates at a given time/place.
pub fn altitude_of(t: DateTime<Utc>, ra_deg: f64, dec_deg: f64, lat_deg: f64, lon_deg: f64) -> f64 {
    ra_dec_to_alt_az(ra_deg, dec_deg, lat_deg, lst_deg(t, lon_deg)).alt_deg
}

pub struct MoonIllumination {
    pub pct: f64,
    pub waxing: bool,
}

/// Illuminated fraction of the Moon (0–100) and whether it is waxing.
pub fn moon_illumination(t: DateTime<Utc>) -> MoonIllumination {
    let n = julian_date(t) - 2_451_545.0;
    let elong = norm360(moon_ecliptic(n).0 - sun_ecliptic_lon_deg(n));
    MoonIllumination {
        pct: (1.0 - (elong * DEG).cos()) / 2.0 * 100.0,
        waxing: elong < 180.0,
    }
}

const SUN_RISE_SET_ALT_DEG: f64 = -0.833;
const ASTRO_TWILIGHT_ALT_DEG: f64 = -18.0;
const MOON_RISE_SET_ALT_DEG: f64 = 0.0;
const EVENT_STEP: chrono::Duration = chrono::Duration::minutes(10);

/// First crossing of `alt_fn(t)` through `threshold_deg` at or after `from`,
/// scanning in `step` increments up to (not including) `to`; `rising`
/// selects an ascending vs. descending crossing. Refined to ~1 minute via
/// bisection. `None` if no such crossing occurs in the window.
fn find_crossing<F: Fn(DateTime<Utc>) -> f64>(
    alt_fn: &F,
    threshold_deg: f64,
    rising: bool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    step: chrono::Duration,
) -> Option<DateTime<Utc>> {
    let mut t0 = from;
    let mut above0 = alt_fn(t0) >= threshold_deg;
    let mut t1 = t0 + step;
    while t1 <= to {
        let above1 = alt_fn(t1) >= threshold_deg;
        if above1 != above0 && above1 == rising {
            let (mut lo, mut hi, lo_above) = (t0, t1, above0);
            while hi - lo > chrono::Duration::minutes(1) {
                let mid = lo + (hi - lo) / 2;
                if (alt_fn(mid) >= threshold_deg) == lo_above {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            return Some(lo + (hi - lo) / 2);
        }
        t0 = t1;
        above0 = above1;
        t1 += step;
    }
    None
}

/// Time of `alt_fn`'s maximum in `[from, to]`, found at `step` resolution
/// then refined to 1-minute resolution around the coarse peak.
fn find_transit<F: Fn(DateTime<Utc>) -> f64>(
    alt_fn: &F,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    step: chrono::Duration,
) -> DateTime<Utc> {
    let (mut best_t, mut best_alt) = (from, alt_fn(from));
    let mut t = from + step;
    while t <= to {
        let a = alt_fn(t);
        if a > best_alt {
            (best_t, best_alt) = (t, a);
        }
        t += step;
    }
    let (lo, hi) = ((best_t - step).max(from), (best_t + step).min(to));
    let mut t = lo;
    let refine_step = chrono::Duration::minutes(1);
    while t <= hi {
        let a = alt_fn(t);
        if a > best_alt {
            (best_t, best_alt) = (t, a);
        }
        t += refine_step;
    }
    best_t
}

pub struct AstroEvents {
    pub sunrise: Option<DateTime<Utc>>,
    pub sunset: Option<DateTime<Utc>>,
    pub astro_dusk: Option<DateTime<Utc>>,
    pub astro_dawn: Option<DateTime<Utc>>,
    pub moonrise: Option<DateTime<Utc>>,
    pub moonset: Option<DateTime<Utc>>,
    pub moon_transit: DateTime<Utc>,
}

/// Sun/Moon rise, set, astronomical-twilight and moon-transit events within
/// `[from, to]`. Twilight fields are `None` when the sun's altitude never
/// reaches -18° in the window (e.g. midsummer at mid-to-high latitudes).
pub fn astro_events(
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    lat_deg: f64,
    lon_deg: f64,
) -> AstroEvents {
    let sun_alt = |t: DateTime<Utc>| {
        let s = sun_equatorial(t);
        altitude_of(t, s.ra_deg, s.dec_deg, lat_deg, lon_deg)
    };
    let moon_alt = |t: DateTime<Utc>| {
        let m = moon_equatorial(t);
        altitude_of(t, m.ra_deg, m.dec_deg, lat_deg, lon_deg)
    };
    AstroEvents {
        sunset: find_crossing(&sun_alt, SUN_RISE_SET_ALT_DEG, false, from, to, EVENT_STEP),
        astro_dusk: find_crossing(
            &sun_alt,
            ASTRO_TWILIGHT_ALT_DEG,
            false,
            from,
            to,
            EVENT_STEP,
        ),
        astro_dawn: find_crossing(&sun_alt, ASTRO_TWILIGHT_ALT_DEG, true, from, to, EVENT_STEP),
        sunrise: find_crossing(&sun_alt, SUN_RISE_SET_ALT_DEG, true, from, to, EVENT_STEP),
        moonset: find_crossing(
            &moon_alt,
            MOON_RISE_SET_ALT_DEG,
            false,
            from,
            to,
            EVENT_STEP,
        ),
        moonrise: find_crossing(&moon_alt, MOON_RISE_SET_ALT_DEG, true, from, to, EVENT_STEP),
        moon_transit: find_transit(&moon_alt, from, to, EVENT_STEP),
    }
}

/// Frame dimensions plus the camera's native sensor width, for plate scale.
#[derive(Clone, Copy, Debug)]
pub struct LensView {
    pub frame_width: u32,
    pub frame_height: u32,
    /// Native sensor width in px (`CameraInfo::max_width`); equal to
    /// `frame_width` when unknown (binning 1).
    pub native_width: u32,
}

/// Focal length in pixels at this frame's resolution.
pub fn focal_length_px(cal: &LensCalibration, view: &LensView) -> f64 {
    let binning = view.native_width as f64 / view.frame_width as f64;
    cal.focal_length_mm * 1000.0 / (cal.pixel_size_um * binning)
}

/// Furthest usable angle from the optical axis, per lens type. Fisheye keeps
/// the legacy cardinal labels at alt −8° (θ ≈ 98° when zenith-pointed);
/// rectilinear stops short of the 90° tan singularity.
pub fn theta_max_deg(lens: LensType) -> f64 {
    match lens {
        LensType::Fisheye => 120.0,
        LensType::Rectilinear => 85.0,
    }
}

pub fn optical_center(cal: &LensCalibration, view: &LensView) -> (f64, f64) {
    (
        view.frame_width as f64 / 2.0 + cal.center_offset_x_px,
        view.frame_height as f64 / 2.0 + cal.center_offset_y_px,
    )
}

pub struct Projected {
    pub x: f64,
    pub y: f64,
    /// Angle from the optical axis, for field-of-view culling.
    pub theta_deg: f64,
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Camera basis in ENU (x=east, y=north, z=up), built so that at pointing
/// alt 90 / az 0 / roll 0 the image is north-up east-right — exactly the
/// legacy zenith model — and +roll rotates the sky clockwise.
fn cam_basis(cal: &LensCalibration) -> ([f64; 3], [f64; 3], [f64; 3]) {
    let (saz, caz) = (cal.pointing_az_deg * DEG).sin_cos();
    let (salt, calt) = (cal.pointing_alt_deg * DEG).sin_cos();
    let fwd = [calt * saz, calt * caz, salt];
    let u0 = [salt * saz, salt * caz, -calt];
    let r0 = [caz, -saz, 0.0];
    let (sr, cr) = (cal.roll_deg * DEG).sin_cos();
    let right = [
        cr * r0[0] + sr * u0[0],
        cr * r0[1] + sr * u0[1],
        cr * r0[2] + sr * u0[2],
    ];
    let up = [
        cr * u0[0] - sr * r0[0],
        cr * u0[1] - sr * r0[1],
        cr * u0[2] - sr * r0[2],
    ];
    (fwd, right, up)
}

/// r = f·tan θ diverges at 90°; clamp so culled points still get finite pixels.
const RECTILINEAR_THETA_CLAMP_DEG: f64 = 89.5;

/// Pixel distance from the optical center at `theta_deg` from the optical
/// axis — the lens's radial mapping (fisheye r = f·θ, rectilinear r = f·tan θ).
pub fn theta_to_radius_px(cal: &LensCalibration, view: &LensView, theta_deg: f64) -> f64 {
    let f_px = focal_length_px(cal, view);
    let theta = theta_deg * DEG;
    match cal.lens_type {
        LensType::Fisheye => f_px * theta,
        LensType::Rectilinear => f_px * theta.min(RECTILINEAR_THETA_CLAMP_DEG * DEG).tan(),
    }
}

/// Physical lens projection into source-image pixels.
pub fn alt_az_to_image(
    alt_deg: f64,
    az_deg: f64,
    cal: &LensCalibration,
    view: &LensView,
) -> Projected {
    let (sa, ca) = (alt_deg * DEG).sin_cos();
    let (saz, caz) = (az_deg * DEG).sin_cos();
    let v = [ca * saz, ca * caz, sa];
    let (fwd, right, up) = cam_basis(cal);
    let theta = dot(fwd, v).clamp(-1.0, 1.0).acos();
    let r = theta_to_radius_px(cal, view, theta / DEG);
    let phi = dot(v, right).atan2(dot(v, up));
    let sx = if cal.flip { -1.0 } else { 1.0 };
    let (ocx, ocy) = optical_center(cal, view);
    Projected {
        x: ocx + sx * r * phi.sin(),
        y: ocy - r * phi.cos(),
        theta_deg: theta / DEG,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        alt_az_to_image, altitude_of, astro_events, gmst_deg, julian_date, moon_equatorial,
        moon_illumination, ra_dec_to_alt_az, sun_equatorial,
    };
    use chrono::TimeZone;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    fn cal() -> crate::settings::LensCalibration {
        crate::settings::LensCalibration {
            lens_type: crate::settings::LensType::Fisheye,
            focal_length_mm: 0.88 / std::f64::consts::PI, // fPx = 880/π → horizon at 440 px
            pixel_size_um: 1.0,
            pointing_az_deg: 0.0,
            pointing_alt_deg: 90.0,
            roll_deg: 0.0,
            flip: false,
            center_offset_x_px: 0.0,
            center_offset_y_px: 0.0,
        }
    }

    fn view() -> super::LensView {
        super::LensView {
            frame_width: 960,
            frame_height: 960,
            native_width: 960,
        }
    }

    #[test]
    fn jd_and_gmst_at_j2000() {
        let jd = julian_date(utc(2000, 1, 1, 12, 0));
        assert!((jd - 2_451_545.0).abs() < 1e-6);
        assert!((gmst_deg(jd) - 280.4606).abs() < 1e-3);
    }

    #[test]
    fn object_at_dec_eq_lat_on_meridian_is_at_zenith() {
        let aa = ra_dec_to_alt_az(120.0, 50.0, 50.0, 120.0);
        assert!((aa.alt_deg - 90.0).abs() < 1e-5);
    }

    #[test]
    fn celestial_pole_sits_at_alt_lat_az_zero() {
        for lst in [0.0, 90.0, 217.0] {
            let aa = ra_dec_to_alt_az(33.0, 90.0, 50.45, lst);
            assert!((aa.alt_deg - 50.45).abs() < 1e-4);
            assert!(aa.az_deg.min(360.0 - aa.az_deg) < 1e-4);
        }
    }

    #[test]
    fn zenith_projects_to_center_horizon_n_up_e_right() {
        // Legacy zenith vectors: the physical model must reduce exactly to
        // the old radiusPx model (radiusPx = fPx·π/2 = 440).
        let z = alt_az_to_image(90.0, 123.0, &cal(), &view());
        assert!((z.x - 480.0).abs() < 1e-6 && (z.y - 480.0).abs() < 1e-6);
        let n = alt_az_to_image(0.0, 0.0, &cal(), &view());
        assert!((n.x - 480.0).abs() < 1e-6 && (n.y - 40.0).abs() < 1e-6);
        assert!((n.theta_deg - 90.0).abs() < 1e-6);
        let e = alt_az_to_image(0.0, 90.0, &cal(), &view());
        assert!((e.x - 920.0).abs() < 1e-6 && (e.y - 480.0).abs() < 1e-6);
    }

    #[test]
    fn roll_and_flip_behave_like_the_ts_reference() {
        let mut c = cal();
        c.roll_deg = 90.0;
        let n = alt_az_to_image(0.0, 0.0, &c, &view());
        assert!((n.x - 920.0).abs() < 1e-6 && (n.y - 480.0).abs() < 1e-6);
        let mut f = cal();
        f.flip = true;
        let e = alt_az_to_image(0.0, 90.0, &f, &view());
        assert!((e.x - 40.0).abs() < 1e-6 && (e.y - 480.0).abs() < 1e-6);
    }

    #[test]
    fn tilted_pointing_puts_the_pointing_on_center_and_zenith_below() {
        // Camera tilted toward the south horizon at alt 45: the pointing
        // direction hits the optical center; the zenith (θ = 45°) lands
        // fPx·π/4 = 220 px straight below it (ρ = 0 convention).
        let mut c = cal();
        c.pointing_az_deg = 180.0;
        c.pointing_alt_deg = 45.0;
        let p = alt_az_to_image(45.0, 180.0, &c, &view());
        assert!((p.x - 480.0).abs() < 1e-6 && (p.y - 480.0).abs() < 1e-6);
        assert!(p.theta_deg < 1e-6);
        let z = alt_az_to_image(90.0, 0.0, &c, &view());
        assert!((z.x - 480.0).abs() < 1e-6 && (z.y - 700.0).abs() < 1e-6);
        assert!((z.theta_deg - 45.0).abs() < 1e-6);
    }

    #[test]
    fn rectilinear_projects_tan_theta() {
        let mut c = cal();
        c.lens_type = crate::settings::LensType::Rectilinear;
        // θ = 45° → r = fPx·tan 45 = fPx = 880/π
        let p = alt_az_to_image(45.0, 0.0, &c, &view());
        assert!((p.x - 480.0).abs() < 1e-6);
        assert!((p.y - (480.0 - 880.0 / std::f64::consts::PI)).abs() < 1e-6);
    }

    #[test]
    fn binning_halves_the_plate_scale() {
        // Same lens on a 2×-binned frame: native 960, frame 480 → fPx halves.
        let v = super::LensView {
            frame_width: 480,
            frame_height: 480,
            native_width: 960,
        };
        let n = alt_az_to_image(0.0, 0.0, &cal(), &v);
        assert!((n.x - 240.0).abs() < 1e-6 && (n.y - 20.0).abs() < 1e-6);
    }

    #[test]
    fn theta_to_radius_matches_the_lens_mapping() {
        // Fisheye: r = fPx·θ → 440 px at the 90° horizon, 220 px at 45°.
        let r90 = super::theta_to_radius_px(&cal(), &view(), 90.0);
        assert!((r90 - 440.0).abs() < 1e-6);
        let r45 = super::theta_to_radius_px(&cal(), &view(), 45.0);
        assert!((r45 - 220.0).abs() < 1e-6);
        // Rectilinear: r = fPx·tan θ → fPx at 45°; θ ≥ 90° stays finite (clamped).
        let mut c = cal();
        c.lens_type = crate::settings::LensType::Rectilinear;
        let t45 = super::theta_to_radius_px(&c, &view(), 45.0);
        assert!((t45 - 880.0 / std::f64::consts::PI).abs() < 1e-6);
        assert!(super::theta_to_radius_px(&c, &view(), 90.0).is_finite());
    }

    #[test]
    fn sun_declination_at_solstice_and_equinox() {
        let june = sun_equatorial(utc(2026, 6, 21, 12, 0));
        assert!((june.dec_deg - 23.4).abs() < 0.5);
        let march = sun_equatorial(utc(2026, 3, 20, 12, 0));
        assert!(march.dec_deg.abs() < 1.0);
    }

    #[test]
    fn kyiv_sun_high_at_noon_below_horizon_at_midnight() {
        let noon = utc(2026, 6, 21, 10, 0);
        let s1 = sun_equatorial(noon);
        assert!(altitude_of(noon, s1.ra_deg, s1.dec_deg, 50.45, 30.52) > 55.0);
        let midnight = utc(2026, 6, 21, 22, 0);
        let s2 = sun_equatorial(midnight);
        assert!(altitude_of(midnight, s2.ra_deg, s2.dec_deg, 50.45, 30.52) < -5.0);
    }

    #[test]
    fn moon_illumination_at_documented_lunations() {
        // documented lunations: new 2000-01-06 18:14 UTC, full 2000-01-21 04:40 UTC
        assert!(moon_illumination(utc(2000, 1, 6, 18, 14)).pct < 2.0);
        assert!(moon_illumination(utc(2000, 1, 21, 4, 40)).pct > 97.0);
        let mid = moon_illumination(utc(2000, 1, 14, 0, 0));
        assert!(mid.waxing);
        let m = moon_equatorial(utc(2000, 1, 14, 0, 0));
        assert!((0.0..360.0).contains(&m.ra_deg));
        assert!(m.dec_deg.abs() <= 29.0);
    }

    const KYIV_LAT: f64 = 50.45;
    const KYIV_LON: f64 = 30.52;

    #[test]
    fn astro_events_equinox_kyiv_all_events_present_and_ordered() {
        let from = utc(2026, 3, 20, 12, 0);
        let to = from + chrono::Duration::hours(24);
        let ev = astro_events(from, to, KYIV_LAT, KYIV_LON);
        let sunset = ev.sunset.expect("sunset");
        let dusk = ev.astro_dusk.expect("astro dusk");
        let dawn = ev.astro_dawn.expect("astro dawn");
        let sunrise = ev.sunrise.expect("sunrise");
        assert!(sunset < dusk && dusk < dawn && dawn < sunrise);

        let sun_alt = |t| {
            let s = sun_equatorial(t);
            altitude_of(t, s.ra_deg, s.dec_deg, KYIV_LAT, KYIV_LON)
        };
        assert!((sun_alt(sunset) - (-0.833)).abs() < 0.1);
        assert!((sun_alt(sunrise) - (-0.833)).abs() < 0.1);
        assert!((sun_alt(dusk) - (-18.0)).abs() < 0.1);
        assert!((sun_alt(dawn) - (-18.0)).abs() < 0.1);
    }

    #[test]
    fn astro_events_summer_solstice_kyiv_has_no_astronomical_night() {
        // At 50.45°N the sun's lower culmination near the June solstice
        // stays above -18° (φ+δ-90 ≈ -16°): true astronomical night never
        // begins.
        let from = utc(2026, 6, 21, 12, 0);
        let to = from + chrono::Duration::hours(24);
        let ev = astro_events(from, to, KYIV_LAT, KYIV_LON);
        assert!(ev.sunset.is_some());
        assert!(ev.sunrise.is_some());
        assert!(ev.astro_dusk.is_none());
        assert!(ev.astro_dawn.is_none());
    }

    #[test]
    fn astro_events_moon_transit_is_a_local_altitude_maximum() {
        let from = utc(2026, 3, 20, 12, 0);
        let to = from + chrono::Duration::hours(24);
        let ev = astro_events(from, to, KYIV_LAT, KYIV_LON);
        let moon_alt = |t| {
            let m = moon_equatorial(t);
            altitude_of(t, m.ra_deg, m.dec_deg, KYIV_LAT, KYIV_LON)
        };
        let at_transit = moon_alt(ev.moon_transit);
        let ten_min = chrono::Duration::minutes(10);
        assert!(at_transit >= moon_alt(ev.moon_transit - ten_min) - 1e-6);
        assert!(at_transit >= moon_alt(ev.moon_transit + ten_min) - 1e-6);
    }

    #[test]
    fn astro_events_moonrise_moonset_cross_the_horizon_when_present() {
        let from = utc(2026, 3, 20, 12, 0);
        let to = from + chrono::Duration::hours(24);
        let ev = astro_events(from, to, KYIV_LAT, KYIV_LON);
        let moon_alt = |t| {
            let m = moon_equatorial(t);
            altitude_of(t, m.ra_deg, m.dec_deg, KYIV_LAT, KYIV_LON)
        };
        if let Some(t) = ev.moonrise {
            assert!(moon_alt(t).abs() < 0.2);
        }
        if let Some(t) = ev.moonset {
            assert!(moon_alt(t).abs() < 0.2);
        }
    }
}
