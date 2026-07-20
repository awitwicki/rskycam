use std::ffi::OsString;
use std::path::Path;

/// Build the ffmpeg argument vector for a timelapse encoding run.
///
/// Uses glob input pattern with ffmpeg's native glob expansion for chronological
/// ordering. Applies video scaling, x264 encoding with yuv420p pixel format,
/// and allows extra user-supplied arguments (e.g., `-preset veryfast`).
pub fn build_ffmpeg_args(night_dir: &Path, fps: u32, extra_args: &str) -> Vec<OsString> {
    let mut v: Vec<OsString> = vec![
        "-y".into(),
        "-f".into(),
        "image2".into(),
        "-pattern_type".into(),
        "glob".into(),
        "-framerate".into(),
        fps.to_string().into(),
        "-i".into(),
        night_dir.join("frames").join("*.jpg").into_os_string(),
        "-vf".into(),
        "scale=trunc(iw/2)*2:trunc(ih/2)*2".into(),
        "-c:v".into(),
        "libx264".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
    ];
    v.extend(extra_args.split_whitespace().map(OsString::from));
    // ffmpeg 7 cannot infer mp4 muxer from .tmp extension; must specify explicitly
    v.push("-f".into());
    v.push("mp4".into());
    v.push(night_dir.join("timelapse.mp4.tmp").into_os_string());
    v
}

/// Run ffmpeg (wrapped in `nice -n 19`) for one night. Blocking — call
/// from spawn_blocking. Writes timelapse.mp4 atomically via tmp+rename.
pub fn run_timelapse(
    ffmpeg: &Path,
    night_dir: &Path,
    fps: u32,
    extra_args: &str,
) -> Result<(), String> {
    let tmp = night_dir.join("timelapse.mp4.tmp");
    let _ = std::fs::remove_file(&tmp); // stale tmp from a crashed run
    let out = std::process::Command::new("nice")
        .arg("-n")
        .arg("19")
        .arg(ffmpeg)
        .args(build_ffmpeg_args(night_dir, fps, extra_args))
        .output()
        .map_err(|e| format!("running ffmpeg ({}): {e}", ffmpeg.display()))?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&tmp);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: String = stderr
            .chars()
            .skip(stderr.chars().count().saturating_sub(500))
            .collect();
        return Err(format!("ffmpeg exited with {}: {tail}", out.status));
    }
    std::fs::rename(&tmp, night_dir.join("timelapse.mp4"))
        .map_err(|e| format!("renaming timelapse output: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-ffmpeg")
    }

    fn night_dir() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("frames")).unwrap();
        dir
    }

    #[test]
    fn args_include_fps_glob_input_and_split_extra_args() {
        let args = build_ffmpeg_args(std::path::Path::new("/n"), 25, "-preset veryfast");
        let flat: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let joined = flat.join(" ");
        assert!(joined.contains("-framerate 25"));
        assert!(joined.contains("-pattern_type glob"));
        assert!(joined.contains("/n/frames/*.jpg"));
        assert!(joined.contains("-c:v libx264"));
        assert!(joined.contains("-pix_fmt yuv420p"));
        assert!(joined.contains("-preset veryfast"));
        assert!(joined.contains("-f mp4"));
        // extra args come before the explicit muxer format, which comes before the output
        assert_eq!(flat.last().unwrap(), "/n/timelapse.mp4.tmp");
        assert_eq!(flat.get(flat.len() - 3), Some(&"-f".to_string()));
        assert_eq!(flat.get(flat.len() - 2), Some(&"mp4".to_string()));
        let preset_i = flat.iter().position(|a| a == "-preset").unwrap();
        assert!(preset_i < flat.len() - 3); // preset must come before -f mp4
    }

    #[test]
    fn success_renames_tmp_to_final_output() {
        let dir = night_dir();
        run_timelapse(&fixture(), dir.path(), 25, "").unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("timelapse.mp4")).unwrap(),
            b"fake-video"
        );
        assert!(!dir.path().join("timelapse.mp4.tmp").exists());
        let args = std::fs::read_to_string(dir.path().join("timelapse.mp4.tmp.args")).unwrap();
        assert!(args.contains("-framerate 25"));
    }

    #[test]
    fn failure_surfaces_stderr_and_leaves_no_output() {
        let dir = night_dir();
        std::fs::write(dir.path().join("fake-ffmpeg-fail"), b"").unwrap();
        let err = run_timelapse(&fixture(), dir.path(), 25, "").unwrap_err();
        assert!(err.contains("simulated encoder explosion"), "err: {err}");
        assert!(!dir.path().join("timelapse.mp4").exists());
        assert!(!dir.path().join("timelapse.mp4.tmp").exists());
    }

    #[test]
    fn missing_binary_is_an_error_not_a_panic() {
        let dir = night_dir();
        let err = run_timelapse(
            std::path::Path::new("/nonexistent/ffmpeg"),
            dir.path(),
            25,
            "",
        )
        .unwrap_err();
        assert!(err.contains("ffmpeg"), "err: {err}");
    }
}
