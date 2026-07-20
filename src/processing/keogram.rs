use ab_glyph::{FontRef, PxScale};
use chrono::NaiveDateTime;
use image::{Rgb, RgbImage};
use imageproc::drawing::{draw_text_mut, text_size};

/// Height of the time-scale band appended below the pixel strip.
const BAND_H: u32 = 28;
/// Band background — matches the UI's dark panel tone.
const BAND_BG: Rgb<u8> = Rgb([13, 18, 30]);
/// Label/date color — matches the overlay text color.
const TEXT: Rgb<u8> = Rgb([226, 232, 244]);
const LABEL_PX: f32 = 15.0;

/// Capture time from a frame filename ("20260716-220130.jpg", local time).
pub fn frame_time(file: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(file.get(..15)?, "%Y%m%d-%H%M%S").ok()
}

/// Incremental keogram: the central vertical pixel column of every frame,
/// appended left-to-right in arrival order. The first frame fixes the
/// keogram height; later frames of a different height get their column
/// nearest-neighbour resampled so a mid-night resolution change doesn't
/// leave gaps. Each column carries its capture time so the rendered file
/// can grow an hour scale.
#[derive(Default)]
pub struct Keogram {
    height: Option<u32>,
    cols: Vec<Vec<Rgb<u8>>>,
    times: Vec<Option<NaiveDateTime>>,
}

impl Keogram {
    pub fn add_frame(&mut self, img: &RgbImage, time: Option<NaiveDateTime>) {
        let h = *self.height.get_or_insert(img.height());
        let x = img.width() / 2;
        let src_h = img.height();
        let col: Vec<Rgb<u8>> = (0..h)
            .map(|y| {
                let sy = if src_h == h { y } else { y * src_h / h };
                *img.get_pixel(x, sy.min(src_h - 1))
            })
            .collect();
        self.cols.push(col);
        self.times.push(time);
    }

    pub fn to_image(&self) -> Option<RgbImage> {
        let h = self.height?;
        if self.cols.is_empty() {
            return None;
        }
        let mut img = RgbImage::new(self.cols.len() as u32, h);
        for (x, col) in self.cols.iter().enumerate() {
            for (y, px) in col.iter().enumerate() {
                img.put_pixel(x as u32, y as u32, *px);
            }
        }
        Some(img)
    }

    /// Whole-hour boundaries as (column, "HH:00") pairs. A boundary maps to
    /// the first column captured at or after it; boundaries at column 0 are
    /// skipped (a line on the very edge marks nothing).
    fn hour_marks(&self) -> Vec<(u32, String)> {
        let timed: Vec<(usize, NaiveDateTime)> = self
            .times
            .iter()
            .enumerate()
            .filter_map(|(i, t)| t.map(|t| (i, t)))
            .collect();
        let (Some(&(_, first)), Some(&(_, last))) = (timed.first(), timed.last()) else {
            return Vec::new();
        };
        let mut marks = Vec::new();
        let mut t = first
            .date()
            .and_hms_opt(chrono::Timelike::hour(&first.time()), 0, 0)
            .expect("valid hour")
            + chrono::Duration::hours(1);
        while t <= last {
            if let Some(&(i, _)) = timed.iter().find(|(_, ct)| *ct >= t) {
                if i > 0 {
                    marks.push((i as u32, t.format("%H:%M").to_string()));
                }
            }
            t += chrono::Duration::hours(1);
        }
        marks.dedup_by_key(|m| m.0); // an empty hour maps to the same column
        marks
    }

