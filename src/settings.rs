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
    pub interval_sec: u64,
    pub capture_during_day: bool,
    // serde defaults let a config.toml written before these fields existed
    // load without resetting the rest of the settings.
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageSettings {
    pub mask_mode: MaskMode,
    pub crop: Option<CropRect>,
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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LensCalibration {
    pub cx: f64,
    pub cy: f64,
    pub radius_px: f64,
    pub rotation_deg: f64,
    pub flip: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayLayers {
    pub cardinal: bool,
    pub alt_az_grid: bool,
    pub ra_dec_grid: bool,
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
    pub text_fields: Vec<OverlayTextField>,
    pub bake_into_saved_frames: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingSettings {
    pub keogram: bool,
    pub startrails: bool,
    pub startrails_brightness_limit: f64,
    pub timelapse: bool,
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
#[allow(dead_code)]
pub struct Settings {
    pub camera: CameraSettings,
    pub image: ImageSettings,
    pub location: LocationSettings,
    pub sensor: SensorSettings,
    pub overlay: OverlaySettings,
    pub processing: ProcessingSettings,
    pub storage: StorageSettings,
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
                interval_sec: 60,
                capture_during_day: false,
                capture_width: 1640, // imx219 full-FoV 2x2 binned mode (2 MP)
                capture_height: 1232,
            },
            image: ImageSettings {
                mask_mode: MaskMode::None,
                crop: None,
            },
            location: LocationSettings {
                latitude_deg: 50.45,
                longitude_deg: 30.52,
            },
            sensor: SensorSettings { enabled: true },
            overlay: OverlaySettings {
                calibration: LensCalibration {
                    cx: 640.0,
                    cy: 480.0,
                    radius_px: 620.0,
                    rotation_deg: 0.0,
                    flip: false,
                },
                layers: OverlayLayers {
                    cardinal: true,
                    alt_az_grid: true,
                    ra_dec_grid: true,
                },
                grid_opacity: 0.45,
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
                timelapse: true,
                timelapse_fps: 25,
                timelapse_extra_args: String::new(),
            },
            storage: StorageSettings {
                frames_retention_days: 14,
                artifacts_retention_days: 60,
            },
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
        c.interval_sec = c.interval_sec.max(1);
        c.target_brightness = c.target_brightness.clamp(1.0, 254.0);
        c.capture_width = c.capture_width.max(8);
        c.capture_height = c.capture_height.max(2);

        self.overlay.grid_opacity = self.overlay.grid_opacity.clamp(0.0, 1.0);

        let p = &mut self.processing;
        p.timelapse_fps = p.timelapse_fps.clamp(1, 120);
        p.startrails_brightness_limit = p.startrails_brightness_limit.clamp(0.0, 255.0);

        self.storage.frames_retention_days = self.storage.frames_retention_days.max(1);
        self.storage.artifacts_retention_days = self.storage.artifacts_retention_days.max(1);
    }
}

/// What lives in config.toml: settings + fields the API must never return.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct ConfigFile {
    pub version: u32,
    pub password_hash: String,
    pub settings: Settings,
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
        assert_eq!(s.camera.interval_sec, 60);
        assert_eq!(s.image.mask_mode, MaskMode::None);
        assert!(s.image.crop.is_none());
        assert!(s.sensor.enabled);
        assert_eq!(s.overlay.grid_opacity, 0.45);
        assert_eq!(s.overlay.calibration.radius_px, 620.0);
        assert_eq!(s.overlay.text_fields.len(), 2);
        assert_eq!(s.storage.frames_retention_days, 14);
        assert_eq!(
            (s.camera.capture_width, s.camera.capture_height),
            (1640, 1232)
        );
    }

    #[test]
    fn wire_json_is_camel_case_and_matches_the_ts_contract() {
        let s = Settings::default();
        let v: serde_json::Value = serde_json::to_value(&s).unwrap();
        assert_eq!(v["camera"]["driver"], "rpicam");
        assert!(v["camera"]["exposureUsMin"].is_number());
        assert_eq!(v["camera"]["captureWidth"], 1640);
        assert_eq!(v["camera"]["captureHeight"], 1232);
        assert_eq!(v["image"]["maskMode"], "none");
        assert_eq!(v["image"]["crop"], serde_json::Value::Null);
        assert_eq!(v["sensor"]["enabled"], true);
        assert_eq!(v["overlay"]["gridOpacity"], 0.45);
        assert_eq!(v["overlay"]["textFields"][1]["kind"], "exposure");
        assert_eq!(v["overlay"]["calibration"]["radiusPx"], 620.0);
        assert_eq!(v["storage"]["artifactsRetentionDays"], 60);
        assert_eq!(v["processing"]["timelapseExtraArgs"], "");
        // settings JSON must never leak the password hash
        assert!(v.get("passwordHash").is_none());
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
    fn sanitize_clamps_out_of_range_fields() {
        let mut s = Settings::default();
        s.camera.manual_gain = 0.0; // below a sane floor
        s.camera.gain_min = -5.0;
        s.camera.gain_max = 0.5; // below gain_min after its own clamp
        s.camera.manual_exposure_us = 0;
        s.camera.interval_sec = 0;
        s.camera.target_brightness = 999.0;
        s.camera.capture_width = 0;
        s.camera.capture_height = 1;
        s.overlay.grid_opacity = 5.0;
        s.processing.timelapse_fps = 0;
        s.processing.startrails_brightness_limit = 900.0;
        s.storage.frames_retention_days = 0;
        s.storage.artifacts_retention_days = 0;
        s.sanitize();
        assert!(s.camera.gain_min >= 0.0);
        assert!(s.camera.gain_max >= s.camera.gain_min);
        assert!(
            s.camera.manual_gain >= s.camera.gain_min && s.camera.manual_gain <= s.camera.gain_max
        );
        assert!(s.camera.manual_exposure_us >= 1);
        assert!(s.camera.interval_sec >= 1);
        assert!(s.camera.target_brightness >= 1.0 && s.camera.target_brightness <= 254.0);
        assert!(s.camera.capture_width >= 8 && s.camera.capture_height >= 2);
        assert!(s.overlay.grid_opacity >= 0.0 && s.overlay.grid_opacity <= 1.0);
        assert!(s.processing.timelapse_fps >= 1);
        assert!(
            s.processing.startrails_brightness_limit >= 0.0
                && s.processing.startrails_brightness_limit <= 255.0
        );
        assert!(s.storage.frames_retention_days >= 1 && s.storage.artifacts_retention_days >= 1);
    }

    #[test]
    fn sanitize_leaves_valid_settings_unchanged() {
        let mut s = Settings::default();
        let before = s.clone();
        s.sanitize();
        assert_eq!(s, before);
    }
}
