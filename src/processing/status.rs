use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

pub const STATUS_FILE: &str = "processing.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ArtifactProgress {
    Generating,
    Error { message: String },
}

/// Per-night processing progress, persisted next to the artifacts so the
/// nights API can surface generating/error states across restarts.
/// Absent entry = no news (artifact presence on disk decides ready/pending).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NightProcessingStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keogram: Option<ArtifactProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startrails: Option<ArtifactProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timelapse: Option<ArtifactProgress>,
}

pub fn load(night_dir: &Path) -> NightProcessingStatus {
    std::fs::read_to_string(night_dir.join(STATUS_FILE))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

// Per-process, per-call unique suffix so two concurrent `save` callers (e.g. a
// live frame's write_artifacts and a background rebuild's finalize_night,
// racing on the currently-open night's dir — see final-review finding I-1)
// never share a tmp path. A shared fixed tmp name would let one writer's
// truncate/write interleave with another's, and the atomic rename could then
// move a corrupt/mixed file into place. Unique names make that impossible:
// each writer renames only its own complete file.
static NEXT_TMP_ID: AtomicU64 = AtomicU64::new(0);

// Consumed by nights.rs and nights tests; Tasks 8/9 write it via this.
pub fn save(night_dir: &Path, st: &NightProcessingStatus) -> anyhow::Result<()> {
    // Uniqueness note: a crash between the write and the rename below would
    // leave one of these uniquely-named tmp files behind. We deliberately do
    // NOT add cleanup-on-write-start logic for this — it's the simpler choice
    // and keeps this function's job to "write one status file atomically".
    // Stragglers are harmless (never read by `load`, which only looks at
    // STATUS_FILE) and are cleared whenever the night directory itself is
    // eventually deleted by retention.
    let n = NEXT_TMP_ID.fetch_add(1, Ordering::Relaxed);
    let tmp = night_dir.join(format!("{STATUS_FILE}.tmp-{}-{n}", std::process::id()));
    std::fs::write(&tmp, serde_json::to_vec_pretty(st)?)?;
    std::fs::rename(&tmp, night_dir.join(STATUS_FILE))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_and_defaults_when_missing_or_corrupt() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(load(dir.path()), NightProcessingStatus::default());

        let st = NightProcessingStatus {
            timelapse: Some(ArtifactProgress::Error {
                message: "no ffmpeg".into(),
            }),
            keogram: Some(ArtifactProgress::Generating),
            ..Default::default()
        };
        save(dir.path(), &st).unwrap();
        assert_eq!(load(dir.path()), st);

        std::fs::write(dir.path().join(STATUS_FILE), "not json").unwrap();
        assert_eq!(load(dir.path()), NightProcessingStatus::default());
    }

    #[test]
    fn concurrent_saves_never_produce_a_corrupt_or_mixed_file() {
        // Regression test for final-review finding I-1: a live frame's
        // write_artifacts and a background rebuild's finalize_night can both
        // call status::save on the same night dir concurrently. Before the
        // fix, both shared a fixed "processing.json.tmp" path, so one
        // writer's truncate/write could interleave with another's before the
        // atomic rename, risking a corrupt/mixed file landing at
        // STATUS_FILE. Looped for confidence since the race window is a
        // handful of instructions.
        for i in 0..50 {
            let dir = tempfile::TempDir::new().unwrap();
            let path = dir.path().to_path_buf();
            let a = NightProcessingStatus {
                keogram: Some(ArtifactProgress::Generating),
                ..Default::default()
            };
            let b = NightProcessingStatus {
                startrails: Some(ArtifactProgress::Error {
                    message: "boom".into(),
                }),
                ..Default::default()
            };
            let (pa, pb) = (path.clone(), path.clone());
            let (sa, sb) = (a.clone(), b.clone());
            let t1 = std::thread::spawn(move || save(&pa, &sa));
            let t2 = std::thread::spawn(move || save(&pb, &sb));
            t1.join().unwrap().unwrap();
            t2.join().unwrap().unwrap();

            let raw = std::fs::read_to_string(path.join(STATUS_FILE)).unwrap();
            let parsed: NightProcessingStatus = serde_json::from_str(&raw).unwrap_or_else(|e| {
                panic!("iteration {i}: corrupt processing.json: {e}\nraw: {raw}")
            });
            assert!(
                parsed == a || parsed == b,
                "iteration {i}: final file must be exactly one writer's payload, got {parsed:?}"
            );
        }
    }

    #[test]
    fn wire_format_matches_artifact_state_tags() {
        let st = NightProcessingStatus {
            timelapse: Some(ArtifactProgress::Error {
                message: "boom".into(),
            }),
            startrails: Some(ArtifactProgress::Generating),
            ..Default::default()
        };
        let v: serde_json::Value = serde_json::to_value(&st).unwrap();
        assert_eq!(v["timelapse"]["state"], "error");
        assert_eq!(v["timelapse"]["message"], "boom");
        assert_eq!(v["startrails"]["state"], "generating");
        assert!(v.get("keogram").is_none()); // absent, not null
    }
}
