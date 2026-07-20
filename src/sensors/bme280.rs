#[derive(Clone, Copy, Debug, Default)]
pub struct Calibration {
    pub dig_t1: u16,
    pub dig_t2: i16,
    pub dig_t3: i16,
    pub dig_p1: u16,
    pub dig_p2: i16,
    pub dig_p3: i16,
    pub dig_p4: i16,
    pub dig_p5: i16,
    pub dig_p6: i16,
    pub dig_p7: i16,
    pub dig_p8: i16,
    pub dig_p9: i16,
    pub dig_h1: u8,
    pub dig_h2: i16,
    pub dig_h3: u8,
    pub dig_h4: i16,
    pub dig_h5: i16,
    pub dig_h6: i8,
}

/// Returns (t_fine, temperature °C). Datasheet double-precision variant.
pub fn compensate_temperature(adc_t: i32, c: &Calibration) -> (f64, f64) {
    let adc = adc_t as f64;
    let var1 = (adc / 16384.0 - c.dig_t1 as f64 / 1024.0) * c.dig_t2 as f64;
    let var2 = (adc / 131072.0 - c.dig_t1 as f64 / 8192.0).powi(2) * c.dig_t3 as f64;
    let t_fine = var1 + var2;
    (t_fine, t_fine / 5120.0)
}

/// Pressure in Pa.
pub fn compensate_pressure(adc_p: i32, t_fine: f64, c: &Calibration) -> f64 {
    let mut var1 = t_fine / 2.0 - 64000.0;
    let mut var2 = var1 * var1 * c.dig_p6 as f64 / 32768.0;
    var2 += var1 * c.dig_p5 as f64 * 2.0;
    var2 = var2 / 4.0 + c.dig_p4 as f64 * 65536.0;
    var1 = (c.dig_p3 as f64 * var1 * var1 / 524288.0 + c.dig_p2 as f64 * var1) / 524288.0;
    var1 = (1.0 + var1 / 32768.0) * c.dig_p1 as f64;
    if var1 == 0.0 {
        return 0.0;
    }
    let mut p = 1048576.0 - adc_p as f64;
    p = (p - var2 / 4096.0) * 6250.0 / var1;
    let var1b = c.dig_p9 as f64 * p * p / 2147483648.0;
    let var2b = p * c.dig_p8 as f64 / 32768.0;
    p + (var1b + var2b + c.dig_p7 as f64) / 16.0
}

/// Relative humidity %, clamped to 0..100.
pub fn compensate_humidity(adc_h: i32, t_fine: f64, c: &Calibration) -> f64 {
    let mut h = t_fine - 76800.0;
    h = (adc_h as f64 - (c.dig_h4 as f64 * 64.0 + c.dig_h5 as f64 / 16384.0 * h))
        * (c.dig_h2 as f64 / 65536.0
            * (1.0 + c.dig_h6 as f64 / 67108864.0 * h * (1.0 + c.dig_h3 as f64 / 67108864.0 * h)));
    h *= 1.0 - c.dig_h1 as f64 * h / 524288.0;
    h.clamp(0.0, 100.0)
}

/// Parse the raw calibration registers (0x88..0xA1 block, plus the 0xE1..0xE7
/// humidity block). Pure so the bit-packing is testable without hardware.
pub fn parse_calibration(cal: &[u8; 26], hum: Option<&[u8; 7]>) -> Calibration {
    let u16le = |i: usize| u16::from_le_bytes([cal[i], cal[i + 1]]);
    let i16le = |i: usize| i16::from_le_bytes([cal[i], cal[i + 1]]);
    let mut c = Calibration {
        dig_t1: u16le(0),
        dig_t2: i16le(2),
        dig_t3: i16le(4),
        dig_p1: u16le(6),
        dig_p2: i16le(8),
        dig_p3: i16le(10),
        dig_p4: i16le(12),
        dig_p5: i16le(14),
        dig_p6: i16le(16),
        dig_p7: i16le(18),
        dig_p8: i16le(20),
        dig_p9: i16le(22),
        ..Calibration::default()
    };
    if let Some(h) = hum {
        c.dig_h1 = cal[25];
        c.dig_h2 = i16::from_le_bytes([h[0], h[1]]);
        c.dig_h3 = h[2];
        // dig_H4/dig_H5 are 12-bit two's-complement; registers 0xE4/0xE6 hold
        // the sign bit, so sign-extend through i8 before shifting.
        c.dig_h4 = ((h[3] as i8 as i16) << 4) | (h[4] & 0x0F) as i16;
        c.dig_h5 = ((h[5] as i8 as i16) << 4) | ((h[4] >> 4) & 0x0F) as i16;
        c.dig_h6 = h[6] as i8;
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn datasheet_calibration() -> Calibration {
        Calibration {
            dig_t1: 27504,
            dig_t2: 26435,
            dig_t3: -1000,
            dig_p1: 36477,
            dig_p2: -10685,
            dig_p3: 3024,
            dig_p4: 2855,
            dig_p5: 140,
            dig_p6: -7,
            dig_p7: 15500,
            dig_p8: -14600,
            dig_p9: 6000,
            dig_h1: 0,
            dig_h2: 0,
            dig_h3: 0,
            dig_h4: 0,
            dig_h5: 0,
            dig_h6: 0,
        }
    }

    #[test]
    fn temperature_matches_the_datasheet_worked_example() {
        let (_, t) = compensate_temperature(519888, &datasheet_calibration());
        assert!((t - 25.08).abs() < 0.01, "got {t}");
    }

    #[test]
    fn pressure_matches_the_datasheet_worked_example() {
        let c = datasheet_calibration();
        let (t_fine, _) = compensate_temperature(519888, &c);
        let p = compensate_pressure(415148, t_fine, &c);
        assert!((p - 100653.27).abs() < 1.0, "got {p}");
    }

    #[test]
    fn humidity_is_clamped_to_0_100() {
        let c = datasheet_calibration();
        let (t_fine, _) = compensate_temperature(519888, &c);
        let h = compensate_humidity(30000, t_fine, &c);
        assert!((0.0..=100.0).contains(&h));
    }

    #[test]
    fn parse_calibration_sign_extends_h4_and_h5() {
        let mut cal = [0u8; 26];
        cal[0..2].copy_from_slice(&27504u16.to_le_bytes());
        cal[25] = 75;
        // h[3]=0x80 → negative dig_h4; h[5]=0x80, h[4] hi nibble 0 → negative dig_h5
        let hum = [0x00, 0x00, 0x00, 0x80, 0x05, 0x80, 0x00];
        let c = parse_calibration(&cal, Some(&hum));
        assert_eq!(c.dig_t1, 27504);
        assert_eq!(c.dig_h1, 75);
        assert_eq!(c.dig_h4, -2043); // 0x80 sign-extended << 4 | 0x5
        assert_eq!(c.dig_h5, -2048);
    }

    #[test]
    fn parse_calibration_positive_h4_h5_and_no_humidity() {
        let cal = [0u8; 26];
        let hum = [0x00, 0x00, 0x00, 0x40, 0x25, 0x12, 0x00];
        let c = parse_calibration(&cal, Some(&hum));
        assert_eq!(c.dig_h4, (0x40 << 4) | 0x5); // 1029
        assert_eq!(c.dig_h5, (0x12 << 4) | 0x2); // 290
        let no_hum = parse_calibration(&cal, None);
        assert_eq!(no_hum.dig_h2, 0);
        assert_eq!(no_hum.dig_h4, 0);
    }
}
