// Only exercised by the Linux I2C path below; unused on other build targets.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub mod bme280;

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorState {
    Disabled,
    NotDetected,
    Ok,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorReading {
    pub temperature_c: f64,
    pub pressure_hpa: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub humidity_pct: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorStatus {
    pub state: SensorState,
    pub reading: Option<SensorReading>, // null on the wire when missing
}

/// Read the BME280/BMP280. Disabled in settings wins; then probe I2C.
pub fn read_sensor(enabled: bool) -> SensorStatus {
    if !enabled {
        return SensorStatus {
            state: SensorState::Disabled,
            reading: None,
        };
    }
    match probe_and_read() {
        Some(reading) => SensorStatus {
            state: SensorState::Ok,
            reading: Some(reading),
        },
        None => SensorStatus {
            state: SensorState::NotDetected,
            reading: None,
        },
    }
}

#[cfg(target_os = "linux")]
fn probe_and_read() -> Option<SensorReading> {
    use i2cdev::core::I2CDevice;
    use i2cdev::linux::LinuxI2CDevice;
    for addr in [0x76u16, 0x77] {
        let Ok(mut dev) = LinuxI2CDevice::new("/dev/i2c-1", addr) else {
            continue;
        };
        let Ok(chip_id) = dev.smbus_read_byte_data(0xD0) else {
            continue;
        };
        let is_bme = chip_id == 0x60;
        let is_bmp = chip_id == 0x58;
        if !is_bme && !is_bmp {
            continue;
        }
        return read_measurement(&mut dev, is_bme);
    }
    None
}

#[cfg(target_os = "linux")]
fn read_measurement(
    dev: &mut i2cdev::linux::LinuxI2CDevice,
    has_humidity: bool,
) -> Option<SensorReading> {
    use i2cdev::core::I2CDevice;
    let cal: [u8; 26] = dev
        .smbus_read_i2c_block_data(0x88, 26)
        .ok()?
        .try_into()
        .ok()?;
    let hum: Option<[u8; 7]> = if has_humidity {
        Some(
            dev.smbus_read_i2c_block_data(0xE1, 7)
                .ok()?
                .try_into()
                .ok()?,
        )
    } else {
        None
    };
    let c = bme280::parse_calibration(&cal, hum.as_ref());
    if has_humidity {
        dev.smbus_write_byte_data(0xF2, 0x01).ok()?; // hum oversampling x1
    }
    dev.smbus_write_byte_data(0xF4, 0x25).ok()?; // T x1, P x1, forced mode
    std::thread::sleep(std::time::Duration::from_millis(20));
    let d: [u8; 8] = dev
        .smbus_read_i2c_block_data(0xF7, 8)
        .ok()?
        .try_into()
        .ok()?;
    let adc_p = ((d[0] as i32) << 12) | ((d[1] as i32) << 4) | ((d[2] as i32) >> 4);
    let adc_t = ((d[3] as i32) << 12) | ((d[4] as i32) << 4) | ((d[5] as i32) >> 4);
    let adc_h = ((d[6] as i32) << 8) | d[7] as i32;
    let (t_fine, t) = bme280::compensate_temperature(adc_t, &c);
    let p = bme280::compensate_pressure(adc_p, t_fine, &c);
    Some(SensorReading {
        temperature_c: t,
        pressure_hpa: p / 100.0,
        humidity_pct: has_humidity.then(|| bme280::compensate_humidity(adc_h, t_fine, &c)),
    })
}

#[cfg(not(target_os = "linux"))]
fn probe_and_read() -> Option<SensorReading> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_wins_over_everything() {
        let s = read_sensor(false);
        assert_eq!(s.state, SensorState::Disabled);
        assert!(s.reading.is_none());
    }

    #[test]
    fn enabled_without_hardware_reports_not_detected() {
        // dev Mac and CI have no /dev/i2c-1; on the Pi the bus is disabled —
        // in every current environment this must be NotDetected, not a panic.
        let s = read_sensor(true);
        assert_eq!(s.state, SensorState::NotDetected);
        assert!(s.reading.is_none());
    }

    #[test]
    fn reading_serializes_as_null_when_missing() {
        let v = serde_json::to_value(read_sensor(false)).unwrap();
        assert_eq!(v["state"], "disabled");
        assert!(v["reading"].is_null());
        assert!(v.as_object().unwrap().contains_key("reading"));
    }
}