    /// The keogram as written to disk: the pixel strip plus a bottom band
    /// with hour labels and the night's date, and semi-transparent dashed
    /// vertical lines at whole-hour boundaries.
    pub fn annotated(&self, date_label: &str) -> Option<RgbImage> {
        let strip = self.to_image()?;
        let (w, sh) = (strip.width(), strip.height());
        let mut img = RgbImage::from_pixel(w, sh + BAND_H, BAND_BG);
        for (x, y, px) in strip.enumerate_pixels() {
            img.put_pixel(x, y, *px);
        }

        let font =
            FontRef::try_from_slice(crate::overlay::bake::FONT_BYTES).expect("embedded font");
        let scale = PxScale::from(LABEL_PX);
        let marks = self.hour_marks();

        // Dashed 50%-white hour lines over the strip (4 px on / 4 px off).
        for (x, _) in &marks {
            for y in 0..sh {
                if (y / 4) % 2 == 0 {
                    let p = img.get_pixel_mut(*x, y);
                    for c in &mut p.0 {
                        *c = ((*c as u16 + 255) / 2) as u8;
                    }
                }
            }
        }

        // The night's date, right-aligned in the band. Drawn first so hour
        // labels know to stay clear of its zone.
        let (dw, dh) = text_size(scale, &font, date_label);
        let dx = (w as i32 - dw as i32 - 6).max(0);
        let dy = sh as i32 + (BAND_H as i32 - dh as i32) / 2;
        draw_text_mut(&mut img, TEXT, dx, dy, scale, &font, date_label);

        // Hour labels centered under their lines; skip ones that would
        // overlap the previous label (sparse keograms) or run into the date.
        let mut last_right = i32::MIN;
        for (x, label) in &marks {
            let (tw, th) = text_size(scale, &font, label);
            let xl = (*x as i32 - tw as i32 / 2).clamp(0, (w as i32 - tw as i32).max(0));
            if xl <= last_right + 8 || xl + tw as i32 > dx - 8 {
                continue;
            }
            let yt = sh as i32 + (BAND_H as i32 - th as i32) / 2;
            draw_text_mut(&mut img, TEXT, xl, yt, scale, &font, label);
            last_right = xl + tw as i32;
        }
        Some(img)
    }

    // Not called by the supervisor (it only needs add_frame/to_image); kept
    // for the len_without_is_empty convention and exercised by the tests below.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.cols.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.cols.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 4x3 frame whose center column (x = 2) is red; everything else black.
    fn frame_with_red_center_column() -> RgbImage {
        let mut img = RgbImage::new(4, 3);
        for y in 0..3 {
            img.put_pixel(2, y, Rgb([255, 0, 0]));
        }
        img
    }

    #[test]
    fn takes_the_central_column_and_grows_one_per_frame() {
        let mut k = Keogram::default();
        assert!(k.is_empty());
        k.add_frame(&frame_with_red_center_column(), None);
        k.add_frame(&RgbImage::from_pixel(4, 3, Rgb([0, 0, 255])), None);
        assert_eq!(k.len(), 2);
        let img = k.to_image().unwrap();
        assert_eq!((img.width(), img.height()), (2, 3));
        for y in 0..3 {
            assert_eq!(img.get_pixel(0, y), &Rgb([255, 0, 0])); // column 0 = frame 1 center
            assert_eq!(img.get_pixel(1, y), &Rgb([0, 0, 255])); // column 1 = frame 2
        }
    }

    #[test]
    fn resamples_a_frame_of_different_height() {
        let mut k = Keogram::default();
        k.add_frame(&RgbImage::from_pixel(4, 4, Rgb([10, 10, 10])), None); // fixes height 4
                                                                           // 2-high frame: top half white, bottom half black
        let mut small = RgbImage::new(4, 2);
        for x in 0..4 {
            small.put_pixel(x, 0, Rgb([255, 255, 255]));
        }
        k.add_frame(&small, None);
        let img = k.to_image().unwrap();
        assert_eq!(img.height(), 4);
        // nearest-neighbour: rows 0-1 from src row 0 (white), rows 2-3 from src row 1 (black)
        assert_eq!(img.get_pixel(1, 0), &Rgb([255, 255, 255]));
        assert_eq!(img.get_pixel(1, 1), &Rgb([255, 255, 255]));
        assert_eq!(img.get_pixel(1, 2), &Rgb([0, 0, 0]));
        assert_eq!(img.get_pixel(1, 3), &Rgb([0, 0, 0]));
    }

