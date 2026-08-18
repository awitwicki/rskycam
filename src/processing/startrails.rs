use image::RgbImage;

/// Mean luma of the frame's center box (half the width × half the height).
/// The brightness gate below is a sky-brightness test, but on an allsky
/// frame the whole-image mean is dominated by what surrounds the sky —
/// ground lights, buildings, the fisheye border — so a clear dark night
/// can still average far above any sensible limit. The center of the
/// frame is sky by construction; measure that instead.
fn center_zone_mean(img: &RgbImage) -> f64 {
    let (w, h) = img.dimensions();
    let (cw, ch) = ((w / 2).max(1), (h / 2).max(1));
    let crop = image::imageops::crop_imm(img, (w - cw) / 2, (h - ch) / 2, cw, ch).to_image();
    crate::camera::mean_brightness(&crop)
}

/// Incremental star-trails: per-pixel per-channel max (lighten blend).
/// Frames whose center-zone mean brightness exceeds the configured limit
/// (clouds, moonlight, dawn) are skipped, as are frames whose dimensions
/// differ from the accumulator (mid-night resolution change).
#[derive(Default)]
pub struct Startrails {
    acc: Option<RgbImage>,
    pub used: u32,
    pub skipped: u32,
}

impl Startrails {
    pub fn add_frame(&mut self, img: &RgbImage, limit: f64) -> bool {
        if center_zone_mean(img) > limit {
            self.skipped += 1;
            return false;
        }
        match &mut self.acc {
            None => self.acc = Some(img.clone()),
            Some(acc) => {
                if acc.dimensions() != img.dimensions() {
                    self.skipped += 1;
                    return false;
                }
                for (a, p) in acc.pixels_mut().zip(img.pixels()) {
                    for c in 0..3 {
                        a.0[c] = a.0[c].max(p.0[c]);
                    }
                }
            }
        }
        self.used += 1;
        true
    }

    pub fn to_image(&self) -> Option<&RgbImage> {
        self.acc.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    #[test]
    fn lighten_blend_takes_per_channel_max() {
        let mut st = Startrails::default();
        assert!(st.add_frame(&RgbImage::from_pixel(2, 2, Rgb([10, 200, 30])), 255.0));
        assert!(st.add_frame(&RgbImage::from_pixel(2, 2, Rgb([90, 20, 60])), 255.0));
        let img = st.to_image().unwrap();
        assert_eq!(img.get_pixel(0, 0), &Rgb([90, 200, 60]));
        assert_eq!((st.used, st.skipped), (2, 0));
    }

    #[test]
    fn bright_frames_are_skipped() {
        let mut st = Startrails::default();
        assert!(!st.add_frame(&RgbImage::from_pixel(2, 2, Rgb([200, 200, 200])), 35.0));
        assert!(st.to_image().is_none());
        assert_eq!((st.used, st.skipped), (0, 1));
    }

    #[test]
    fn mismatched_dimensions_are_skipped() {
        let mut st = Startrails::default();
        assert!(st.add_frame(&RgbImage::from_pixel(2, 2, Rgb([1, 1, 1])), 35.0));
        assert!(!st.add_frame(&RgbImage::from_pixel(4, 4, Rgb([9, 9, 9])), 35.0));
        assert_eq!((st.used, st.skipped), (1, 1));
        assert_eq!(st.to_image().unwrap().width(), 2);
    }

    /// Paint the center box (w/2 × h/2) of an image one color, the border
    /// another — the shape of a fisheye allsky frame: sky in the middle,
    /// ground lights at the edges.
    fn framed(border: [u8; 3], center: [u8; 3]) -> RgbImage {
        let mut img = RgbImage::from_pixel(8, 8, Rgb(border));
        for y in 2..6 {
            for x in 2..6 {
                img.put_pixel(x, y, Rgb(center));
            }
        }
        img
    }

    #[test]
    fn gate_measures_the_center_zone_not_the_whole_frame() {
        // Bright border (ground lights), dark center (clear sky): the
        // whole-frame mean (~194) is far over the limit, but the center
        // zone (10) is what matters — must be accepted.
        let mut st = Startrails::default();
        assert!(st.add_frame(&framed([255, 255, 255], [10, 10, 10]), 35.0));
        assert_eq!((st.used, st.skipped), (1, 0));

        // Inverse — dark border, bright center (moonlit/cloudy sky):
        // whole-frame mean (~50) sneaks under a lenient read, but the sky
        // itself is bright — must be rejected.
        let mut st = Startrails::default();
        assert!(!st.add_frame(&framed([0, 0, 0], [200, 200, 200]), 35.0));
        assert_eq!((st.used, st.skipped), (0, 1));
    }
}
