use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::overlay::astro;
use crate::settings::{
    CropRect, ImageSettings, LensCalibration, LocationSettings, MaskMode, OverlayLayers,
    OverlayTextField, TextFieldKind,
};

/// Constellation stick-figure data — see frontend/src/lib/constellations.json
/// (the canonical copy; frontend/src/lib/constellations.NOTICE.md has the
/// license/attribution) for provenance. Embedded here so both sides render
/// from byte-identical data.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConstellationDef {
    name: String,
    label_ra_deg: f64,
    label_dec_deg: f64,
    lines: Vec<Vec<[f64; 2]>>,
}

#[derive(Deserialize)]
struct ConstellationsFile {
    constellations: Vec<ConstellationDef>,
}

const CONSTELLATIONS_JSON: &str = include_str!("../../frontend/src/lib/constellations.json");

fn constellations() -> &'static [ConstellationDef] {
    static DATA: OnceLock<Vec<ConstellationDef>> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str::<ConstellationsFile>(CONSTELLATIONS_JSON)
            .expect("embedded constellations.json is valid")
            .constellations
    })
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayPolyline {
    pub layer: String,
    pub points: Vec<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayLabel {
    pub layer: String,
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub font_size: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayGeometry {
    pub image_width: u32,
    pub image_height: u32,
    pub polylines: Vec<OverlayPolyline>,
    pub labels: Vec<OverlayLabel>,
}

/// Manual fisheye mask circle in sensor-frame pixels.
#[derive(Clone, Copy, Debug)]
pub struct MaskCircle {
    pub center_x_px: f64,
    pub center_y_px: f64,
    pub radius_px: f64,
}

impl MaskCircle {
    /// The manual mask circle, when the image settings enable one.
    pub fn from_image(image: &ImageSettings) -> Option<Self> {
        (image.mask_mode == MaskMode::Circle).then_some(Self {
            center_x_px: image.mask_center_x_px,
            center_y_px: image.mask_center_y_px,
            radius_px: image.mask_radius_px,
        })
    }
}

pub struct BuildOptions<'a> {
    pub time: DateTime<Utc>,
    pub location: &'a LocationSettings,
    pub calibration: &'a LensCalibration,
    pub layers: &'a OverlayLayers,
    pub grid_opacity: Option<f64>,
    pub constellations_opacity: Option<f64>,
    pub image_width: u32,
    pub image_height: u32,
    /// Native sensor width for plate scale; == image_width when unknown.
    pub native_width: u32,
    /// When set, grid lines are culled outside this circle: the mask covers
    /// the grid, never the other way around. Labels are left alone.
    pub mask: Option<MaskCircle>,
}

const MIN_ALT_RADEC: f64 = 0.0;

/// Split a sampled line into segments inside the usable field of view:
/// θ ≤ theta_max, (when min_alt is set) above the horizon, and (when a mask
/// circle is set) inside the mask.
fn visible_segments(
    samples: &[(f64, f64, f64, f64)],
    min_alt: Option<f64>,
    theta_max: f64,
    mask: Option<MaskCircle>,
) -> Vec<Vec<[f64; 2]>> {
    let mut segs = Vec::new();
    let mut cur: Vec<[f64; 2]> = Vec::new();
    let in_mask = |x: f64, y: f64| {
        mask.is_none_or(|m| {
            (x - m.center_x_px).powi(2) + (y - m.center_y_px).powi(2) <= m.radius_px.powi(2)
        })
    };
    for &(alt, theta, x, y) in samples {
        if min_alt.is_none_or(|m| alt >= m) && theta <= theta_max && in_mask(x, y) {
            cur.push([x, y]);
        } else {
            if cur.len() > 1 {
                segs.push(std::mem::take(&mut cur));
            }
            cur.clear();
        }
    }
    if cur.len() > 1 {
        segs.push(cur);
    }
    segs
}

pub fn build_overlay_geometry(o: &BuildOptions) -> OverlayGeometry {
    let cal = o.calibration;
    let view = astro::LensView {
        frame_width: o.image_width,
        frame_height: o.image_height,
        native_width: o.native_width,
    };
    let theta_max = astro::theta_max_deg(cal.lens_type);
    let mut polylines = Vec::new();
    let mut labels = Vec::new();
    let opacity = o.grid_opacity;
    let lst = astro::lst_deg(o.time, o.location.longitude_deg);
    let lat = o.location.latitude_deg;
    let sample = |ra: f64, dec: f64| -> (f64, f64, f64, f64) {
        let aa = astro::ra_dec_to_alt_az(ra, dec, lat, lst);
        let p = astro::alt_az_to_image(aa.alt_deg, aa.az_deg, cal, &view);
        (aa.alt_deg, p.theta_deg, p.x, p.y)
    };

    if o.layers.alt_az_grid {
        for alt in [0.0f64, 30.0, 60.0] {
            let mut samples = Vec::new();
            let mut az = 0.0f64;
            while az <= 360.0 {
                let p = astro::alt_az_to_image(alt, az, cal, &view);
                samples.push((alt, p.theta_deg, p.x, p.y));
                az += 5.0;
            }
            for points in visible_segments(&samples, None, theta_max, o.mask) {
                polylines.push(OverlayPolyline {
                    layer: "altAz".into(),
                    points,
                    opacity,
                });
            }
        }
        let mut az = 0.0f64;
        while az < 360.0 {
            let mut samples = Vec::new();
            let mut alt = 0.0f64;
            while alt <= 80.0 {
                let p = astro::alt_az_to_image(alt, az, cal, &view);
                samples.push((alt, p.theta_deg, p.x, p.y));
                alt += 5.0;
            }
            for points in visible_segments(&samples, None, theta_max, o.mask) {
                polylines.push(OverlayPolyline {
                    layer: "altAz".into(),
                    points,
                    opacity,
                });
            }
            az += 45.0;
        }
    }

    if o.layers.cardinal {
        for (text, az) in [("N", 0.0), ("E", 90.0), ("S", 180.0), ("W", 270.0)] {
            let p = astro::alt_az_to_image(-8.0, az, cal, &view); // a bit outside the horizon circle
            if p.theta_deg > theta_max {
                continue;
            }
            labels.push(OverlayLabel {
                layer: "cardinal".into(),
                text: text.into(),
                x: p.x,
                y: p.y,
                font_size: 28.0,
                align: None,
            });
        }
    }

    if o.layers.ra_dec_grid {
        // ±80 keeps a small circle around each celestial pole (no hole).
        for dec in [-80.0f64, -60.0, -30.0, 0.0, 30.0, 60.0, 80.0] {
            let mut samples = Vec::new();
            let mut ra = 0.0f64;
            while ra <= 360.0 {
                samples.push(sample(ra, dec));
                ra += 3.0;
            }
            for points in visible_segments(&samples, Some(MIN_ALT_RADEC), theta_max, o.mask) {
                polylines.push(OverlayPolyline {
                    layer: "raDec".into(),
                    points,
                    opacity,
                });
            }
        }
        // Meridians run to dec ±90 so they converge exactly at the poles.
        let mut ra = 0.0f64;
        while ra < 360.0 {
            let mut samples = Vec::new();
            let mut dec = -90.0f64;
            while dec <= 90.0 {
                samples.push(sample(ra, dec));
                dec += 3.0;
            }
            for points in visible_segments(&samples, Some(MIN_ALT_RADEC), theta_max, o.mask) {
                polylines.push(OverlayPolyline {
                    layer: "raDec".into(),
                    points,
                    opacity,
                });
            }
            ra += 30.0;
        }
    }

    if o.layers.constellations {
        for c in constellations() {
            for line in &c.lines {
                let samples: Vec<(f64, f64, f64, f64)> =
                    line.iter().map(|p| sample(p[0], p[1])).collect();
                for points in visible_segments(&samples, Some(MIN_ALT_RADEC), theta_max, o.mask) {
                    polylines.push(OverlayPolyline {
                        layer: "constellations".into(),
                        points,
                        opacity: o.constellations_opacity,
                    });
                }
            }
            let (label_alt, label_theta, label_x, label_y) =
                sample(c.label_ra_deg, c.label_dec_deg);
            if label_alt >= MIN_ALT_RADEC && label_theta <= theta_max {
                labels.push(OverlayLabel {
                    layer: "constellationLabels".into(),
                    text: c.name.clone(),
                    x: label_x,
                    y: label_y,
                    font_size: 13.0,
                    align: None,
                });
            }
        }
    }

    OverlayGeometry {
        image_width: o.image_width,
        image_height: o.image_height,
        polylines,
        labels,
    }
}

/// Shift sensor-space geometry into cropped-image coordinates.
pub fn crop_geometry(g: OverlayGeometry, crop: &CropRect) -> OverlayGeometry {
    OverlayGeometry {
        image_width: crop.width.round() as u32,
        image_height: crop.height.round() as u32,
        polylines: g
            .polylines
            .into_iter()
            .map(|pl| OverlayPolyline {
                points: pl
                    .points
                    .iter()
                    .map(|p| [p[0] - crop.x, p[1] - crop.y])
                    .collect(),
                ..pl
            })
            .collect(),
        labels: g
            .labels
            .into_iter()
            .map(|l| OverlayLabel {
                x: l.x - crop.x,
                y: l.y - crop.y,
                ..l
            })
            .collect(),
    }
}

pub struct TextContext {
    pub local_time: String,
    pub exposure_us: Option<u64>,
    pub gain: Option<f64>,
    pub sensor_temp_c: Option<f64>,
}

pub fn format_exposure(us: u64) -> String {
    let s = us as f64 / 1e6;
    if s >= 1.0 {
        if s % 1.0 == 0.0 {
            format!("{} s", s as u64)
        } else {
            format!("{:.1} s", s)
        }
    } else {
        let trimmed = format!("{s:.6}");
        let trimmed = trimmed.trim_end_matches('0');
        let trimmed = trimmed.trim_end_matches('.');
        format!("{trimmed} s")
    }
}

pub fn append_text_fields(g: &mut OverlayGeometry, fields: &[OverlayTextField], ctx: &TextContext) {
    for f in fields {
        let text = match f.kind {
            TextFieldKind::Time => ctx.local_time.clone(),
            TextFieldKind::Exposure => match (ctx.exposure_us, ctx.gain) {
                (Some(us), Some(gain)) => {
                    format!("exp {} · gain {gain:.2}", format_exposure(us))
                }
                _ => "exp — · gain —".into(),
            },
            TextFieldKind::SensorTemp => match ctx.sensor_temp_c {
                Some(t) => format!("{t:.1}°C"),
                None => "—°C".into(),
            },
        };
        g.labels.push(OverlayLabel {
            layer: "text".into(),
            text,
            x: f.x,
            y: f.y,
            font_size: f.font_size,
            align: Some("left".into()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::astro;
    use crate::settings::{CropRect, LensCalibration, LocationSettings, OverlayLayers};
    use chrono::TimeZone;

    fn base() -> (
        chrono::DateTime<chrono::Utc>,
        LocationSettings,
        LensCalibration,
    ) {
        (
            chrono::Utc.with_ymd_and_hms(2026, 7, 14, 0, 0, 0).unwrap(),
            LocationSettings {
                latitude_deg: 50.45,
                longitude_deg: 30.52,
            },
            LensCalibration {
                lens_type: crate::settings::LensType::Fisheye,
                focal_length_mm: 0.88 / std::f64::consts::PI,
                pixel_size_um: 1.0,
                pointing_az_deg: 0.0,
                pointing_alt_deg: 90.0,
                roll_deg: 0.0,
                flip: false,
                center_offset_x_px: 0.0,
                center_offset_y_px: 0.0,
            },
        )
    }

    fn build(layers: OverlayLayers, grid_opacity: Option<f64>) -> OverlayGeometry {
        let (time, loc, cal) = base();
        build_overlay_geometry(&BuildOptions {
            time,
            location: &loc,
            calibration: &cal,
            layers: &layers,
            grid_opacity,
            constellations_opacity: None,
            image_width: 960,
            image_height: 960,
            native_width: 960,
            mask: None,
        })
    }

    const NONE: OverlayLayers = OverlayLayers {
        cardinal: false,
        alt_az_grid: false,
        ra_dec_grid: false,
        constellations: false,
    };

    #[test]
    fn empty_when_all_layers_off() {
        let g = build(NONE, None);
        assert!(g.polylines.is_empty() && g.labels.is_empty());
        assert_eq!(g.image_width, 960);
    }

    #[test]
    fn alt_az_grid_has_3_circles_and_8_radials() {
        let g = build(
            OverlayLayers {
                alt_az_grid: true,
                ..NONE
            },
            None,
        );
        assert_eq!(g.polylines.len(), 11);
        assert!(g.polylines.iter().all(|p| p.layer == "altAz"));
        // horizon circle points sit at radius_px from center
        for p in &g.polylines[0].points {
            let r = ((p[0] - 480.0).powi(2) + (p[1] - 480.0).powi(2)).sqrt();
            assert!((r - 440.0).abs() < 1e-6);
        }
    }

    #[test]
    fn cardinal_labels_n_above_center() {
        let g = build(
            OverlayLayers {
                cardinal: true,
                ..NONE
            },
            None,
        );
        let mut texts: Vec<_> = g.labels.iter().map(|l| l.text.as_str()).collect();
        texts.sort_unstable();
        assert_eq!(texts, ["E", "N", "S", "W"]);
        let n = g.labels.iter().find(|l| l.text == "N").unwrap();
        assert!(n.y < 480.0 && (n.x - 480.0).abs() < 1e-6);
    }

    #[test]
    fn ra_dec_meridians_converge_at_the_pole_with_a_dec80_ring() {
        let g = build(
            OverlayLayers {
                ra_dec_grid: true,
                ..NONE
            },
            None,
        );
        let (time, loc, cal) = base();
        let lst = astro::lst_deg(time, loc.longitude_deg);
        let ncp = astro::ra_dec_to_alt_az(0.0, 90.0, loc.latitude_deg, lst);
        let pole = astro::alt_az_to_image(
            ncp.alt_deg,
            ncp.az_deg,
            &cal,
            &astro::LensView {
                frame_width: 960,
                frame_height: 960,
                native_width: 960,
            },
        );
        let at_pole = g.polylines.iter().filter(|pl| {
            pl.points
                .iter()
                .any(|p| ((p[0] - pole.x).powi(2) + (p[1] - pole.y).powi(2)).sqrt() < 0.01)
        });
        assert!(at_pole.count() >= 12);
        let ring = g.polylines.iter().any(|pl| {
            pl.points.len() == 121
                && pl.points.iter().all(|p| {
                    ((p[0] - pole.x).powi(2) + (p[1] - pole.y).powi(2)).sqrt() < 0.13 * 440.0
                })
        });
        assert!(ring);
        // nothing leaves the horizon circle
        for pl in &g.polylines {
            assert_eq!(pl.layer, "raDec");
            for p in &pl.points {
                assert!(((p[0] - 480.0).powi(2) + (p[1] - 480.0).powi(2)).sqrt() <= 440.01);
            }
        }
    }

    #[test]
    fn grid_opacity_is_stamped_and_serialized_camel_case() {
        let g = build(
            OverlayLayers {
                alt_az_grid: true,
                ra_dec_grid: true,
                ..NONE
            },
            Some(0.3),
        );
        assert!(g.polylines.iter().all(|p| p.opacity == Some(0.3)));
        let v = serde_json::to_value(&g).unwrap();
        assert!(v["imageWidth"].is_number());
        assert_eq!(v["polylines"][0]["opacity"], 0.3);
    }

    #[test]
    fn mask_circle_culls_grid_points_outside_it_but_not_cardinal_labels() {
        let layers = OverlayLayers {
            alt_az_grid: true,
            ra_dec_grid: true,
            cardinal: true,
            constellations: false,
        };
        let mask = MaskCircle {
            center_x_px: 500.0,
            center_y_px: 460.0,
            radius_px: 200.0,
        };
        let dist = |x: f64, y: f64| {
            ((x - mask.center_x_px).powi(2) + (y - mask.center_y_px).powi(2)).sqrt()
        };

        let unmasked = build(layers, None);
        assert!(unmasked
            .polylines
            .iter()
            .any(|pl| pl.points.iter().any(|p| dist(p[0], p[1]) > mask.radius_px)));

        let (time, loc, cal) = base();
        let g = build_overlay_geometry(&BuildOptions {
            time,
            location: &loc,
            calibration: &cal,
            layers: &layers,
            grid_opacity: None,
            constellations_opacity: None,
            image_width: 960,
            image_height: 960,
            native_width: 960,
            mask: Some(mask),
        });
        assert!(!g.polylines.is_empty());
        for pl in &g.polylines {
            assert!(pl.points.len() > 1);
            for p in &pl.points {
                assert!(dist(p[0], p[1]) <= mask.radius_px + 0.01);
            }
        }
        // cardinal labels are annotations, not sky lines — the mask leaves them
        let mut texts: Vec<_> = g.labels.iter().map(|l| l.text.as_str()).collect();
        texts.sort_unstable();
        assert_eq!(texts, ["E", "N", "S", "W"]);
    }

    #[test]
    fn rectilinear_culls_the_horizon_circle() {
        let (time, loc, mut cal) = base();
        cal.lens_type = crate::settings::LensType::Rectilinear;
        let g = build_overlay_geometry(&BuildOptions {
            time,
            location: &loc,
            calibration: &cal,
            layers: &OverlayLayers {
                alt_az_grid: true,
                ..NONE
            },
            grid_opacity: None,
            constellations_opacity: None,
            image_width: 960,
            image_height: 960,
            native_width: 960,
            mask: None,
        });
        // The alt-0 horizon circle sits at θ = 90° — beyond the 85°
        // rectilinear limit — so only the alt 30/60 circles and the 8
        // radials (their lowest points culled) survive: 2 + 8 = 10.
        assert_eq!(g.polylines.len(), 10);
    }

    #[test]
    fn crop_offsets_points_and_labels() {
        let g = OverlayGeometry {
            image_width: 1280,
            image_height: 960,
            polylines: vec![OverlayPolyline {
                layer: "altAz".into(),
                points: vec![[200.0, 150.0], [300.0, 250.0]],
                opacity: Some(0.3),
            }],
            labels: vec![OverlayLabel {
                layer: "cardinal".into(),
                text: "N".into(),
                x: 640.0,
                y: 30.0,
                font_size: 28.0,
                align: None,
            }],
        };
        let c = crop_geometry(
            g,
            &CropRect {
                x: 100.0,
                y: 50.0,
                width: 800.0,
                height: 700.0,
            },
        );
        assert_eq!(c.image_width, 800);
        assert_eq!(c.polylines[0].points, vec![[100.0, 100.0], [200.0, 200.0]]);
        assert_eq!(c.polylines[0].opacity, Some(0.3));
        assert_eq!(c.labels[0].x, 540.0);
        assert_eq!(c.labels[0].y, -20.0);
    }

    #[test]
    fn text_fields_render_from_context_with_dashes_for_missing() {
        use crate::settings::{OverlayTextField, TextFieldKind};
        let mut g = build(NONE, None);
        let fields = vec![
            OverlayTextField {
                id: "a".into(),
                kind: TextFieldKind::Time,
                x: 24.0,
                y: 40.0,
                font_size: 24.0,
            },
            OverlayTextField {
                id: "b".into(),
                kind: TextFieldKind::Exposure,
                x: 24.0,
                y: 72.0,
                font_size: 18.0,
            },
            OverlayTextField {
                id: "c".into(),
                kind: TextFieldKind::SensorTemp,
                x: 24.0,
                y: 104.0,
                font_size: 18.0,
            },
        ];
        let ctx = TextContext {
            local_time: "2026-07-15 22:00:00".into(),
            exposure_us: Some(30_000_000),
            gain: Some(8.0),
            sensor_temp_c: None,
        };
        append_text_fields(&mut g, &fields, &ctx);
        assert_eq!(g.labels.len(), 3);
        assert!(g
            .labels
            .iter()
            .all(|l| l.layer == "text" && l.align.as_deref() == Some("left")));
        assert_eq!(g.labels[0].text, "2026-07-15 22:00:00");
        assert_eq!(g.labels[1].text, "exp 30 s · gain 8.00");
        assert_eq!(g.labels[2].text, "—°C");
    }

    #[test]
    fn format_exposure_covers_both_ranges() {
        assert_eq!(format_exposure(30_000_000), "30 s");
        assert_eq!(format_exposure(2_500_000), "2.5 s");
        assert_eq!(format_exposure(4_000), "0.004 s");
        assert_eq!(format_exposure(32), "0.000032 s");
    }

    #[test]
    fn dec_zero_circle_reaches_the_horizon_at_latitude_90() {
        // At latitude 90 the celestial equator IS the horizon; the old 2°
        // altitude floor culled the whole dec-0 circle.
        let (time, mut loc, cal) = base();
        loc.latitude_deg = 90.0;
        loc.longitude_deg = 0.0;
        let geometry = build_overlay_geometry(&BuildOptions {
            time,
            location: &loc,
            calibration: &cal,
            layers: &OverlayLayers {
                ra_dec_grid: true,
                ..NONE
            },
            grid_opacity: None,
            constellations_opacity: None,
            image_width: 1280,
            image_height: 960,
            native_width: 1280,
            mask: None,
        });
        let max_r = geometry
            .polylines
            .iter()
            .flat_map(|p| p.points.iter())
            .map(|&[x, y]| ((x - 640.0).powi(2) + (y - 480.0).powi(2)).sqrt())
            .fold(0.0_f64, f64::max);
        assert!(
            (max_r - 440.0).abs() < 1.0,
            "raDec must reach the horizon radius, got {max_r}"
        );
    }

    #[test]
    fn constellations_opacity_is_stamped_on_lines_but_not_labels() {
        let (time, loc, cal) = base();
        let layers = OverlayLayers {
            constellations: true,
            ..NONE
        };
        let g = build_overlay_geometry(&BuildOptions {
            time,
            location: &loc,
            calibration: &cal,
            layers: &layers,
            grid_opacity: None,
            constellations_opacity: Some(0.2),
            image_width: 960,
            image_height: 960,
            native_width: 960,
            mask: None,
        });
        assert!(!g.polylines.is_empty());
        assert!(g
            .polylines
            .iter()
            .all(|p| p.layer == "constellations" && p.opacity == Some(0.2)));
        assert!(!g.labels.is_empty());
        assert!(g.labels.iter().all(|l| l.layer == "constellationLabels"));
    }

    #[test]
    fn constellations_layer_projects_a_known_constellation_and_its_label() {
        let g = build(
            OverlayLayers {
                constellations: true,
                ..NONE
            },
            None,
        );
        let (time, loc, cal) = base();
        let lst = astro::lst_deg(time, loc.longitude_deg);
        let view = astro::LensView {
            frame_width: 960,
            frame_height: 960,
            native_width: 960,
        };
        let project = |ra: f64, dec: f64| -> (f64, f64) {
            let aa = astro::ra_dec_to_alt_az(ra, dec, loc.latitude_deg, lst);
            let p = astro::alt_az_to_image(aa.alt_deg, aa.az_deg, &cal, &view);
            (p.x, p.y)
        };
        // Ursa Minor (UMi) from frontend/src/lib/constellations.json —
        // circumpolar at latitude 50.45 (lowest dec 71.8 > 90 - lat), so
        // every point stays above the horizon and this renders as a single
        // unsplit 8-point polyline. Same fixed vertices as the paired TS
        // test in overlayGeometry.test.ts.
        const UMI_LINE: [(f64, f64); 8] = [
            (236.0147, 77.7945),
            (244.3762, 75.7553),
            (230.1821, 71.834),
            (222.6764, 74.1555),
            (236.0147, 77.7945),
            (251.4927, 82.0373),
            (263.0542, 86.5865),
            (37.9545, 89.2641),
        ];
        let expected: Vec<(f64, f64)> =
            UMI_LINE.iter().map(|&(ra, dec)| project(ra, dec)).collect();
        let found =
            g.polylines.iter().any(|pl| {
                pl.layer == "constellations"
                    && pl.points.len() == expected.len()
                    && pl.points.iter().zip(expected.iter()).all(|(p, &(ex, ey))| {
                        ((p[0] - ex).powi(2) + (p[1] - ey).powi(2)).sqrt() < 1e-6
                    })
            });
        assert!(found, "expected a projected Ursa Minor polyline");

        let (lx, ly) = project(226.5, 68.0); // UMi's label_ra_deg/label_dec_deg
        let label = g
            .labels
            .iter()
            .find(|l| l.layer == "constellationLabels" && l.text == "Ursa Minor");
        assert!(label.is_some(), "expected a Ursa Minor label");
        let label = label.unwrap();
        assert!((label.x - lx).abs() < 1e-6);
        assert!((label.y - ly).abs() < 1e-6);
    }
}
