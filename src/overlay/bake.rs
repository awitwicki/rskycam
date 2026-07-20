use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use imageproc::pixelops::interpolate;

use crate::overlay::geometry::OverlayGeometry;

// Shared with the keogram annotator so the font is embedded exactly once.
pub(crate) const FONT_BYTES: &[u8] = include_bytes!("../../assets/fonts/DejaVuSansMono.ttf");

/// Colors and alphas mirror frontend OverlayCanvas.tsx LAYER_STYLE so the
/// baked frame matches what the browser preview shows.
fn layer_style(layer: &str) -> (Rgb<u8>, f32) {
    match layer {
        "altAz" => (Rgb([76, 201, 240]), 1.0),
        "raDec" => (Rgb([240, 164, 76]), 1.0),
        "cardinal" => (Rgb([226, 232, 244]), 0.9),
        "text" => (Rgb([226, 232, 244]), 0.95),
        _ => (Rgb([255, 255, 255]), 0.4),
    }
}

/// Rasterize the overlay geometry onto the image in place.
pub fn bake_overlay(img: &mut RgbImage, geo: &OverlayGeometry) {
    let font = FontRef::try_from_slice(FONT_BYTES).expect("embedded font is valid");
    let (w, h) = (img.width() as i32, img.height() as i32);

    for pl in &geo.polylines {
        let (color, layer_alpha) = layer_style(&pl.layer);
        let alpha = layer_alpha * pl.opacity.unwrap_or(1.0) as f32;
        if alpha <= 0.0 {
            continue;
        }
        for pair in pl.points.windows(2) {
            let (x0, y0) = (pair[0][0] as i32, pair[0][1] as i32);
            let (x1, y1) = (pair[1][0] as i32, pair[1][1] as i32);
            // Cheap clip: skip segments entirely outside; imageproc clips
            // partially-outside antialiased segments per pixel.
            if (x0 < 0 && x1 < 0)
                || (y0 < 0 && y1 < 0)
                || (x0 >= w && x1 >= w)
                || (y0 >= h && y1 >= h)
            {
                continue;
            }
            imageproc::drawing::draw_antialiased_line_segment_mut(
                img,
                (x0, y0),
                (x1, y1),
                color,
                |line, bg, weight| interpolate(line, bg, weight * alpha),
            );
        }
    }

    for l in &geo.labels {
        let (color, _alpha) = layer_style(&l.layer);
        // draw_text_mut is opaque; label layer alphas are >= 0.9, which is
        // visually indistinguishable from 1.0 — deliberate simplification.
        let scale = PxScale::from(l.font_size as f32);
        let (tw, _th) = imageproc::drawing::text_size(scale, &font, &l.text);
        // Canvas draws with textBaseline='middle'; draw_text_mut's y is the
        // glyph top. Center vertically using the scaled font's ascent+descent.
        let scaled = font.as_scaled(scale);
        let text_h = scaled.ascent() - scaled.descent();
        let x_left = match l.align.as_deref() {
            Some("left") => l.x,
            _ => l.x - tw as f64 / 2.0, // canvas default in OverlayCanvas is center
        };
        let y_top = l.y - text_h as f64 / 2.0;
        imageproc::drawing::draw_text_mut(
            img,
            color,
            x_left.round() as i32,
            y_top.round() as i32,
            scale,
            &font,
            &l.text,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::geometry::{OverlayLabel, OverlayPolyline};

    fn black(w: u32, h: u32) -> RgbImage {
        RgbImage::new(w, h)
    }

    fn geo(polylines: Vec<OverlayPolyline>, labels: Vec<OverlayLabel>) -> OverlayGeometry {
        OverlayGeometry {
            image_width: 100,
            image_height: 100,
            polylines,
            labels,
        }
    }

    #[test]
    fn draws_a_polyline_with_its_opacity() {
        let mut img = black(100, 100);
        bake_overlay(
            &mut img,
            &geo(
                vec![OverlayPolyline {
                    layer: "altAz".into(),
                    points: vec![[10.0, 50.0], [90.0, 50.0]],
                    opacity: Some(0.5),
                }],
                vec![],
            ),
        );
        // Mid-line pixel: black blended 50% toward rgb(76,201,240).
        let px = img.get_pixel(50, 50);
        assert!(
            px.0[2] > 100 && px.0[2] < 140,
            "blue channel was {}",
            px.0[2]
        );
        // Far from the line: untouched.
        assert_eq!(img.get_pixel(50, 10), &Rgb([0, 0, 0]));
    }

    #[test]
    fn full_opacity_line_reaches_the_layer_color() {
        let mut img = black(100, 100);
        bake_overlay(
            &mut img,
            &geo(
                vec![OverlayPolyline {
                    layer: "raDec".into(),
                    points: vec![[10.0, 50.0], [90.0, 50.0]],
                    opacity: None,
                }],
                vec![],
            ),
        );
        assert_eq!(img.get_pixel(50, 50), &Rgb([240, 164, 76]));
    }

    #[test]
    fn draws_label_text_pixels_near_the_anchor() {
        let mut img = black(100, 100);
        bake_overlay(
            &mut img,
            &geo(
                vec![],
                vec![OverlayLabel {
                    layer: "text".into(),
                    text: "N".into(),
                    x: 50.0,
                    y: 50.0,
                    font_size: 20.0,
                    align: Some("center".into()),
                }],
            ),
        );
        // Some non-black pixels must appear within a box around the anchor
        // (baseline math means we don't assert exact pixels).
        let lit = (35..65)
            .flat_map(|x| (35..65).map(move |y| (x, y)))
            .filter(|&(x, y)| img.get_pixel(x, y).0 != [0, 0, 0])
            .count();
        assert!(lit > 5, "expected text pixels near the anchor, got {lit}");
    }

    #[test]
    fn out_of_bounds_points_do_not_panic() {
        let mut img = black(50, 50);
        bake_overlay(
            &mut img,
            &geo(
                vec![OverlayPolyline {
                    layer: "altAz".into(),
                    points: vec![[-20.0, 25.0], [80.0, 25.0]],
                    opacity: Some(1.0),
                }],
                vec![OverlayLabel {
                    layer: "cardinal".into(),
                    text: "W".into(),
                    x: -10.0,
                    y: 200.0,
                    font_size: 16.0,
                    align: None,
                }],
            ),
        );
    }

    #[test]
    fn real_geometry_bakes_without_panicking_and_changes_pixels() {
        use crate::overlay::geometry::{build_overlay_geometry, BuildOptions};
        use crate::settings::Settings;
        let s = Settings::default();
        let g = build_overlay_geometry(&BuildOptions {
            time: chrono::Utc::now(),
            location: &s.location,
            calibration: &s.overlay.calibration,
            layers: &s.overlay.layers,
            grid_opacity: Some(s.overlay.grid_opacity),
            image_width: 1280,
            image_height: 960,
        });
        let mut img = black(1280, 960);
        bake_overlay(&mut img, &g);
        assert!(img.pixels().any(|p| p.0 != [0, 0, 0]));
    }
}
