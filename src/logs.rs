//! Log-file tail for the web UI Logs page. The file layer in `main` writes
//! daily-rotated files to `<data_dir>/logs/rskycam.YYYY-MM-DD.log`; this
//! module reads the newest one or two back (the day-boundary case, so the
//! page isn't near-empty just after midnight) and returns the last N lines.

use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_LINES: usize = 500;
pub const MAX_LINES: usize = 2000;

pub fn log_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

/// Last `n` non-empty lines across file contents ordered oldest→newest.
pub fn tail_lines(files_oldest_first: &[String], n: usize) -> Vec<String> {
    let mut lines: Vec<String> = files_oldest_first
        .iter()
        .flat_map(|content| content.lines())
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    let start = lines.len().saturating_sub(n);
    lines.split_off(start)
}

/// Tail of the current log across the two newest daily files. Unreadable or
/// absent files simply contribute nothing — the page shows what exists.
pub fn read_tail(data_dir: &Path, n: usize) -> Vec<String> {
    let Ok(entries) = fs::read_dir(log_dir(data_dir)) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|f| f.to_str())
                .is_some_and(|f| f.starts_with("rskycam.") && f.ends_with(".log"))
        })
        .collect();
    // Dated names (rskycam.YYYY-MM-DD.log) sort chronologically as strings.
    files.sort();
    let newest_two_oldest_first = files.iter().rev().take(2).rev();
    let contents: Vec<String> = newest_two_oldest_first
        .filter_map(|p| fs::read_to_string(p).ok())
        .collect();
    tail_lines(&contents, n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn tail_spans_files_and_keeps_only_the_last_n() {
        let yesterday = "a1\na2\na3\n".to_string();
        let today = "b1\nb2\n".to_string();
        assert_eq!(
            tail_lines(&[yesterday.clone(), today.clone()], 4),
            vec!["a2", "a3", "b1", "b2"]
        );
        // n larger than everything: all lines, in order
        assert_eq!(
            tail_lines(&[yesterday, today], 100),
            vec!["a1", "a2", "a3", "b1", "b2"]
        );
        assert!(tail_lines(&[], 10).is_empty());
    }

    #[test]
    fn tail_skips_empty_lines() {
        assert_eq!(tail_lines(&["x\n\n\ny\n".to_string()], 10), vec!["x", "y"]);
    }

    #[test]
    fn read_tail_uses_the_two_newest_files_in_date_order() {
        let dir = TempDir::new().unwrap();
        let logs = log_dir(dir.path());
        fs::create_dir_all(&logs).unwrap();
        fs::write(logs.join("rskycam.2026-07-30.log"), "old\n").unwrap();
        fs::write(logs.join("rskycam.2026-07-31.log"), "mid1\nmid2\n").unwrap();
        fs::write(logs.join("rskycam.2026-08-01.log"), "new\n").unwrap();
        fs::write(logs.join("unrelated.txt"), "junk\n").unwrap();
        // Only the two newest dated files, oldest first, junk ignored.
        assert_eq!(read_tail(dir.path(), 10), vec!["mid1", "mid2", "new"]);
        assert_eq!(read_tail(dir.path(), 2), vec!["mid2", "new"]);
    }

    #[test]
    fn read_tail_is_empty_without_a_log_dir() {
        let dir = TempDir::new().unwrap();
        assert!(read_tail(dir.path(), 10).is_empty());
    }
}
