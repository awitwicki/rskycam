use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Concat-demuxer list content for an ordered sequence of frame paths, each
/// shown for `1/fps` seconds. Per the documented ffmpeg concat-demuxer
/// workaround, a `duration` line is otherwise not honored on the final
/// entry, so the last path is repeated once more with no trailing duration.
pub fn build_concat_list(frames: &[PathBuf], fps: u32) -> String {
    let duration = 1.0 / f64::from(fps.max(1));
    let mut out = String::new();
    for f in frames {
        out.push_str(&format!("file '{}'\n", f.display()));
        out.push_str(&format!("duration {duration}\n"));
    }
    if let Some(last) = frames.last() {
        out.push_str(&format!("file '{}'\n", last.display()));
    }
    out
}

/// Build the ffmpeg argument vector for a timelapse encoding run from an
/// explicit concat-demuxer list file (so an arbitrary subset of frames —
/// not necessarily every file in a directory — can be encoded, in order).
pub fn build_ffmpeg_args(
    list_path: &Path,
    fps: u32,
    extra_args: &str,
    output: &Path,
) -> Vec<OsString> {
    let mut v: Vec<OsString> = vec![
        "-y".into(),
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-i".into(),
        list_path.into(),
        // Every concat-list entry already carries the same explicit
        // `duration` (1/fps), so the input is constant-rate by
        // construction — a fixed output `-r` is enough. Real ffmpeg
        // rejects `-r`/`-fpsmax` combined with a non-CFR `-vsync`/
        // `-fps_mode` (e.g. `vfr`) as contradictory; don't add one.
        "-r".into(),
        fps.to_string().into(),
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
    v.push(output.into());
    v
}

/// Run ffmpeg (wrapped in `nice -n 19`) over an explicit, ordered list of
/// frame paths, producing `night_dir/<output_name>`. Blocking — call from
/// `spawn_blocking`. Writes the output atomically via tmp+rename. An empty
/// `frames` list is a no-op success — nothing to encode (e.g. a night with
/// no frames of that day/night classification) — leaving the artifact
/// absent (pending) rather than erroring.
pub fn run_timelapse(
    ffmpeg: &Path,
    night_dir: &Path,
    output_name: &str,
    frames: &[PathBuf],
    fps: u32,
    extra_args: &str,
) -> Result<(), String> {
    if frames.is_empty() {
        return Ok(());
    }
    let list_path = night_dir.join(format!("{output_name}.list.txt"));
    std::fs::write(&list_path, build_concat_list(frames, fps))
        .map_err(|e| format!("writing concat list: {e}"))?;
    let tmp = night_dir.join(format!("{output_name}.tmp"));
    let _ = std::fs::remove_file(&tmp); // stale tmp from a crashed run
    let out = std::process::Command::new("nice")
        .arg("-n")
        .arg("19")
        .arg(ffmpeg)
        .args(build_ffmpeg_args(&list_path, fps, extra_args, &tmp))
        .output()
        .map_err(|e| format!("running ffmpeg ({}): {e}", ffmpeg.display()));
    let _ = std::fs::remove_file(&list_path);
    let out = out?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&tmp);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: String = stderr
            .chars()
            .skip(stderr.chars().count().saturating_sub(500))
            .collect();
        return Err(format!("ffmpeg exited with {}: {tail}", out.status));
    }
    std::fs::rename(&tmp, night_dir.join(output_name))
        .map_err(|e| format!("renaming timelapse output: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-ffmpeg")
    }

    #[test]
    fn concat_list_repeats_the_last_entry_per_the_ffmpeg_workaround() {
        let frames = vec![
            PathBuf::from("/n/frames/a.jpg"),
            PathBuf::from("/n/frames/b.jpg"),
        ];
        let list = build_concat_list(&frames, 25);
        assert_eq!(
            list,
            "file '/n/frames/a.jpg'\nduration 0.04\nfile '/n/frames/b.jpg'\nduration 0.04\nfile '/n/frames/b.jpg'\n"
        );
    }

    #[test]
    fn concat_list_of_no_frames_is_empty() {
        assert_eq!(build_concat_list(&[], 25), "");
    }

    #[test]
    fn args_use_the_concat_demuxer_and_split_extra_args() {
        let args = build_ffmpeg_args(
            Path::new("/n/list.txt"),
            25,
            "-preset veryfast",
            Path::new("/n/out.mp4.tmp"),
        );
        let flat: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let joined = flat.join(" ");
        assert!(joined.contains("-f concat"));
        assert!(joined.contains("-safe 0"));
        assert!(joined.contains("/n/list.txt"));
        assert!(joined.contains("-r 25"));
        assert!(joined.contains("-c:v libx264"));
        assert!(joined.contains("-pix_fmt yuv420p"));
        assert!(joined.contains("-preset veryfast"));
        // extra args come before the explicit muxer format, which comes before the output
        assert_eq!(flat.last().unwrap(), "/n/out.mp4.tmp");
        assert_eq!(flat.get(flat.len() - 3), Some(&"-f".to_string()));
        assert_eq!(flat.get(flat.len() - 2), Some(&"mp4".to_string()));
        let preset_i = flat.iter().position(|a| a == "-preset").unwrap();
        assert!(preset_i < flat.len() - 3);
    }

    #[test]
    fn success_renames_tmp_to_the_named_output_and_cleans_up_the_list() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f1.jpg"), b"x").unwrap();
        run_timelapse(
            &fixture(),
            dir.path(),
            "timelapse-night.mp4",
            &[dir.path().join("f1.jpg")],
            25,
            "",
        )
        .unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("timelapse-night.mp4")).unwrap(),
            b"fake-video"
        );
        assert!(!dir.path().join("timelapse-night.mp4.tmp").exists());
        assert!(!dir.path().join("timelapse-night.mp4.list.txt").exists());
    }

    #[test]
    fn empty_frame_list_is_a_noop_success() {
        let dir = tempfile::TempDir::new().unwrap();
        run_timelapse(&fixture(), dir.path(), "timelapse-day.mp4", &[], 25, "").unwrap();
        assert!(!dir.path().join("timelapse-day.mp4").exists());
        assert!(!dir.path().join("timelapse-day.mp4.list.txt").exists());
    }

    #[test]
    fn failure_surfaces_stderr_and_leaves_no_output() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("fake-ffmpeg-fail"), b"").unwrap();
        std::fs::write(dir.path().join("f1.jpg"), b"x").unwrap();
        let err = run_timelapse(
            &fixture(),
            dir.path(),
            "timelapse-night.mp4",
            &[dir.path().join("f1.jpg")],
            25,
            "",
        )
        .unwrap_err();
        assert!(err.contains("simulated encoder explosion"), "err: {err}");
        assert!(!dir.path().join("timelapse-night.mp4").exists());
        assert!(!dir.path().join("timelapse-night.mp4.tmp").exists());
    }

    #[test]
    fn missing_binary_is_an_error_not_a_panic() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("f1.jpg"), b"x").unwrap();
        let err = run_timelapse(
            Path::new("/nonexistent/ffmpeg"),
            dir.path(),
            "timelapse-night.mp4",
            &[dir.path().join("f1.jpg")],
            25,
            "",
        )
        .unwrap_err();
        assert!(err.contains("ffmpeg"), "err: {err}");
    }
}
