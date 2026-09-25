use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

// ── wire/settings types (mirror frontend/src/api/types.ts) ─────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CameraDriver {
    Asi,
    Rpicam,
    Mock,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraSettings {
    pub driver: CameraDriver,
    pub auto_exposure: bool,
    pub target_brightness: f64,
    pub exposure_us_min: u64,
    pub exposure_us_max: u64,
    pub gain_min: f64,
    pub gain_max: f64,
    pub manual_exposure_us: u64,
    pub manual_gain: f64,
    // serde defaults let a config.toml written before these fields existed
    // load without resetting the rest of the settings.
    /// 0 = continuous shooting: capture again immediately, no sleep.
    #[serde(default = "default_interval_sec_day")]
    pub interval_sec_day: u64,
    /// 0 = continuous shooting: capture again immediately, no sleep.
    #[serde(default = "default_interval_sec_night")]
    pub interval_sec_night: u64,
    pub capture_during_day: bool,
    #[serde(default = "default_capture_width")]
    pub capture_width: u32,
    #[serde(default = "default_capture_height")]
    pub capture_height: u32,
}

fn default_capture_width() -> u32 {
    1640
}

fn default_capture_height() -> u32 {
    1232
}

// Day frames need less density (short exposures, slow-changing sky) and only
// feed the optional day timelapse; night frames feed the keogram, star
// trails, and night timelapse, so a tighter cadence makes those noticeably
// smoother.
fn default_interval_sec_day() -> u64 {
    120
}

