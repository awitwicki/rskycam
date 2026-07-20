pub mod asi;
pub mod mock;
pub mod rpicam;

use chrono::{DateTime, Utc};
use image::RgbImage;

use crate::settings::{CropRect, LensCalibration};

pub struct CameraInfo {
    // Reserved for future UI display (Phase 3); not read by the capture pipeline today.
    #[allow(dead_code)]
    pub model: String,
    #[allow(dead_code)]
    pub width: u32,
    #[allow(dead_code)]
    pub height: u32,
    /// Native sensor maximum — the largest ROI the camera supports.
    pub max_width: u32,
    pub max_height: u32,
    pub min_exposure_us: u64,
    pub max_exposure_us: u64,
    pub min_gain: f64,
    pub max_gain: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptureParams {
    pub exposure_us: u64,
    pub gain: f64,
}

pub struct Frame {
    pub image: RgbImage,
    pub timestamp: DateTime<Utc>,
    pub exposure_us: u64,
    pub gain: f64,
}

#[derive(thiserror::Error, Debug)]
pub enum CameraError {
    #[error("camera unavailable: {0}")]
    Unavailable(String),
    #[error("capture failed: {0}")]
    Capture(String),
}

pub trait Camera: Send {
    fn info(&self) -> CameraInfo;
    fn capture(&mut self, p: CaptureParams) -> Result<Frame, CameraError>;
}

/// Mean luma (Rec.601) over all pixels, 0..255.
pub fn mean_brightness(img: &RgbImage) -> f64 {
    if img.pixels().len() == 0 {
        return 0.0;
    }
    let sum: f64 = img
        .pixels()
        .map(|p| 0.299 * p.0[0] as f64 + 0.587 * p.0[1] as f64 + 0.114 * p.0[2] as f64)
        .sum();
    sum / img.pixels().len() as f64
}

/// Black out everything outside the lens circle (maskMode = 'circle').
pub fn apply_mask_circle(img: &mut RgbImage, cal: &LensCalibration) {
    let r2 = cal.radius_px * cal.radius_px;
    for (x, y, px) in img.enumerate_pixels_mut() {
        let dx = x as f64 - cal.cx;
        let dy = y as f64 - cal.cy;
        if dx * dx + dy * dy > r2 {
            *px = image::Rgb([0, 0, 0]);
        }
    }
}

/// Crop clamped to image bounds; applied LAST in the pipeline.
pub fn apply_crop(img: &RgbImage, c: &CropRect) -> RgbImage {
    let x = (c.x.max(0.0) as u32).min(img.width().saturating_sub(1));
    let y = (c.y.max(0.0) as u32).min(img.height().saturating_sub(1));
    let w = (c.width as u32).min(img.width() - x);
    let h = (c.height as u32).min(img.height() - y);
    image::imageops::crop_imm(img, x, y, w.max(1), h.max(1)).to_image()
}

pub fn encode_jpeg(img: &RgbImage) -> Result<Vec<u8>, CameraError> {
    let mut buf = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 85);
    enc.encode_image(img)
        .map_err(|e| CameraError::Capture(e.to_string()))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{CropRect, LensCalibration};
    use image::RgbImage;

    #[test]
    fn mean_brightness_of_flat_images() {
        let black = RgbImage::from_pixel(8, 8, image::Rgb([0, 0, 0]));
        let gray = RgbImage::from_pixel(8, 8, image::Rgb([100, 100, 100]));
        assert_eq!(mean_brightness(&black), 0.0);
        assert!((mean_brightness(&gray) - 100.0).abs() < 0.6);
    }

    #[test]
    fn mask_circle_blacks_out_corners_keeps_center() {
        let mut img = RgbImage::from_pixel(100, 100, image::Rgb([200, 200, 200]));
        let cal = LensCalibration {
            cx: 50.0,
            cy: 50.0,
            radius_px: 40.0,
            rotation_deg: 0.0,
            flip: false,
        };
        apply_mask_circle(&mut img, &cal);
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 0]);
        assert_eq!(img.get_pixel(50, 50).0, [200, 200, 200]);
    }

    #[test]
    fn crop_clamps_to_bounds() {
        let img = RgbImage::from_pixel(100, 80, image::Rgb([9, 9, 9]));
        let c = apply_crop(
            &img,
            &CropRect {
                x: 60.0,
                y: 40.0,
                width: 100.0,
                height: 100.0,
            },
        );
        assert_eq!((c.width(), c.height()), (40, 40));
    }

    #[test]
    fn jpeg_encoding_produces_a_decodable_image() {
        let img = RgbImage::from_pixel(32, 32, image::Rgb([10, 20, 30]));
        let jpg = encode_jpeg(&img).unwrap();
        assert!(jpg.starts_with(&[0xFF, 0xD8]));
        let decoded = image::load_from_memory(&jpg).unwrap();
        assert_eq!(decoded.width(), 32);
    }
}
