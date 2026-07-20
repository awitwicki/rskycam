use chrono::{DateTime, Utc};

use crate::settings::LensCalibration;

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

pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// Equidistant fisheye projection into source-image pixels.
pub fn alt_az_to_image(alt_deg: f64, az_deg: f64, cal: &LensCalibration) -> Point {
    let r = cal.radius_px * (90.0 - alt_deg) / 90.0;
    let theta = (az_deg + cal.rotation_deg) * DEG;
    let sx = if cal.flip { -1.0 } else { 1.0 };
    Point {
        x: cal.cx + sx * r * theta.sin(),
        y: cal.cy - r * theta.cos(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        alt_az_to_image, altitude_of, gmst_deg, julian_date, moon_equatorial, moon_illumination,
        ra_dec_to_alt_az, sun_equatorial,
    };
    use chrono::TimeZone;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    fn cal() -> crate::settings::LensCalibration {
        crate::settings::LensCalibration {
            cx: 480.0,
            cy: 480.0,
            radius_px: 440.0,
            rotation_deg: 0.0,
            flip: false,
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
        let z = alt_az_to_image(90.0, 123.0, &cal());
        assert!((z.x - 480.0).abs() < 1e-6 && (z.y - 480.0).abs() < 1e-6);
        let n = alt_az_to_image(0.0, 0.0, &cal());
        assert!((n.x - 480.0).abs() < 1e-6 && (n.y - 40.0).abs() < 1e-6);
        let e = alt_az_to_image(0.0, 90.0, &cal());
        assert!((e.x - 920.0).abs() < 1e-6 && (e.y - 480.0).abs() < 1e-6);
    }

    #[test]
    fn rotation_and_flip_behave_like_the_ts_reference() {
        let mut c = cal();
        c.rotation_deg = 90.0;
        let n = alt_az_to_image(0.0, 0.0, &c);
        assert!((n.x - 920.0).abs() < 1e-6 && (n.y - 480.0).abs() < 1e-6);
        let mut f = cal();
        f.flip = true;
        let e = alt_az_to_image(0.0, 90.0, &f);
        assert!((e.x - 40.0).abs() < 1e-6 && (e.y - 480.0).abs() < 1e-6);
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
}