    #[test]
    fn empty_keogram_yields_no_image() {
        assert!(Keogram::default().to_image().is_none());
        assert!(Keogram::default().annotated("2026-07-15").is_none());
    }

    fn t(s: &str) -> Option<chrono::NaiveDateTime> {
        Some(chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap())
    }

    #[test]
    fn frame_time_parses_capture_filenames() {
        assert_eq!(frame_time("20260716-220130.jpg"), t("2026-07-16 22:01:30"));
        assert_eq!(frame_time("garbage.jpg"), None);
        assert_eq!(frame_time(""), None);
    }

    #[test]
    fn annotated_draws_a_dashed_hour_line_at_the_boundary_column() {
        let dark = RgbImage::from_pixel(8, 6, Rgb([10, 10, 10]));
        let mut k = Keogram::default();
        k.add_frame(&dark, t("2026-07-15 21:59:30"));
        k.add_frame(&dark, t("2026-07-15 22:00:30")); // first column past 22:00
        k.add_frame(&dark, t("2026-07-15 22:01:30"));
        let img = k.annotated("2026-07-15").unwrap();
        assert_eq!((img.width(), img.height()), (3, 6 + BAND_H));
        // Dash-on segment at the boundary column is blended toward white...
        assert!(
            img.get_pixel(1, 0).0[0] > 100,
            "expected a bright dashed pixel, got {:?}",
            img.get_pixel(1, 0)
        );
        // ...the dash-off gap and the neighbouring columns stay untouched.
        assert_eq!(img.get_pixel(1, 4), &Rgb([10, 10, 10]));
        assert_eq!(img.get_pixel(0, 0), &Rgb([10, 10, 10]));
        assert_eq!(img.get_pixel(2, 0), &Rgb([10, 10, 10]));
    }

    #[test]
    fn annotated_band_carries_hour_label_and_date_text() {
        let dark = RgbImage::from_pixel(8, 6, Rgb([10, 10, 10]));
        let mut k = Keogram::default();
        for i in 0..120 {
            // 21:30..23:30 at one frame per minute — one boundary at 22:00 (col 30)
            // and one at 23:00 (col 90).
            let time = chrono::NaiveDate::from_ymd_opt(2026, 7, 15)
                .unwrap()
                .and_hms_opt(21, 30, 0)
                .unwrap()
                + chrono::Duration::minutes(i);
            k.add_frame(&dark, Some(time));
        }
        let img = k.annotated("2026-07-15").unwrap();
        assert_eq!(img.height(), 6 + BAND_H);
        // The band (below the strip) must contain text pixels — hour labels
        // and the date — i.e. plenty of non-background pixels.
        let lit = (0..img.width())
            .flat_map(|x| (6..img.height()).map(move |y| (x, y)))
            .filter(|&(x, y)| img.get_pixel(x, y) != &BAND_BG)
            .count();
        assert!(
            lit > 30,
            "expected label/date pixels in the band, got {lit}"
        );
    }

    #[test]
    fn annotated_without_times_has_the_band_but_no_hour_lines() {
        let dark = RgbImage::from_pixel(8, 6, Rgb([10, 10, 10]));
        let mut k = Keogram::default();
        for _ in 0..120 {
            k.add_frame(&dark, None);
        }
        let img = k.annotated("2026-07-15").unwrap();
        assert_eq!(img.height(), 6 + BAND_H);
        // No time info → no dashed lines anywhere in the strip.
        for x in 0..img.width() {
            for y in 0..6 {
                assert_eq!(img.get_pixel(x, y), &Rgb([10, 10, 10]));
            }
        }
        // The date is still drawn in the band.
        let lit = (0..img.width())
            .flat_map(|x| (6..img.height()).map(move |y| (x, y)))
            .filter(|&(x, y)| img.get_pixel(x, y) != &BAND_BG)
            .count();
        assert!(lit > 10, "expected date pixels in the band, got {lit}");
    }
}
