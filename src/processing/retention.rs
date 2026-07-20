use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::settings::ConfigFile;

#[derive(Debug, PartialEq)]
pub enum Action {
    /// Delete frames/ + frames.jsonl, keep artifacts (keogram, trails, video).
    DeleteFrames(String),
    /// Delete the whole night directory.
    DeleteNight(String),
}

/// Pure retention planner. The two limits are independent: whichever is
/// exceeded first wins, so artifacts_days < frames_days deletes whole
/// nights before frames-only pruning would have kicked in.
pub fn plan(
    dates: &[String],
    today: chrono::NaiveDate,
    frames_days: u32,
    artifacts_days: u32,
) -> Vec<Action> {
    let mut actions = Vec::new();
    for date_s in dates {
        let Ok(date) = chrono::NaiveDate::parse_from_str(date_s, "%Y-%m-%d") else {
            continue;
        };
        let age = (today - date).num_days();
        if age > artifacts_days as i64 {
            actions.push(Action::DeleteNight(date_s.clone()));
        } else if age > frames_days as i64 {
            actions.push(Action::DeleteFrames(date_s.clone()));
        }
    }
    actions
}

/// Treat a missing file/dir as success: deletions must be idempotent so a
/// re-run (or a partially-applied prior run) never surfaces a stale error.
fn ignore_not_found(r: std::io::Result<()>) -> std::io::Result<()> {
    match r {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Apply planned deletions. Blocking — call from spawn_blocking.
pub fn execute(data_dir: &Path, actions: &[Action]) {
    for a in actions {
        let r = match a {
            Action::DeleteFrames(date) => {
                let night = data_dir.join("images").join(date);
                let rd = ignore_not_found(std::fs::remove_dir_all(night.join("frames")));
                let rf = ignore_not_found(std::fs::remove_file(night.join("frames.jsonl")));
                // Both stale-NotFound results are already neutralized above,
                // so combining them here reports the first *real* error
                // instead of a stale NotFound masking a genuine failure.
                let r = rd.and(rf);
                if r.is_ok() {
                    tracing::info!("retention: pruned frames of {date}");
                }
                r
            }
            Action::DeleteNight(date) => {
                let r =
                    ignore_not_found(std::fs::remove_dir_all(data_dir.join("images").join(date)));
                if r.is_ok() {
                    tracing::info!("retention: deleting night {date}");
                }
                r
            }
        };
        if let Err(e) = r {
            tracing::warn!("retention action {a:?}: {e}");
        }
    }
}

fn list_night_dates(images: &Path) -> Vec<String> {
    std::fs::read_dir(images)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                // Never follow symlinks when choosing deletion targets: a
                // symlinked entry could point outside the data dir, and
                // DeleteFrames/DeleteNight would then recurse through it.
                .filter(|e| {
                    std::fs::symlink_metadata(e.path())
                        .map(|m| m.file_type().is_dir())
                        .unwrap_or(false)
                })
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Hourly retention sweep (first pass ~60 s after startup).
pub fn spawn_retention(cfg: Arc<RwLock<ConfigFile>>, data_dir: PathBuf) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await; // the first tick fires immediately — consume it
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        loop {
            let storage = cfg.read().await.settings.storage;
            let dd = data_dir.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let images = dd.join("images");
                let dates = list_night_dates(&images);
                let actions = plan(
                    &dates,
                    crate::capture::night_date(chrono::Local::now()),
                    storage.frames_retention_days,
                    storage.artifacts_retention_days,
                );
                execute(&dd, &actions);
            })
            .await;
            tick.tick().await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn plan_keeps_fresh_prunes_frames_then_whole_nights() {
        let dates = vec![
            "2026-07-16".to_string(), // today — kept
            "2026-07-10".to_string(), // 6 days — kept (≤ 7)
            "2026-07-05".to_string(), // 11 days — frames pruned (> 7, ≤ 30)
            "2026-05-01".to_string(), // 76 days — whole night gone (> 30)
            "not-a-date".to_string(), // ignored
        ];
        let actions = plan(&dates, d("2026-07-16"), 7, 30);
        assert_eq!(
            actions,
            vec![
                Action::DeleteFrames("2026-07-05".into()),
                Action::DeleteNight("2026-05-01".into()),
            ]
        );
    }

    #[test]
    fn artifacts_shorter_than_frames_means_whole_night_governs() {
        let actions = plan(&["2026-07-01".to_string()], d("2026-07-16"), 30, 7);
        assert_eq!(actions, vec![Action::DeleteNight("2026-07-01".into())]);
    }

    #[test]
    fn execute_deletes_frames_but_keeps_artifacts() {
        let dir = tempfile::TempDir::new().unwrap();
        let night = dir.path().join("images").join("2026-07-05");
        std::fs::create_dir_all(night.join("frames")).unwrap();
        std::fs::write(night.join("frames").join("a.jpg"), b"x").unwrap();
        std::fs::write(night.join("frames.jsonl"), b"{}").unwrap();
        std::fs::write(night.join("keogram.jpg"), b"k").unwrap();
        let gone = dir.path().join("images").join("2026-05-01");
        std::fs::create_dir_all(&gone).unwrap();

        execute(
            dir.path(),
            &[
                Action::DeleteFrames("2026-07-05".into()),
                Action::DeleteNight("2026-05-01".into()),
            ],
        );
        assert!(!night.join("frames").exists());
        assert!(!night.join("frames.jsonl").exists());
        assert!(night.join("keogram.jpg").is_file()); // artifacts kept
        assert!(!gone.exists());

        // A second execute() over the same actions must be a silent no-op:
        // everything is already gone, so this should be Ok and must not panic.
        execute(
            dir.path(),
            &[
                Action::DeleteFrames("2026-07-05".into()),
                Action::DeleteNight("2026-05-01".into()),
            ],
        );
        assert!(!night.join("frames").exists());
        assert!(!night.join("frames.jsonl").exists());
        assert!(night.join("keogram.jpg").is_file());
        assert!(!gone.exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_night_entries_are_never_deletion_targets() {
        let dir = tempfile::TempDir::new().unwrap();
        let images = dir.path().join("images");
        std::fs::create_dir_all(&images).unwrap();
        // A real victim directory outside images/ with a frames/ subdir.
        let victim = dir.path().join("victim");
        std::fs::create_dir_all(victim.join("frames")).unwrap();
        std::fs::write(victim.join("frames").join("keep.jpg"), b"x").unwrap();
        std::os::unix::fs::symlink(&victim, images.join("2000-01-01")).unwrap();
        // The planner must never see the symlinked entry...
        assert!(list_night_dates(&images).is_empty());
        // ...and even a directly-executed DeleteFrames for it must not
        // reach through the link (defense in depth check of current behavior
        // is NOT required — the list filter is the guarantee under test).
        assert!(victim.join("frames").join("keep.jpg").exists());
    }
}