fn default_interval_sec_night() -> u64 {
    60
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MaskMode {
    Circle,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CropRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Caps on the metering mask. Mirrored in frontend/src/lib/editorMath.ts —
/// the editor must not be able to build something sanitize would throw away.
pub const MAX_METER_REGIONS: usize = 8;
pub const MAX_METER_POINTS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeterPoint {
    /// 0..1 fraction of the raw sensor frame, NOT pixels — so the mask
    /// survives a capture-resolution change.
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeterPolygon {
    pub points: Vec<MeterPoint>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageSettings {
    pub mask_mode: MaskMode,
    /// Manual mask circle in sensor-frame pixels — set by hand in the
    /// editor, deliberately independent of the lens calibration.
    #[serde(default = "default_mask_center_x_px")]
    pub mask_center_x_px: f64,
    #[serde(default = "default_mask_center_y_px")]
    pub mask_center_y_px: f64,
    #[serde(default = "default_mask_radius_px")]
    pub mask_radius_px: f64,
    pub crop: Option<CropRect>,
    /// Regions auto-exposure is allowed to measure. Empty = whole frame,
    /// which is what every config written before this feature means.
    #[serde(default)]
    pub meter_polygons: Vec<MeterPolygon>,
}

fn default_mask_center_x_px() -> f64 {
    640.0
}

fn default_mask_center_y_px() -> f64 {
    480.0
}

fn default_mask_radius_px() -> f64 {
    620.0
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocationSettings {
    pub latitude_deg: f64,
    pub longitude_deg: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorSettings {
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LensType {
    Fisheye,
    Rectilinear,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LensCalibration {
    // serde defaults migrate a config.toml written by the old pixel-based
    // model (cx/cy/radiusPx are simply unknown keys and ignored).
    #[serde(default = "default_lens_type")]
    pub lens_type: LensType,
    #[serde(default = "default_focal_length_mm")]
    pub focal_length_mm: f64,
    /// Native sensor pixel size (datasheet); binning is derived per frame.
    #[serde(default = "default_pixel_size_um")]
    pub pixel_size_um: f64,
    #[serde(default)]
    pub pointing_az_deg: f64,
    #[serde(default = "default_pointing_alt_deg")]
    pub pointing_alt_deg: f64,
    /// Rotation about the optical axis; migrates the old `rotationDeg`.
    #[serde(default, alias = "rotationDeg")]
    pub roll_deg: f64,
    #[serde(default)]
    pub flip: bool,
    /// Optical center minus image center (lens mounted off-axis).
    #[serde(default)]
    pub center_offset_x_px: f64,
    #[serde(default)]
    pub center_offset_y_px: f64,
}

fn default_lens_type() -> LensType {
    LensType::Fisheye
}

fn default_focal_length_mm() -> f64 {
    1.8
}

fn default_pixel_size_um() -> f64 {
    1.12
}

fn default_pointing_alt_deg() -> f64 {
    90.0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayLayers {
    pub cardinal: bool,
    pub alt_az_grid: bool,
    pub ra_dec_grid: bool,
    #[serde(default)]
    pub constellations: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextFieldKind {
    Time,
    Exposure,
    SensorTemp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayTextField {
    pub id: String,
    pub kind: TextFieldKind,
    pub x: f64,
    pub y: f64,
    pub font_size: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlaySettings {
    pub calibration: LensCalibration,
    pub layers: OverlayLayers,
    pub grid_opacity: f64,
    #[serde(default = "default_constellations_opacity")]
    pub constellations_opacity: f64,
    pub text_fields: Vec<OverlayTextField>,
    pub bake_into_saved_frames: bool,
}

fn default_constellations_opacity() -> f64 {
    0.55
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingSettings {
    pub keogram: bool,
    pub startrails: bool,
    pub startrails_brightness_limit: f64,
    /// Timelapse of daytime frames for the night. Independent of
    /// `timelapse_night` — either, both, or neither can be enabled.
    #[serde(default = "default_true")]
    pub timelapse_day: bool,
    /// Timelapse of nighttime frames for the night.
    #[serde(default = "default_true")]
    pub timelapse_night: bool,
    pub timelapse_fps: u32,
    /// Extra ffmpeg args appended before the output path, whitespace-split
    /// into argv (no shell). Empty by default.
    #[serde(default)]
    pub timelapse_extra_args: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageSettings {
    pub frames_retention_days: u32,
    pub artifacts_retention_days: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RtspSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_rtsp_port")]
    pub port: u16,
    #[serde(default = "default_rtsp_fps")]
    pub fps: u32,
    #[serde(default = "default_true")]
    pub overlay: bool,
    /// 0 = native capture width.
    #[serde(default)]
    pub output_width: u32,
    #[serde(default = "default_rtsp_bitrate_kbps")]
    pub bitrate_kbps: u32,
    #[serde(default = "default_true")]
    pub auth_enabled: bool,
    /// Extra ffmpeg args appended before the output, whitespace-split into
    /// argv (no shell) -- same convention as `timelapse_extra_args`.
    #[serde(default)]
    pub extra_args: String,
}

fn default_rtsp_port() -> u16 {
    8554
}

fn default_rtsp_fps() -> u32 {
    5
}

fn default_rtsp_bitrate_kbps() -> u32 {
    2000
}

fn default_rtsp_settings() -> RtspSettings {
    RtspSettings {
        enabled: false,
        port: default_rtsp_port(),
        fps: default_rtsp_fps(),
        overlay: true,
        output_width: 0,
        bitrate_kbps: default_rtsp_bitrate_kbps(),
        auth_enabled: true,
        extra_args: String::new(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DarkFrameSettings {
    pub enabled: bool,
    pub min_gain_to_apply: f64,
    pub min_exposure_us_to_apply: u64,
}

fn default_dark_frame_settings() -> DarkFrameSettings {
    DarkFrameSettings {
        enabled: false,
        min_gain_to_apply: 15.0,
        min_exposure_us_to_apply: 10_000_000,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct Settings {
    pub camera: CameraSettings,
    pub image: ImageSettings,
    pub location: LocationSettings,
    pub sensor: SensorSettings,
    pub overlay: OverlaySettings,
    pub processing: ProcessingSettings,
    pub storage: StorageSettings,
    #[serde(default = "default_dark_frame_settings")]
    pub darks: DarkFrameSettings,
    #[serde(default = "default_rtsp_settings")]
    pub rtsp: RtspSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            camera: CameraSettings {
                driver: CameraDriver::Rpicam,
                auto_exposure: true,
                target_brightness: 100.0,
                exposure_us_min: 32,
                exposure_us_max: 10_000_000, // imx219 tops out around ~11.7 s
                gain_min: 1.0,
                gain_max: 16.0,
                manual_exposure_us: 5_000_000,
                manual_gain: 8.0,
                interval_sec_day: default_interval_sec_day(),
                interval_sec_night: default_interval_sec_night(),
                capture_during_day: false,
                capture_width: 1640, // imx219 full-FoV 2x2 binned mode (2 MP)
                capture_height: 1232,
            },
            image: ImageSettings {
                mask_mode: MaskMode::None,
                mask_center_x_px: 640.0,
                mask_center_y_px: 480.0,
                mask_radius_px: 620.0,
                crop: None,
                meter_polygons: Vec::new(),
            },
            location: LocationSettings {
                latitude_deg: 50.45,
                longitude_deg: 30.52,
            },
            sensor: SensorSettings { enabled: true },
            overlay: OverlaySettings {
                calibration: LensCalibration {
                    lens_type: LensType::Fisheye,
                    focal_length_mm: 1.8, // the M12 lens on the imx219
                    pixel_size_um: 1.12,  // imx219 native pixel
                    pointing_az_deg: 0.0,
                    pointing_alt_deg: 90.0,
                    roll_deg: 0.0,
                    flip: false,
                    center_offset_x_px: 0.0,
                    center_offset_y_px: 0.0,
                },
                layers: OverlayLayers {
                    cardinal: true,
                    alt_az_grid: true,
                    ra_dec_grid: true,
                    constellations: false,
                },
                grid_opacity: 0.45,
                constellations_opacity: default_constellations_opacity(),
                text_fields: vec![
                    OverlayTextField {
                        id: "time".into(),
                        kind: TextFieldKind::Time,
                        x: 24.0,
                        y: 40.0,
                        font_size: 24.0,
                    },
                    OverlayTextField {
                        id: "exposure".into(),
                        kind: TextFieldKind::Exposure,
                        x: 24.0,
                        y: 72.0,
                        font_size: 18.0,
                    },
                ],
                bake_into_saved_frames: false,
            },
            processing: ProcessingSettings {
                keogram: true,
                startrails: true,
                startrails_brightness_limit: 35.0,
                timelapse_day: true,
                timelapse_night: true,
                timelapse_fps: 25,
                timelapse_extra_args: String::new(),
            },
            storage: StorageSettings {
                frames_retention_days: 14,
                artifacts_retention_days: 60,
            },
            darks: DarkFrameSettings {
                enabled: false,
                min_gain_to_apply: 15.0,
                min_exposure_us_to_apply: 10_000_000,
            },
            rtsp: default_rtsp_settings(),
        }
    }
}

impl Settings {
    /// Clamp every numeric field to a safe range so a malformed or hostile
    /// PUT can't persist nonsense (gain below the sensor floor, zero retention,
    /// opacity > 1, ...). Called on the incoming settings before they are saved.
    pub fn sanitize(&mut self) {
        let c = &mut self.camera;
        c.gain_min = c.gain_min.max(0.0);
        c.gain_max = c.gain_max.max(c.gain_min);
        c.manual_gain = c.manual_gain.clamp(c.gain_min, c.gain_max);
        c.exposure_us_min = c.exposure_us_min.max(1);
        c.exposure_us_max = c.exposure_us_max.max(c.exposure_us_min);
        c.manual_exposure_us = c
            .manual_exposure_us
            .clamp(c.exposure_us_min, c.exposure_us_max);
        c.target_brightness = c.target_brightness.clamp(1.0, 254.0);
        c.capture_width = c.capture_width.max(8);
        c.capture_height = c.capture_height.max(2);

        self.overlay.grid_opacity = self.overlay.grid_opacity.clamp(0.0, 1.0);
        self.overlay.constellations_opacity = self.overlay.constellations_opacity.clamp(0.0, 1.0);
        self.image.mask_center_x_px = self.image.mask_center_x_px.clamp(-10_000.0, 10_000.0);
        self.image.mask_center_y_px = self.image.mask_center_y_px.clamp(-10_000.0, 10_000.0);
        self.image.mask_radius_px = self.image.mask_radius_px.clamp(20.0, 10_000.0);

        let cal = &mut self.overlay.calibration;
        cal.focal_length_mm = cal.focal_length_mm.clamp(0.1, 100.0);
        cal.pixel_size_um = cal.pixel_size_um.clamp(0.5, 50.0);
        cal.pointing_alt_deg = cal.pointing_alt_deg.clamp(-90.0, 90.0);
        cal.pointing_az_deg = cal.pointing_az_deg.rem_euclid(360.0);
        cal.roll_deg = cal.roll_deg.rem_euclid(360.0);
        cal.center_offset_x_px = cal.center_offset_x_px.clamp(-5000.0, 5000.0);
        cal.center_offset_y_px = cal.center_offset_y_px.clamp(-5000.0, 5000.0);

        let p = &mut self.processing;
        p.timelapse_fps = p.timelapse_fps.clamp(1, 120);
        p.startrails_brightness_limit = p.startrails_brightness_limit.clamp(0.0, 255.0);

        self.storage.frames_retention_days = self.storage.frames_retention_days.max(1);
        self.storage.artifacts_retention_days = self.storage.artifacts_retention_days.max(1);

        self.darks.min_gain_to_apply = self.darks.min_gain_to_apply.max(0.0);

        let r = &mut self.rtsp;
        if r.port == 0 {
            r.port = default_rtsp_port();
        }
        r.fps = r.fps.clamp(1, 30);
        r.bitrate_kbps = r.bitrate_kbps.clamp(200, 20_000);
        if r.output_width != 0 {
            r.output_width = r.output_width.max(160);
        }

        // Metering mask: finite, in range, non-degenerate, bounded. Order
        // matters — truncate points first so a polygon cut down below three
        // points is then dropped by the retain.
        let img = &mut self.image;
        img.meter_polygons.truncate(MAX_METER_REGIONS);
        for poly in &mut img.meter_polygons {
            poly.points.truncate(MAX_METER_POINTS);
            for p in &mut poly.points {
                // Map non-finite BEFORE clamping: clamp passes NaN through.
                p.x = if p.x.is_finite() {
                    p.x.clamp(0.0, 1.0)
                } else {
                    0.0
                };
                p.y = if p.y.is_finite() {
                    p.y.clamp(0.0, 1.0)
                } else {
                    0.0
                };
            }
        }
        img.meter_polygons.retain(|p| p.points.len() >= 3);
    }
}

/// What lives in config.toml: settings + fields the API must never return.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct ConfigFile {
    pub version: u32,
    pub password_hash: String,
    #[serde(default = "default_rtsp_username")]
    pub rtsp_username: String,
    #[serde(default = "default_rtsp_password_ha1")]
    pub rtsp_password_ha1: String,
    pub settings: Settings,
}

fn default_rtsp_username() -> String {
    "admin".to_string()
}

fn default_rtsp_password_ha1() -> String {
    crate::rtsp::digest::ha1(
        &default_rtsp_username(),
        crate::rtsp::digest::REALM,
        crate::auth::DEFAULT_PASSWORD,
    )
}

#[allow(dead_code)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    #[allow(dead_code)]
    pub fn new(data_dir: &Path) -> Self {
        SettingsStore {
            path: data_dir.join("config.toml"),
        }
    }

    /// Load config; on a corrupt file back it up and start from defaults;
    /// on a missing file create defaults with the given password hash.
    #[allow(dead_code)]
    pub fn load_or_create(&self, default_password_hash: &str) -> anyhow::Result<ConfigFile> {
        match fs::read_to_string(&self.path) {
            Ok(raw) => match toml::from_str::<ConfigFile>(&raw) {
                Ok(cfg) => Ok(cfg),
                Err(e) => {
                    let backup = self
                        .path
                        .with_extension(format!("toml.bak-{}", chrono::Utc::now().timestamp()));
                    tracing::error!("config.toml is corrupt ({e}); backing up to {backup:?}");
                    fs::rename(&self.path, &backup).context("backing up corrupt config")?;
                    let cfg = self.default_config(default_password_hash);
                    self.save(&cfg)?;
                    Ok(cfg)
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let cfg = self.default_config(default_password_hash);
                self.save(&cfg)?;
                Ok(cfg)
            }
            Err(e) => Err(e).context("reading config.toml"),
        }
    }

    #[allow(dead_code)]
    fn default_config(&self, password_hash: &str) -> ConfigFile {
        ConfigFile {
            version: 1,
            password_hash: password_hash.to_string(),
            rtsp_username: default_rtsp_username(),
            rtsp_password_ha1: default_rtsp_password_ha1(),
            settings: Settings::default(),
        }
    }

    /// Atomic write: tmp file + rename.
    #[allow(dead_code)]
    pub fn save(&self, cfg: &ConfigFile) -> anyhow::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).context("creating data dir")?;
        }
        let tmp = self.path.with_extension("toml.tmp");
        fs::write(
            &tmp,
            toml::to_string_pretty(cfg).context("serializing config")?,
        )
        .context("writing temp config")?;
        fs::rename(&tmp, &self.path).context("renaming temp config")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn defaults_match_the_phase1_mock() {
        let s = Settings::default();
        assert_eq!(s.location.latitude_deg, 50.45);
        assert_eq!(s.location.longitude_deg, 30.52);
        assert_eq!(s.camera.driver, CameraDriver::Rpicam);
        assert!(s.camera.auto_exposure);
        assert_eq!(s.camera.interval_sec_day, 120);
        assert_eq!(s.camera.interval_sec_night, 60);
        assert_eq!(s.image.mask_mode, MaskMode::None);
        assert_eq!(s.image.mask_radius_px, 620.0);
        assert_eq!(
            (s.image.mask_center_x_px, s.image.mask_center_y_px),
            (640.0, 480.0)
        );
        assert!(s.image.crop.is_none());
        assert!(s.sensor.enabled);
        assert_eq!(s.overlay.grid_opacity, 0.45);
        assert_eq!(s.overlay.constellations_opacity, 0.55);
        assert_eq!(s.overlay.calibration.lens_type, LensType::Fisheye);
        assert_eq!(s.overlay.calibration.focal_length_mm, 1.8);
        assert_eq!(s.overlay.calibration.pixel_size_um, 1.12);
        assert_eq!(s.overlay.calibration.pointing_alt_deg, 90.0);
        assert_eq!(s.overlay.calibration.roll_deg, 0.0);
        assert_eq!(s.overlay.text_fields.len(), 2);
        assert_eq!(s.storage.frames_retention_days, 14);
        assert_eq!(
            (s.camera.capture_width, s.camera.capture_height),
            (1640, 1232)
        );
    }

    #[test]
    fn darks_defaults_are_disabled_with_documented_thresholds() {
        let s = Settings::default();
        assert!(!s.darks.enabled);
        assert_eq!(s.darks.min_gain_to_apply, 15.0);
        assert_eq!(s.darks.min_exposure_us_to_apply, 10_000_000);
    }

    #[test]
    fn wire_json_is_camel_case_and_matches_the_ts_contract() {
        let s = Settings::default();
        let v: serde_json::Value = serde_json::to_value(&s).unwrap();
        assert_eq!(v["camera"]["driver"], "rpicam");
        assert!(v["camera"]["exposureUsMin"].is_number());
        assert_eq!(v["camera"]["captureWidth"], 1640);
        assert_eq!(v["camera"]["captureHeight"], 1232);
        assert_eq!(v["camera"]["intervalSecDay"], 120);
        assert_eq!(v["camera"]["intervalSecNight"], 60);
        assert_eq!(v["image"]["maskMode"], "none");
        assert_eq!(v["image"]["maskRadiusPx"], 620.0);
        assert_eq!(v["image"]["maskCenterXPx"], 640.0);
        assert_eq!(v["image"]["crop"], serde_json::Value::Null);
        assert_eq!(v["sensor"]["enabled"], true);
        assert_eq!(v["overlay"]["gridOpacity"], 0.45);
        assert_eq!(v["overlay"]["constellationsOpacity"], 0.55);
        assert_eq!(v["overlay"]["textFields"][1]["kind"], "exposure");
        assert_eq!(v["overlay"]["calibration"]["lensType"], "fisheye");
        assert_eq!(v["overlay"]["calibration"]["focalLengthMm"], 1.8);
        assert_eq!(v["overlay"]["calibration"]["pixelSizeUm"], 1.12);
        assert_eq!(v["overlay"]["calibration"]["pointingAltDeg"], 90.0);
        assert!(v["overlay"]["calibration"].get("radiusPx").is_none());
        assert_eq!(v["storage"]["artifactsRetentionDays"], 60);
        assert_eq!(v["processing"]["timelapseExtraArgs"], "");
        assert_eq!(v["processing"]["timelapseDay"], true);
        assert_eq!(v["processing"]["timelapseNight"], true);
        assert_eq!(v["darks"]["enabled"], false);
        assert_eq!(v["darks"]["minGainToApply"], 15.0);
        assert_eq!(v["darks"]["minExposureUsToApply"], 10_000_000);
        // settings JSON must never leak the password hash
        assert!(v.get("passwordHash").is_none());
    }

    #[test]
    fn config_without_mask_circle_fields_loads_with_defaults() {
        // A config.toml written before the manual mask circle existed must
        // still load, gaining the default center/radius.
        let dir = TempDir::new().unwrap();
        let store = SettingsStore::new(dir.path());
        let mut cfg = store.load_or_create("h").unwrap();
        cfg.settings.image.mask_mode = MaskMode::Circle;
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        assert!(toml_str.contains("maskRadiusPx"));
        let older: String = toml_str
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                !t.starts_with("maskRadiusPx")
                    && !t.starts_with("maskCenterXPx")
                    && !t.starts_with("maskCenterYPx")
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!older.contains("maskRadiusPx"));
        std::fs::write(dir.path().join("config.toml"), older).unwrap();

        let loaded = store.load_or_create("h").unwrap();
        assert_eq!(loaded.settings.image.mask_mode, MaskMode::Circle); // preserved
        assert_eq!(loaded.settings.image.mask_radius_px, 620.0);
        assert_eq!(
            (
                loaded.settings.image.mask_center_x_px,
                loaded.settings.image.mask_center_y_px
            ),
            (640.0, 480.0)
        );
    }

    #[test]
    fn config_without_capture_resolution_loads_with_defaults() {
        // A config.toml written before capture_width/height existed must still
        // load, gaining the default resolution rather than failing to parse.
        let dir = TempDir::new().unwrap();
        let store = SettingsStore::new(dir.path());
        let mut cfg = store.load_or_create("h").unwrap();
        cfg.settings.location.latitude_deg = 12.34;
        // Serialize, then strip the two new keys to simulate an older file.
        // Keys are camelCase in the TOML (serde rename_all), not snake_case.
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        assert!(
            toml_str.contains("captureWidth"),
            "expected camelCase key in TOML"
        );
        let older: String = toml_str
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                !t.starts_with("captureWidth") && !t.starts_with("captureHeight")
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !older.contains("captureWidth"),
            "keys must be stripped for the test to be meaningful"
        );
        std::fs::write(dir.path().join("config.toml"), older).unwrap();

        let loaded = store.load_or_create("h").unwrap();
        assert_eq!(loaded.settings.location.latitude_deg, 12.34); // preserved
        assert_eq!(
            (
                loaded.settings.camera.capture_width,
                loaded.settings.camera.capture_height
            ),
            (1640, 1232)
        );
    }

    #[test]
    fn config_without_timelapse_extra_args_loads_with_default() {
        let dir = TempDir::new().unwrap();
        let store = SettingsStore::new(dir.path());
        let mut cfg = store.load_or_create("h").unwrap();
        cfg.settings.processing.timelapse_fps = 30;
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        assert!(toml_str.contains("timelapseExtraArgs"));
        let older: String = toml_str
            .lines()
            .filter(|l| !l.trim_start().starts_with("timelapseExtraArgs"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!older.contains("timelapseExtraArgs"));
        std::fs::write(dir.path().join("config.toml"), older).unwrap();

        let loaded = store.load_or_create("h").unwrap();
        assert_eq!(loaded.settings.processing.timelapse_fps, 30); // preserved
        assert_eq!(loaded.settings.processing.timelapse_extra_args, "");
    }

    #[test]
    fn config_without_the_day_night_timelapse_split_loads_with_both_defaulted_true() {
        // A config.toml written before the day/night timelapse split (no
        // `timelapseDay`/`timelapseNight` keys — an old `timelapse` key, if
        // present, is simply an unrecognized extra key and ignored) must
        // still load, with both new flags defaulting to true.
        let dir = TempDir::new().unwrap();
        let store = SettingsStore::new(dir.path());
        let cfg = store.load_or_create("h").unwrap();
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        assert!(toml_str.contains("timelapseDay"));
        let older: String = toml_str
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                !t.starts_with("timelapseDay") && !t.starts_with("timelapseNight")
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!older.contains("timelapseDay"));
        std::fs::write(dir.path().join("config.toml"), older).unwrap();

        let loaded = store.load_or_create("h").unwrap();
        assert!(loaded.settings.processing.timelapse_day);
        assert!(loaded.settings.processing.timelapse_night);
    }

    #[test]
    fn config_without_the_day_night_interval_split_loads_with_defaults() {
        // A config.toml written before the day/night interval split (an old
        // `intervalSec` key, if present, is simply an unrecognized extra key
        // and ignored) must still load, with both new fields defaulted.
        let dir = TempDir::new().unwrap();
        let store = SettingsStore::new(dir.path());
        let mut cfg = store.load_or_create("h").unwrap();
        cfg.settings.location.latitude_deg = 41.9;
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        assert!(toml_str.contains("intervalSecDay"));
        let older: String = toml_str
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                !t.starts_with("intervalSecDay") && !t.starts_with("intervalSecNight")
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!older.contains("intervalSecDay"));
        std::fs::write(dir.path().join("config.toml"), older).unwrap();

        let loaded = store.load_or_create("h").unwrap();
        assert_eq!(loaded.settings.location.latitude_deg, 41.9); // preserved
        assert_eq!(loaded.settings.camera.interval_sec_day, 120);
        assert_eq!(loaded.settings.camera.interval_sec_night, 60);
    }

    #[test]
    fn config_without_darks_section_loads_with_default() {
        let dir = TempDir::new().unwrap();
        let store = SettingsStore::new(dir.path());
        let mut cfg = store.load_or_create("h").unwrap();
        cfg.settings.location.latitude_deg = 41.9;
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        assert!(toml_str.contains("[settings.darks]"));
        // Strip the whole [settings.darks] table (its header line through the
        // next table header) to simulate a config.toml written before this
        // feature existed.
        let mut older = String::new();
        let mut skipping = false;
        for line in toml_str.lines() {
            if line.trim_start() == "[settings.darks]" {
                skipping = true;
                continue;
            }
            if skipping && line.starts_with('[') {
                skipping = false;
            }
            if skipping {
                continue;
            }
            older.push_str(line);
            older.push('\n');
        }
        assert!(!older.contains("[settings.darks]"));
        std::fs::write(dir.path().join("config.toml"), older).unwrap();

        let loaded = store.load_or_create("h").unwrap();
        assert_eq!(loaded.settings.location.latitude_deg, 41.9); // preserved
        assert!(!loaded.settings.darks.enabled);
        assert_eq!(loaded.settings.darks.min_gain_to_apply, 15.0);
        assert_eq!(loaded.settings.darks.min_exposure_us_to_apply, 10_000_000);
    }

    #[test]
    fn config_without_rtsp_section_loads_with_default() {
        let dir = TempDir::new().unwrap();
        let store = SettingsStore::new(dir.path());
        let mut cfg = store.load_or_create("h").unwrap();
        cfg.settings.location.latitude_deg = 41.9;
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        assert!(toml_str.contains("[settings.rtsp]"));
        let mut older = String::new();
        let mut skipping = false;
        for line in toml_str.lines() {
            if line.trim_start() == "[settings.rtsp]" {
                skipping = true;
                continue;
            }
            if skipping && line.starts_with('[') {
                skipping = false;
            }
            if skipping {
                continue;
            }
            older.push_str(line);
            older.push('\n');
        }
        assert!(!older.contains("[settings.rtsp]"));
        std::fs::write(dir.path().join("config.toml"), older).unwrap();

        let loaded = store.load_or_create("h").unwrap();
        assert_eq!(loaded.settings.location.latitude_deg, 41.9); // preserved
        assert!(!loaded.settings.rtsp.enabled);
        assert_eq!(loaded.settings.rtsp.port, 8554);
        assert_eq!(loaded.settings.rtsp.fps, 5);
        assert!(loaded.settings.rtsp.overlay);
        assert!(loaded.settings.rtsp.auth_enabled);
    }

    #[test]
    fn sanitize_clamps_rtsp_fields() {
        let mut s = Settings::default();
        s.rtsp.port = 0;
        s.rtsp.fps = 999;
        s.rtsp.bitrate_kbps = 1;
        s.rtsp.output_width = 1;
        s.sanitize();
        assert_eq!(s.rtsp.port, 8554);
        assert_eq!(s.rtsp.fps, 30);
        assert_eq!(s.rtsp.bitrate_kbps, 200);
        assert_eq!(s.rtsp.output_width, 160);
    }

    #[test]
    fn store_roundtrips_and_creates_defaults() {
        let dir = TempDir::new().unwrap();
        let store = SettingsStore::new(dir.path());
        let mut cfg = store.load_or_create("test-hash").unwrap();
        assert_eq!(cfg.version, 1);
        assert_eq!(cfg.password_hash, "test-hash");
        cfg.settings.location.latitude_deg = 48.85;
        store.save(&cfg).unwrap();
        let again = store.load_or_create("other").unwrap();
        assert_eq!(again.settings.location.latitude_deg, 48.85);
        assert_eq!(again.password_hash, "test-hash"); // not recreated
    }

    #[test]
    fn corrupt_config_is_backed_up_and_replaced_with_defaults() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("config.toml"), "not [valid toml").unwrap();
        let store = SettingsStore::new(dir.path());
        let cfg = store.load_or_create("h").unwrap();
        assert_eq!(cfg.settings, Settings::default());
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("config.toml.bak")
            })
            .collect();
        assert_eq!(backups.len(), 1);
    }

    #[test]
    fn interval_zero_survives_sanitize_and_round_trips() {
        // 0 = continuous shooting: sanitize must not clamp it away, and it
        // must survive a TOML save/load cycle.
        let mut s = Settings::default();
        s.camera.interval_sec_day = 0;
        s.camera.interval_sec_night = 0;
        s.sanitize();
        assert_eq!(s.camera.interval_sec_day, 0);
        assert_eq!(s.camera.interval_sec_night, 0);

        let dir = TempDir::new().unwrap();
        let store = SettingsStore::new(dir.path());
        let mut cfg = store.load_or_create("h").unwrap();
        cfg.settings.camera.interval_sec_day = 0;
        cfg.settings.camera.interval_sec_night = 0;
        store.save(&cfg).unwrap();
        let loaded = store.load_or_create("h").unwrap();
        assert_eq!(loaded.settings.camera.interval_sec_day, 0);
        assert_eq!(loaded.settings.camera.interval_sec_night, 0);
    }

    #[test]
    fn sanitize_clamps_out_of_range_fields() {
        let mut s = Settings::default();
        s.camera.manual_gain = 0.0; // below a sane floor
        s.camera.gain_min = -5.0;
        s.camera.gain_max = 0.5; // below gain_min after its own clamp
        s.camera.manual_exposure_us = 0;
        s.camera.target_brightness = 999.0;
        s.camera.capture_width = 0;
        s.camera.capture_height = 1;
        s.overlay.grid_opacity = 5.0;
        s.overlay.constellations_opacity = -5.0;
        s.image.mask_radius_px = 5.0;
        s.image.mask_center_x_px = 99_999.0;
        s.processing.timelapse_fps = 0;
        s.processing.startrails_brightness_limit = 900.0;
        s.storage.frames_retention_days = 0;
        s.storage.artifacts_retention_days = 0;
        s.darks.min_gain_to_apply = -5.0;
        s.sanitize();
        assert!(s.camera.gain_min >= 0.0);
        assert!(s.camera.gain_max >= s.camera.gain_min);
        assert!(
            s.camera.manual_gain >= s.camera.gain_min && s.camera.manual_gain <= s.camera.gain_max
        );
        assert!(s.camera.manual_exposure_us >= 1);
        assert!(s.camera.target_brightness >= 1.0 && s.camera.target_brightness <= 254.0);
        assert!(s.camera.capture_width >= 8 && s.camera.capture_height >= 2);
        assert!(s.overlay.grid_opacity >= 0.0 && s.overlay.grid_opacity <= 1.0);
        assert!(s.overlay.constellations_opacity >= 0.0 && s.overlay.constellations_opacity <= 1.0);
        assert_eq!(s.image.mask_radius_px, 20.0);
        assert_eq!(s.image.mask_center_x_px, 10_000.0);
        assert!(s.processing.timelapse_fps >= 1);
        assert!(
            s.processing.startrails_brightness_limit >= 0.0
                && s.processing.startrails_brightness_limit <= 255.0
        );
        assert!(s.storage.frames_retention_days >= 1 && s.storage.artifacts_retention_days >= 1);
        assert!(s.darks.min_gain_to_apply >= 0.0);
    }

    #[test]
    fn sanitize_leaves_valid_settings_unchanged() {
        let mut s = Settings::default();
        let before = s.clone();
        s.sanitize();
        assert_eq!(s, before);
    }

    #[test]
    fn old_pixel_calibration_migrates_with_roll_alias_and_defaults() {
        // A config.toml calibration table written by the old model: unknown
        // keys (cx/cy/radiusPx) are ignored, rotationDeg feeds roll_deg via
        // its serde alias, missing physical fields get defaults.
        let old = "cx = 640.0\ncy = 480.0\nradiusPx = 620.0\nrotationDeg = 33.0\nflip = true\n";
        let cal: LensCalibration = toml::from_str(old).unwrap();
        assert_eq!(cal.roll_deg, 33.0);
        assert!(cal.flip);
        assert_eq!(cal.lens_type, LensType::Fisheye);
        assert_eq!(cal.focal_length_mm, 1.8);
        assert_eq!(cal.pixel_size_um, 1.12);
        assert_eq!(cal.pointing_alt_deg, 90.0);
        assert_eq!(cal.pointing_az_deg, 0.0);
        assert_eq!((cal.center_offset_x_px, cal.center_offset_y_px), (0.0, 0.0));
    }

    #[test]
    fn old_overlay_layers_without_constellations_default_it_off() {
        // A config.toml written before the constellations layer existed.
        let old = "cardinal = true\naltAzGrid = true\nraDecGrid = false\n";
        let layers: OverlayLayers = toml::from_str(old).unwrap();
        assert!(layers.cardinal);
        assert!(layers.alt_az_grid);
        assert!(!layers.ra_dec_grid);
        assert!(!layers.constellations);
    }

    #[test]
    fn old_overlay_settings_without_constellations_opacity_get_the_documented_default() {
        // A config.toml written before per-layer constellation opacity existed.
        let old = "gridOpacity = 0.3\nbakeIntoSavedFrames = false\ntextFields = []\n\
                    [calibration]\nlensType = \"fisheye\"\nfocalLengthMm = 1.8\n\
                    pixelSizeUm = 1.12\npointingAzDeg = 0.0\npointingAltDeg = 90.0\n\
                    rollDeg = 0.0\nflip = false\ncenterOffsetXPx = 0.0\ncenterOffsetYPx = 0.0\n\
                    [layers]\ncardinal = true\naltAzGrid = true\nraDecGrid = true\n";
        let overlay: OverlaySettings = toml::from_str(old).unwrap();
        assert_eq!(overlay.grid_opacity, 0.3);
        assert_eq!(overlay.constellations_opacity, 0.55);
    }

    #[test]
    fn sanitize_clamps_calibration_fields() {
        let mut s = Settings::default();
        s.overlay.calibration.focal_length_mm = 0.0;
        s.overlay.calibration.pixel_size_um = 999.0;
        s.overlay.calibration.pointing_alt_deg = -400.0;
        s.overlay.calibration.pointing_az_deg = 725.0;
        s.overlay.calibration.roll_deg = -90.0;
        s.overlay.calibration.center_offset_x_px = 99_999.0;
        s.overlay.calibration.center_offset_y_px = -99_999.0;
        s.sanitize();
        let c = &s.overlay.calibration;
        assert_eq!(c.focal_length_mm, 0.1);
        assert_eq!(c.pixel_size_um, 50.0);
        assert_eq!(c.pointing_alt_deg, -90.0);
        assert_eq!(c.pointing_az_deg, 5.0);
        assert_eq!(c.roll_deg, 270.0);
        assert_eq!(
            (c.center_offset_x_px, c.center_offset_y_px),
            (5000.0, -5000.0)
        );
    }

    #[test]
    fn fresh_config_seeds_rtsp_credentials_matching_the_default_web_login() {
        let dir = TempDir::new().unwrap();
        let store = SettingsStore::new(dir.path());
        let cfg = store
            .load_or_create(&crate::auth::hash_password(crate::auth::DEFAULT_PASSWORD).unwrap())
            .unwrap();
        assert_eq!(cfg.rtsp_username, "admin");
        assert_eq!(
            cfg.rtsp_password_ha1,
            crate::rtsp::digest::ha1("admin", crate::rtsp::digest::REALM, "pa$$word!0")
        );
    }

    #[test]
    fn config_without_rtsp_credentials_loads_with_default() {
        let dir = TempDir::new().unwrap();
        let store = SettingsStore::new(dir.path());
        let cfg = store.load_or_create("h").unwrap();
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        let older: String = toml_str
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                !t.starts_with("rtspUsername") && !t.starts_with("rtspPasswordHa1")
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(dir.path().join("config.toml"), older).unwrap();

        let loaded = store.load_or_create("h").unwrap();
        assert_eq!(loaded.rtsp_username, "admin");
        assert_eq!(loaded.password_hash, "h"); // untouched by the rtsp defaulting
    }

    #[test]
    fn config_without_meter_polygons_loads_as_whole_frame() {
        // A fresh install meters the whole frame.
        assert!(Settings::default().image.meter_polygons.is_empty());

        // A config.toml written before this feature has no meterPolygons key.
        // Build that text by serializing a real config and deleting the key,
        // so the test keeps working as other settings sections grow.
        let dir = TempDir::new().unwrap();
        let store = SettingsStore::new(dir.path());
        let cfg = store.load_or_create("h").unwrap();
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        let older: String = toml_str
            .lines()
            .filter(|l| !l.trim_start().starts_with("meterPolygons"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!older.contains("meterPolygons"));

        let back: ConfigFile =
            toml::from_str(&older).expect("a config without the key must still load");
        assert!(back.settings.image.meter_polygons.is_empty());
    }

    #[test]
    fn sanitize_bounds_the_metering_mask() {
        let mut s = Settings::default();
        let pt = |x, y| crate::settings::MeterPoint { x, y };

        s.image.meter_polygons = vec![
            // out of range on both axes -> clamped into 0..1
            MeterPolygon {
                points: vec![pt(-3.0, 0.2), pt(4.0, 0.2), pt(0.5, 9.9)],
            },
            // only two points -> dropped entirely
            MeterPolygon {
                points: vec![pt(0.1, 0.1), pt(0.2, 0.2)],
            },
        ];
        s.sanitize();

        assert_eq!(
            s.image.meter_polygons.len(),
            1,
            "the 2-point polygon must be dropped"
        );
        for p in &s.image.meter_polygons[0].points {
            assert!(
                (0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y),
                "{p:?}"
            );
        }

        // Region cap: 9 valid polygons in, 8 out.
        let square = MeterPolygon {
            points: vec![pt(0.1, 0.1), pt(0.9, 0.1), pt(0.9, 0.9), pt(0.1, 0.9)],
        };
        s.image.meter_polygons = vec![square.clone(); 9];
        s.sanitize();
        assert_eq!(
            s.image.meter_polygons.len(),
            crate::settings::MAX_METER_REGIONS
        );

        // Point cap: 100 points in, 64 out.
        s.image.meter_polygons = vec![MeterPolygon {
            points: (0..100).map(|i| pt(i as f64 / 100.0, 0.5)).collect(),
        }];
        s.sanitize();
        assert_eq!(
            s.image.meter_polygons[0].points.len(),
            crate::settings::MAX_METER_POINTS
        );
    }

    #[test]
    fn sanitize_replaces_non_finite_mask_coordinates() {
        // A hand-edited config.toml or a bad API client can deliver nan/inf.
        // f64::clamp returns NaN for a NaN input, so clamping alone would let it
        // straight through into rasterisation, where every comparison is false
        // and the mask silently covers nothing.
        let mut s = Settings::default();
        s.image.meter_polygons = vec![MeterPolygon {
            points: vec![
                crate::settings::MeterPoint {
                    x: f64::NAN,
                    y: 0.2,
                },
                crate::settings::MeterPoint {
                    x: 0.8,
                    y: f64::INFINITY,
                },
                crate::settings::MeterPoint {
                    x: f64::NEG_INFINITY,
                    y: 0.9,
                },
            ],
        }];
        s.sanitize();

        let pts = &s.image.meter_polygons[0].points;
        assert_eq!(pts.len(), 3, "the polygon itself must survive");
        for p in pts {
            assert!(
                p.x.is_finite() && p.y.is_finite(),
                "non-finite survived: {p:?}"
            );
            assert!(
                (0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y),
                "{p:?}"
            );
        }
    }
}
