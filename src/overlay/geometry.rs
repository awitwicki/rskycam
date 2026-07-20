use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::overlay::astro;
use crate::settings::{
    CropRect, LensCalibration, LocationSettings, OverlayLayers, OverlayTextField, TextFieldKind,
};

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

pub struct BuildOptions<'a> {
    pub time: DateTime<Utc>,
    pub location: &'a LocationSettings,
    pub calibration: &'a LensCalibration,
    pub layers: &'a OverlayLayers,
    pub grid_opacity: Option<f64>,
    pub image_width: u32,
    pub image_height: u32,
}

const MIN_ALT_RADEC: f64 = 2.0;

/// Split a sampled line into segments that stay above the horizon.
fn segments_above_horizon(samples: &[(f64, f64, f64)]) -> Vec<Vec<[f64; 2]>> {
    let mut segs = Vec::new();
    let mut cur: Vec<[f64; 2]> = Vec::new();
    for &(alt, x, y) in samples {
        if alt >= MIN_ALT_RADEC {
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
    let mut polylines = Vec::new();
    let mut labels = Vec::new();
    let opacity = o.grid_opacity;

    if o.layers.alt_az_grid {
        for alt in [0.0f64, 30.0, 60.0] {
            let mut points = Vec::new();
            let mut az = 0.0f64;
            while az <= 360.0 {
                let p = astro::alt_az_to_image(alt, az, cal);
                points.push([p.x, p.y]);
                az += 5.0;
            }
            polylines.push(OverlayPolyline {
                layer: "altAz".into(),
                points,
                opacity,
            });
        }
        let mut az = 0.0f64;
        while az < 360.0 {
            let mut points = Vec::new();
            let mut alt = 0.0f64;
            while alt <= 80.0 {
                let p = astro::alt_az_to_image(alt, az, cal);
                points.push([p.x, p.y]);
                alt += 5.0;
            }
            polylines.push(OverlayPolyline {
                layer: "altAz".into(),
                points,
                opacity,
            });
            az += 45.0;
        }
    }

    if o.layers.cardinal {
        for (text, az) in [("N", 0.0), ("E", 90.0), ("S", 180.0), ("W", 270.0)] {
            let p = astro::alt_az_to_image(-8.0, az, cal); // a bit outside the horizon circle
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
        let lst = astro::lst_deg(o.time, o.location.longitude_deg);
        let lat = o.location.latitude_deg;
        let sample = |ra: f64, dec: f64| -> (f64, f64, f64) {
            let aa = astro::ra_dec_to_alt_az(ra, dec, lat, lst);
            let p = astro::alt_az_to_image(aa.alt_deg, aa.az_deg, cal);
            (aa.alt_deg, p.x, p.y)
        };
        // ±80 keeps a small circle around each celestial pole (no hole).
        for dec in [-80.0f64, -60.0, -30.0, 0.0, 30.0, 60.0, 80.0] {
            let mut samples = Vec::new();
            let mut ra = 0.0f64;
            while ra <= 360.0 {
                samples.push(sample(ra, dec));
                ra += 3.0;
            }
            for points in segments_above_horizon(&samples) {
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
            for points in segments_above_horizon(&samples) {
                polylines.push(OverlayPolyline {
                    layer: "raDec".into(),
                    points,
                    opacity,
                });
            }
            ra += 30.0;
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
        format!("1/{} s", (1.0 / s).round() as u64)
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
                cx: 480.0,
                cy: 480.0,
                radius_px: 440.0,
                rotation_deg: 0.0,
                flip: false,
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
            image_width: 960,
            image_height: 960,
        })
    }

    const NONE: OverlayLayers = OverlayLayers {
        cardinal: false,
        alt_az_grid: false,
        ra_dec_grid: false,
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
        let pole = astro::alt_az_to_image(ncp.alt_deg, ncp.az_deg, &cal);
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
        assert_eq!(format_exposure(4_000), "1/250 s");
    }
}
