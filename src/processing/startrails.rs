use image::RgbImage;

/// Incremental star-trails: per-pixel per-channel max (lighten blend).
/// Frames whose mean brightness exceeds the configured limit (clouds,
/// moonlight, dawn) are skipped, as are frames whose dimensions differ
/// from the accumulator (mid-night resolution change).
#[derive(Default)]
pub struct Startrails {
    acc: Option<RgbImage>,
    pub used: u32,
    pub skipped: u32,
}

impl Startrails {
    pub fn add_frame(&mut self, img: &RgbImage, mean: f64, limit: f64) -> bool {
        if mean > limit {
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
        assert!(st.add_frame(&RgbImage::from_pixel(2, 2, Rgb([10, 200, 30])), 5.0, 35.0));
        assert!(st.add_frame(&RgbImage::from_pixel(2, 2, Rgb([90, 20, 60])), 5.0, 35.0));
        let img = st.to_image().unwrap();
        assert_eq!(img.get_pixel(0, 0), &Rgb([90, 200, 60]));
        assert_eq!((st.used, st.skipped), (2, 0));
    }

    #[test]
    fn bright_frames_are_skipped() {
        let mut st = Startrails::default();
        assert!(!st.add_frame(
            &RgbImage::from_pixel(2, 2, Rgb([200, 200, 200])),
            200.0,
            35.0
        ));
        assert!(st.to_image().is_none());
        assert_eq!((st.used, st.skipped), (0, 1));
    }

    #[test]
    fn mismatched_dimensions_are_skipped() {
        let mut st = Startrails::default();
        assert!(st.add_frame(&RgbImage::from_pixel(2, 2, Rgb([1, 1, 1])), 1.0, 35.0));
        assert!(!st.add_frame(&RgbImage::from_pixel(4, 4, Rgb([9, 9, 9])), 1.0, 35.0));
        assert_eq!((st.used, st.skipped), (1, 1));
        assert_eq!(st.to_image().unwrap().width(), 2);
    }
}
