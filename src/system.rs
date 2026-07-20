use std::path::Path;
use std::process::Command;

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatus {
    pub model: String,
    pub cpu_temp_c: f64,
    pub ram_used_mb: u64,
    pub ram_total_mb: u64,
    pub disk_used_gb: f64,
    pub disk_total_gb: f64,
    pub uptime_sec: u64,
    pub undervoltage_now: bool,
    pub undervoltage_since_boot: bool,
}

/// used, total in MB. used = total − MemAvailable.
pub fn parse_meminfo(s: &str) -> Option<(u64, u64)> {
    let field = |name: &str| {
        s.lines()
            .find(|l| l.starts_with(name))?
            .split_whitespace()
            .nth(1)?
            .parse::<u64>()
            .ok()
    };
    let total_kb = field("MemTotal:")?;
    let available_kb = field("MemAvailable:")?;
    let total = total_kb / 1024;
    let used = total_kb.saturating_sub(available_kb) / 1024;
    Some((used, total))
}

/// vcgencmd get_throttled → (undervoltage now: bit 0, since boot: bit 16).
pub fn parse_throttled(s: &str) -> (bool, bool) {
    let Some(hex) = s.trim().strip_prefix("throttled=0x") else {
        return (false, false);
    };
    let Ok(bits) = u64::from_str_radix(hex, 16) else {
        return (false, false);
    };
    (bits & 0x1 != 0, bits & 0x1_0000 != 0)
}

pub fn parse_cpu_temp(millideg: &str) -> Option<f64> {
    millideg.trim().parse::<f64>().ok().map(|v| v / 1000.0)
}

fn disk_usage(path: &Path) -> Option<(f64, f64)> {
    #[cfg(target_os = "linux")]
    {
        let st = nix::sys::statvfs::statvfs(path).ok()?;
        let total = st.blocks() as f64 * st.fragment_size() as f64 / 1e9;
        let used = total - st.blocks_available() as f64 * st.fragment_size() as f64 / 1e9;
        return Some((used, total));
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        None
    }
}

/// Best-effort real values on Linux/Pi; explicit dev fallbacks elsewhere
/// (per spec: non-Pi hosts get mocked values).
pub fn read_system_status(data_dir: &Path) -> SystemStatus {
    let model = std::fs::read_to_string("/proc/device-tree/model")
        .map(|m| m.trim_end_matches('\0').to_string())
        .unwrap_or_else(|_| "Dev host (not a Pi)".into());
    let cpu_temp_c = std::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp")
        .ok()
        .and_then(|s| parse_cpu_temp(&s))
        .unwrap_or(48.0);
    let (ram_used_mb, ram_total_mb) = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| parse_meminfo(&s))
        .unwrap_or((1200, 4096));
    let (disk_used_gb, disk_total_gb) = disk_usage(data_dir).unwrap_or((32.0, 128.0));
    let uptime_sec = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .map(|v| v as u64)
        .unwrap_or(0);
    let (undervoltage_now, undervoltage_since_boot) = Command::new("vcgencmd")
        .arg("get_throttled")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_throttled(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or((false, false));
    SystemStatus {
        model,
        cpu_temp_c,
        ram_used_mb,
        ram_total_mb,
        disk_used_gb,
        disk_total_gb,
        uptime_sec,
        undervoltage_now,
        undervoltage_since_boot,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_meminfo_used_and_total() {
        let s = "MemTotal:        1917292 kB\nMemFree:          120000 kB\nMemAvailable:    1436544 kB\nBuffers:           50000 kB\n";
        let (used, total) = parse_meminfo(s).unwrap();
        assert_eq!(total, 1872); // 1917292/1024
        assert_eq!(used, total - 1403); // total - available(1436544/1024)
    }

    #[test]
    fn parses_throttled_bitmask() {
        assert_eq!(parse_throttled("throttled=0x0"), (false, false));
        assert_eq!(parse_throttled("throttled=0x50005"), (true, true));
        assert_eq!(parse_throttled("throttled=0x50000"), (false, true));
        assert_eq!(parse_throttled("garbage"), (false, false));
    }

    #[test]
    fn parses_cpu_temp_millidegrees() {
        assert_eq!(parse_cpu_temp("48534\n"), Some(48.534));
        assert_eq!(parse_cpu_temp("nope"), None);
    }

    #[test]
    fn read_system_status_never_panics_and_fills_every_field() {
        let s = read_system_status(std::path::Path::new("/tmp"));
        assert!(!s.model.is_empty());
        assert!(s.ram_total_mb > 0);
        assert!(s.disk_total_gb > 0.0);
    }
}
