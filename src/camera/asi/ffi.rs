//! Hand-written bindings for the vendored ZWO ASICamera2 SDK. Layouts are
//! copied from ASICamera2.h and verified live against the 120MM Mini —
//! keep field order and C types exactly as-is.
#![allow(dead_code)]

use std::ffi::{c_char, c_int, c_long};

#[repr(C)]
pub struct AsiCameraInfo {
    pub name: [c_char; 64],
    pub camera_id: c_int,
    pub max_height: c_long,
    pub max_width: c_long,
    pub is_color_cam: c_int,
    pub bayer_pattern: c_int,
    pub supported_bins: [c_int; 16],
    pub supported_video_format: [c_int; 8],
    pub pixel_size: f64,
    pub mechanical_shutter: c_int,
    pub st4_port: c_int,
    pub is_cooler_cam: c_int,
    pub is_usb3_host: c_int,
    pub is_usb3_camera: c_int,
    pub elec_per_adu: f32,
    pub bit_depth: c_int,
    pub is_trigger_cam: c_int,
    pub unused: [u8; 16],
}

#[repr(C)]
pub struct AsiControlCaps {
    pub name: [c_char; 64],
    pub description: [c_char; 128],
    pub max_value: c_long,
    pub min_value: c_long,
    pub default_value: c_long,
    pub is_auto_supported: c_int,
    pub is_writable: c_int,
    pub control_type: c_int,
    pub unused: [c_char; 32],
}

pub const ASI_GAIN: c_int = 0;
pub const ASI_EXPOSURE: c_int = 1;
pub const ASI_IMG_RAW8: c_int = 0;
pub const ASI_EXP_IDLE: c_int = 0;
pub const ASI_EXP_WORKING: c_int = 1;
pub const ASI_EXP_SUCCESS: c_int = 2;
pub const ASI_EXP_FAILED: c_int = 3;

/// ASI_ERROR_CODE names for diagnostics (header order).
pub fn err_name(code: c_int) -> &'static str {
    match code {
        0 => "ASI_SUCCESS",
        1 => "ASI_ERROR_INVALID_INDEX",
        2 => "ASI_ERROR_INVALID_ID",
        3 => "ASI_ERROR_INVALID_CONTROL_TYPE",
        4 => "ASI_ERROR_CAMERA_CLOSED",
        5 => "ASI_ERROR_CAMERA_REMOVED",
        6 => "ASI_ERROR_INVALID_PATH",
        7 => "ASI_ERROR_INVALID_FILEFORMAT",
        8 => "ASI_ERROR_INVALID_SIZE",
        9 => "ASI_ERROR_INVALID_IMGTYPE",
        10 => "ASI_ERROR_OUTOF_BOUNDARY",
        11 => "ASI_ERROR_TIMEOUT",
        12 => "ASI_ERROR_INVALID_SEQUENCE",
        13 => "ASI_ERROR_BUFFER_TOO_SMALL",
        14 => "ASI_ERROR_VIDEO_MODE_ACTIVE",
        15 => "ASI_ERROR_EXPOSURE_IN_PROGRESS",
        16 => "ASI_ERROR_GENERAL_ERROR",
        17 => "ASI_ERROR_INVALID_MODE",
        _ => "ASI_ERROR_UNKNOWN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_sizes_match_the_c_layout() {
        // Guards against accidental field reordering/type edits. On LP64
        // targets (macOS and Linux, x86_64/aarch64) `c_long` is 8 bytes, so
        // hand-computing the C layout (natural alignment, tail padding to
        // the largest member's alignment) gives:
        //   AsiCameraInfo: 64 (name) + 4 (camera_id) + 4 pad + 8 + 8
        //     (max_height/max_width) + 4 + 4 (is_color_cam/bayer_pattern)
        //     + 64 (supported_bins) + 32 (supported_video_format)
        //     + 8 (pixel_size, already 8-aligned) + 4*5 (mechanical_shutter
        //     .. is_usb3_camera) + 4 (elec_per_adu) + 4 + 4 (bit_depth,
        //     is_trigger_cam) + 16 (unused) = 248, already a multiple of 8.
        //   AsiControlCaps: 64 + 128 (name/description) + 8*3 (the three
        //     `long` fields, 8-aligned) + 4*3 (the three `int` fields)
        //     + 32 (unused) = 260, padded up to 264 (struct align 8).
        assert_eq!(std::mem::size_of::<AsiCameraInfo>(), 248);
        assert_eq!(std::mem::size_of::<AsiControlCaps>(), 264);
    }

    #[test]
    fn error_names_cover_the_header_range() {
        assert_eq!(err_name(0), "ASI_SUCCESS");
        assert_eq!(err_name(11), "ASI_ERROR_TIMEOUT");
        assert_eq!(err_name(99), "ASI_ERROR_UNKNOWN");
    }
}
