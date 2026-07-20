pub mod keogram;
pub mod retention;
pub mod startrails;
pub mod status;
pub mod timelapse;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chrono::NaiveDate;
use image::{ImageFormat, RgbImage};
use serde::Deserialize;
use tokio::sync::{mpsc, RwLock};

use crate::settings::{ConfigFile, ProcessingSettings};

pub struct NightFrame {
    pub date: NaiveDate,
    pub file: String,
    pub image: RgbImage,
    pub mean: f64,
}

pub struct ProcessingConfig {
    pub ffmpeg: PathBuf,
    pub dawn_check: std::time::Duration,
}

pub enum Command {
    Rebuild { date: NaiveDate },
}

#[derive(Clone)]
pub struct ProcessingHandle {
    pub frames: mpsc::Sender<NightFrame>,
    pub commands: mpsc::Sender<Command>,
}

struct NightState {
    date: NaiveDate,
    last_file: String,
    keogram: keogram::Keogram,
    startrails: startrails::Startrails,
}

#[derive(Deserialize)]
struct FrameLine {
    file: String,
}

fn night_dir(data_dir: &Path, date: NaiveDate) -> PathBuf {
    data_dir.join("images").join(date.to_string())
}

/// Feed one decoded frame into the accumulators per the current settings.
/// `file` is the frame's filename — its embedded timestamp becomes the
/// keogram column's position on the hour scale.
fn accumulate(st: &mut NightState, img: &RgbImage, mean: f64, file: &str, p: &ProcessingSettings) {
    if p.keogram {
        st.keogram.add_frame(img, keogram::frame_time(file));
    }
    if p.startrails {
        st.startrails
            .add_frame(img, mean, p.startrails_brightness_limit);
    }
}

// Per-process, per-call unique suffix for the JPEG tmp files below — see the
// matching comment on status::NEXT_TMP_ID / status::save for why a shared
// fixed tmp name is unsafe when a rebuild of the currently-open night races
// live accumulation on the same dir (final-review finding I-1). We don't
// proactively clean up stray unique tmp names from a crash mid-write: they're
// never read by anything, and retention deletes the whole night dir once it
// ages out, so leftovers don't accumulate indefinitely.
static NEXT_TMP_ID: AtomicU64 = AtomicU64::new(0);

/// Write current keogram/startrails to disk (tmp+rename). Records write
/// errors in processing.json so the UI can see them.
fn write_artifacts(dir: &Path, st: &NightState, p: &ProcessingSettings) {
    let mut progress = status::load(dir);
    let save_atomic = |img: &RgbImage, name: &str| -> Result<(), String> {
        let n = NEXT_TMP_ID.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(format!("{name}.tmp-{}-{n}", std::process::id()));
        img.save_with_format(&tmp, ImageFormat::Jpeg)
            .map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, dir.join(name)).map_err(|e| e.to_string())
    };
    if p.keogram {
        progress.keogram = match st.keogram.annotated(&st.date.to_string()) {
            Some(img) => match save_atomic(&img, "keogram.jpg") {
                Ok(()) => None,
                Err(e) => {
                    tracing::error!("writing keogram: {e}");
                    Some(status::ArtifactProgress::Error { message: e })
                }
            },
            None => None, // nothing decodable to render — clear any stale Generating flag
        };
    }
    if p.startrails {
        progress.startrails = match st.startrails.to_image() {
            Some(img) => match save_atomic(img, "startrails.jpg") {
                Ok(()) => None,
                Err(e) => {
                    tracing::error!("writing startrails: {e}");
                    Some(status::ArtifactProgress::Error { message: e })
                }
            },
            None => None, // nothing decodable to render — clear any stale Generating flag
        };
    }
    if let Err(e) = status::save(dir, &progress) {
        tracing::error!("writing {}: {e:#}", status::STATUS_FILE);
    }
}

/// Rebuild a night's accumulators from the frames already on disk.
/// Blocking — call from spawn_blocking.
fn replay_night(data_dir: &Path, date: NaiveDate, p: &ProcessingSettings) -> NightState {
    let dir = night_dir(data_dir, date);
    let mut st = NightState {
        date,
        last_file: String::new(),
        keogram: keogram::Keogram::default(),
        startrails: startrails::Startrails::default(),
    };
    let Ok(raw) = std::fs::read_to_string(dir.join("frames.jsonl")) else {
        // No frames.jsonl at all (e.g. rebuild of a night with nothing
        // captured): still resolve any pre-marked Generating flags via
        // write_artifacts rather than leaving them stuck.
        write_artifacts(&dir, &st, p);
        return st;
    };
    for line in raw.lines() {
        let Ok(fl) = serde_json::from_str::<FrameLine>(line) else {
            continue;
        };
        let Ok(img) = image::open(dir.join("frames").join(&fl.file)) else {
            continue; // pruned or unreadable frame — skip
        };
        let img = img.to_rgb8();
        let mean = crate::camera::mean_brightness(&img);
        accumulate(&mut st, &img, mean, &fl.file, p);
        st.last_file = fl.file;
    }
    write_artifacts(&dir, &st, p);
    st
}

/// Run the dawn finalization for a finished night: the timelapse.
/// Blocking — call from spawn_blocking.
fn finalize_night(data_dir: &Path, date: NaiveDate, p: &ProcessingSettings, ffmpeg: &Path) {
    let dir = night_dir(data_dir, date);
    if !p.timelapse {
        return;
    }
    let mut progress = status::load(&dir);
    progress.timelapse = Some(status::ArtifactProgress::Generating);
    if let Err(e) = status::save(&dir, &progress) {
        tracing::error!("writing {}: {e:#}", status::STATUS_FILE);
    }
    let result = timelapse::run_timelapse(ffmpeg, &dir, p.timelapse_fps, &p.timelapse_extra_args);
    progress.timelapse = match result {
        Ok(()) => {
            tracing::info!("timelapse for {date} done");
            None
        }
        Err(e) => {
            tracing::error!("timelapse for {date} failed: {e}");
            Some(status::ArtifactProgress::Error { message: e })
        }
    };
    if let Err(e) = status::save(&dir, &progress) {
        tracing::error!("writing {}: {e:#}", status::STATUS_FILE);
    }
}

pub fn spawn_processing(
    cfg: Arc<RwLock<ConfigFile>>,
    data_dir: PathBuf,
    pc: ProcessingConfig,
) -> ProcessingHandle {
    let (frames_tx, mut frames_rx) = mpsc::channel::<NightFrame>(4);
    let (commands_tx, mut commands_rx) = mpsc::channel::<Command>(4);

    tokio::spawn(async move {
        let mut state: Option<NightState> = None;
        let mut last_finalized: Option<chrono::NaiveDate> = None;
        let mut pending_rebuild: Option<tokio::task::JoinHandle<NightState>> = None;
        let mut tick = tokio::time::interval(pc.dawn_check);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await; // the first tick fires immediately — consume it

        loop {
            tokio::select! {
                maybe = frames_rx.recv() => {
                    let Some(frame) = maybe else { break }; // all senders dropped
                    // A night is closed once finalized: captureDuringDay can keep
                    // delivering frames for an already-finalized date until local
                    // noon rolls the bucket over. Drop them from artifacts (they
                    // are still captured/persisted by the capture loop).
                    if last_finalized.is_some_and(|d| frame.date <= d) {
                        tracing::debug!("frame for finalized night {} dropped from artifacts", frame.date);
                        continue;
                    }
                    let p = {
                        let g = cfg.read().await;
                        g.settings.processing.clone()
                    };
                    let dd = data_dir.clone();
                    match state.take() {
                        Some(st) if st.date == frame.date => {
                            if frame.file <= st.last_file {
                                state = Some(st); // duplicate/older than replay — skip
                                continue;
                            }
                            // Move the accumulators through spawn_blocking and back.
                            let joined = tokio::task::spawn_blocking(move || {
                                let mut st = st;
                                accumulate(&mut st, &frame.image, frame.mean, &frame.file, &p);
                                st.last_file = frame.file;
                                write_artifacts(&night_dir(&dd, st.date), &st, &p);
                                st
                            })
                            .await;
                            match joined {
                                Ok(st) => state = Some(st),
                                Err(e) => tracing::error!("processing frame panicked: {e}"),
                            }
                        }
                        other => {
                            // New night (or first frame after startup): finalize
                            // the previous night, then replay this one from disk.
                            // Capture the previous night's date before `other`
                            // moves into the spawn_blocking closure.
                            if let Some(prev_date) = other.as_ref().map(|st| st.date) {
                                last_finalized = Some(prev_date);
                            }
                            let ffmpeg = pc.ffmpeg.clone();
                            let joined = tokio::task::spawn_blocking(move || {
                                if let Some(prev) = other {
                                    finalize_night(&dd, prev.date, &p, &ffmpeg);
                                }
                                replay_night(&dd, frame.date, &p)
                            })
                            .await;
                            match joined {
                                Ok(st) => state = Some(st),
                                Err(e) => tracing::error!("night replay panicked: {e}"),
                            }
                        }
                    }
                }
                // Guarded so at most one rebuild runs at a time; queued
                // Rebuild commands simply wait unread in the channel until
                // the in-flight one completes.
                //
                // Known accepted race — rebuild of the *currently-open* night:
                // this arm's spawn_blocking below and the frames arm's own
                // spawn_blocking (above) are not mutually exclusive, so if
                // `date` equals the night currently being accumulated, both
                // can run concurrently and each does its own
                // status::load -> mutate -> status::save on processing.json.
                // Unique tmp names (status::save) close the atomic-write
                // corruption risk, but a lost update is still possible: a
                // live write_artifacts can load a stale snapshot (before this
                // rebuild's finalize_night clears `timelapse`), then save it
                // back after the rebuild's own save, re-persisting a stale
                // Generating flag. Net effect: the UI can show the timelapse
                // "generating" until the next dawn finalize, which always
                // re-derives and re-saves status fresh. Self-heals; no data
                // loss (frames stay on disk); accepted as documented per the
                // final Phase 3 review (finding I-1) rather than fixed by
                // excluding the open night from rebuild.
                maybe_cmd = commands_rx.recv(), if pending_rebuild.is_none() => {
                    let Some(Command::Rebuild { date }) = maybe_cmd else { break };
                    let p = cfg.read().await.settings.processing.clone();
                    let dd = data_dir.clone();
                    let ffmpeg = pc.ffmpeg.clone();
                    let dir = night_dir(&dd, date);
                    // Mark every enabled artifact as generating up front so the
                    // UI shows progress immediately.
                    let mut progress = status::load(&dir);
                    if p.keogram { progress.keogram = Some(status::ArtifactProgress::Generating); }
                    if p.startrails { progress.startrails = Some(status::ArtifactProgress::Generating); }
                    if p.timelapse { progress.timelapse = Some(status::ArtifactProgress::Generating); }
                    if let Err(e) = status::save(&dir, &progress) {
                        tracing::error!("writing {}: {e:#}", status::STATUS_FILE);
                    }
                    // Run the replay+ffmpeg off the select loop so live frames
                    // and the dawn tick keep flowing while it's in flight.
                    pending_rebuild = Some(tokio::task::spawn_blocking(move || {
                        let st = replay_night(&dd, date, &p); // clears keogram/startrails flags
                        finalize_night(&dd, date, &p, &ffmpeg); // timelapse + its flag
                        st
                    }));
                }
                res = async { pending_rebuild.as_mut().expect("guarded by is_some").await }, if pending_rebuild.is_some() => {
                    pending_rebuild = None;
                    match res {
                        Ok(st) => {
                            // Adopt the rebuilt state only if it's the night
                            // currently being accumulated. Known accepted edge:
                            // if this IS the currently-open night, a live frame
                            // applied between the replay's disk read and this
                            // adoption is dropped from the in-memory
                            // accumulators (it's still persisted to disk, so it
                            // self-heals on any later replay/rebuild).
                            if state.as_ref().is_some_and(|s| s.date == st.date) {
                                state = Some(st);
                            }
                        }
                        Err(e) => tracing::error!("rebuild panicked: {e}"),
                    }
                }
                _ = tick.tick() => {
                    let Some(st) = state.as_ref() else { continue };
                    let (p, loc) = {
                        let g = cfg.read().await;
                        (g.settings.processing.clone(), g.settings.location)
                    };
                    let now_night = crate::capture::is_night(
                        chrono::Utc::now(),
                        loc.latitude_deg,
                        loc.longitude_deg,
                    );
                    let date_moved =
                        crate::capture::night_date(chrono::Local::now()) != st.date;
                    if now_night && !date_moved {
                        continue; // night still running
                    }
                    let date = st.date;
                    state = None;
                    last_finalized = Some(date);
                    let dd = data_dir.clone();
                    let ffmpeg = pc.ffmpeg.clone();
                    if let Err(e) = tokio::task::spawn_blocking(move || {
                        finalize_night(&dd, date, &p, &ffmpeg)
                    })
                    .await
                    {
                        tracing::error!("night finalize panicked: {e}");
                    }
                }
            }
        }
    });

    ProcessingHandle {
        frames: frames_tx,
        commands: commands_tx,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn fixture_ffmpeg() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-ffmpeg")
    }

    fn test_cfg() -> Arc<RwLock<crate::settings::ConfigFile>> {
        Arc::new(RwLock::new(crate::settings::ConfigFile {
            version: 1,
            password_hash: "h".into(),
            settings: crate::settings::Settings::default(),
        }))
    }

    /// Write a frame to disk + jsonl the way the capture loop does, and
    /// return the NightFrame the tap would send for it. Mimics the
    /// persist-then-tap invariant.
    fn seed_frame(
        data_dir: &std::path::Path,
        date: &str,
        file: &str,
        color: [u8; 3],
    ) -> NightFrame {
        let img = RgbImage::from_pixel(8, 6, Rgb(color));
        let night = data_dir.join("images").join(date);
        std::fs::create_dir_all(night.join("frames")).unwrap();
        img.save_with_format(night.join("frames").join(file), image::ImageFormat::Jpeg)
            .unwrap();
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(night.join("frames.jsonl"))
            .unwrap();
        writeln!(
            f,
            "{}",
            serde_json::json!({"timestamp": "t", "file": file, "exposureUs": 1, "gain": 1.0})
        )
        .unwrap();
        NightFrame {
            date: chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            file: file.to_string(),
            image: img,
            mean: crate::camera::mean_brightness(&RgbImage::from_pixel(8, 6, Rgb(color))),
        }
    }

    async fn wait_for<F: Fn() -> bool>(what: &str, f: F) {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if f() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
    }

    #[tokio::test]
    async fn frames_grow_keogram_and_startrails_on_disk() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = test_cfg();
        let date = crate::capture::night_date(chrono::Local::now()).to_string();
        let h = spawn_processing(
            cfg,
            dir.path().to_path_buf(),
            ProcessingConfig {
                ffmpeg: fixture_ffmpeg(),
                dawn_check: std::time::Duration::from_secs(3600), // never ticks in this test
            },
        );
        let night = dir.path().join("images").join(&date);

        let f1 = seed_frame(dir.path(), &date, "20260716-220000.jpg", [10, 10, 10]);
        h.frames.send(f1).await.unwrap();
        wait_for("keogram after first frame", || {
            night.join("keogram.jpg").is_file()
        })
        .await;

        let f2 = seed_frame(dir.path(), &date, "20260716-220100.jpg", [30, 30, 30]);
        h.frames.send(f2).await.unwrap();
        wait_for("keogram grows to 2 columns", || {
            image::open(night.join("keogram.jpg"))
                .map(|i| i.width() == 2)
                .unwrap_or(false)
        })
        .await;
        assert!(night.join("startrails.jpg").is_file());
        // startrails lighten-blend: brighter second frame wins
        let st = image::open(night.join("startrails.jpg")).unwrap().to_rgb8();
        assert!(st.get_pixel(4, 3).0[0] >= 25); // jpeg-lossy but near 30
    }

    #[tokio::test]
    async fn replay_covers_frames_persisted_before_startup() {
        // Two frames already on disk (e.g. rskycam restarted mid-night); the
        // tap only delivers the third. The keogram must still have 3 columns.
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = test_cfg();
        let date = crate::capture::night_date(chrono::Local::now()).to_string();
        seed_frame(dir.path(), &date, "20260716-220000.jpg", [10, 10, 10]);
        seed_frame(dir.path(), &date, "20260716-220100.jpg", [20, 20, 20]);
        let h = spawn_processing(
            cfg,
            dir.path().to_path_buf(),
            ProcessingConfig {
                ffmpeg: fixture_ffmpeg(),
                dawn_check: std::time::Duration::from_secs(3600),
            },
        );
        let f3 = seed_frame(dir.path(), &date, "20260716-220200.jpg", [30, 30, 30]);
        h.frames.send(f3).await.unwrap();
        let night = dir.path().join("images").join(&date);
        wait_for("keogram with 3 columns after replay", || {
            image::open(night.join("keogram.jpg"))
                .map(|i| i.width() == 3)
                .unwrap_or(false)
        })
        .await;
    }

    #[tokio::test]
    async fn dawn_tick_finalizes_a_past_night_with_a_timelapse() {
        // A frame for YESTERDAY's night: the very next tick sees
        // night_date(now) != state.date and finalizes → fake ffmpeg runs.
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = test_cfg();
        let yesterday =
            (crate::capture::night_date(chrono::Local::now()) - chrono::Days::new(1)).to_string();
        let h = spawn_processing(
            cfg,
            dir.path().to_path_buf(),
            ProcessingConfig {
                ffmpeg: fixture_ffmpeg(),
                dawn_check: std::time::Duration::from_millis(100),
            },
        );
        let f = seed_frame(dir.path(), &yesterday, "20260715-220000.jpg", [10, 10, 10]);
        h.frames.send(f).await.unwrap();
        let night = dir.path().join("images").join(&yesterday);
        wait_for("timelapse after dawn finalize", || {
            night.join("timelapse.mp4").is_file()
        })
        .await;
        // status file must not be stuck in generating
        let st = status::load(&night);
        assert_eq!(st.timelapse, None, "generating flag must be cleared");
    }

    #[tokio::test]
    async fn timelapse_failure_lands_in_the_status_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = test_cfg();
        let yesterday =
            (crate::capture::night_date(chrono::Local::now()) - chrono::Days::new(1)).to_string();
        // Failure marker for the fake ffmpeg.
        let night = dir.path().join("images").join(&yesterday);
        std::fs::create_dir_all(&night).unwrap();
        std::fs::write(night.join("fake-ffmpeg-fail"), b"").unwrap();
        let h = spawn_processing(
            cfg,
            dir.path().to_path_buf(),
            ProcessingConfig {
                ffmpeg: fixture_ffmpeg(),
                dawn_check: std::time::Duration::from_millis(100),
            },
        );
        let f = seed_frame(dir.path(), &yesterday, "20260715-220000.jpg", [10, 10, 10]);
        h.frames.send(f).await.unwrap();
        wait_for("timelapse error recorded", || {
            matches!(
                status::load(&night).timelapse,
                Some(status::ArtifactProgress::Error { .. })
            )
        })
        .await;
        assert!(!night.join("timelapse.mp4").exists());
    }

    #[tokio::test]
    async fn finalized_night_is_not_reopened_by_late_frames() {
        // captureDuringDay means frames for an already-finalized night keep
        // arriving until noon; they must NOT re-replay the night or re-run ffmpeg.
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = test_cfg();
        let yesterday =
            (crate::capture::night_date(chrono::Local::now()) - chrono::Days::new(1)).to_string();
        let h = spawn_processing(
            cfg,
            dir.path().to_path_buf(),
            ProcessingConfig {
                ffmpeg: fixture_ffmpeg(),
                dawn_check: std::time::Duration::from_millis(100),
            },
        );
        let f = seed_frame(dir.path(), &yesterday, "20260715-220000.jpg", [10, 10, 10]);
        h.frames.send(f).await.unwrap();
        let night = dir.path().join("images").join(&yesterday);
        wait_for("first finalize", || night.join("timelapse.mp4").is_file()).await;

        // Remove the output, send a late frame for the same (finalized) night,
        // and give the supervisor several ticks: nothing may be regenerated.
        std::fs::remove_file(night.join("timelapse.mp4")).unwrap();
        let late = seed_frame(dir.path(), &yesterday, "20260715-230000.jpg", [20, 20, 20]);
        h.frames.send(late).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        assert!(
            !night.join("timelapse.mp4").exists(),
            "finalized night was re-finalized by a late frame"
        );
        // And the keogram was not re-replayed with the late frame appended.
        assert_eq!(image::open(night.join("keogram.jpg")).unwrap().width(), 1);
    }

    #[tokio::test]
    async fn rebuild_command_regenerates_all_enabled_artifacts() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = test_cfg();
        let date = "2026-07-10"; // an old, closed night
        seed_frame(dir.path(), date, "20260710-220000.jpg", [10, 10, 10]);
        seed_frame(dir.path(), date, "20260710-220100.jpg", [20, 20, 20]);
        let h = spawn_processing(
            cfg,
            dir.path().to_path_buf(),
            ProcessingConfig {
                ffmpeg: fixture_ffmpeg(),
                dawn_check: std::time::Duration::from_secs(3600),
            },
        );
        h.commands
            .send(Command::Rebuild {
                date: chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            })
            .await
            .unwrap();
        let night = dir.path().join("images").join(date);
        wait_for("all three artifacts after rebuild", || {
            night.join("keogram.jpg").is_file()
                && night.join("startrails.jpg").is_file()
                && night.join("timelapse.mp4").is_file()
        })
        .await;
        let st = status::load(&night);
        assert_eq!(
            (st.keogram, st.startrails, st.timelapse),
            (None, None, None)
        );
        assert_eq!(image::open(night.join("keogram.jpg")).unwrap().width(), 2);
    }

    #[tokio::test]
    async fn rebuild_of_an_empty_night_clears_the_generating_flags() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = test_cfg();
        // Night dir exists but has no frames.jsonl / no decodable frames.
        let night = dir.path().join("images").join("2026-07-01");
        std::fs::create_dir_all(&night).unwrap();
        let h = spawn_processing(
            cfg,
            dir.path().to_path_buf(),
            ProcessingConfig {
                ffmpeg: fixture_ffmpeg(),
                dawn_check: std::time::Duration::from_secs(3600),
            },
        );
        h.commands
            .send(Command::Rebuild {
                date: chrono::NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            })
            .await
            .unwrap();
        // Wait for the rebuild to fully finish (timelapse.mp4 is written last,
        // by finalize_night, so this can't race the up-front Generating write
        // the way polling keogram/startrails directly from t=0 would: a fresh
        // dir has no processing.json yet, and status::load defaults absent
        // fields to None, which would let a still-stuck flag slip past a
        // check that runs before the rebuild has even started).
        wait_for("rebuild completes", || {
            night.join("timelapse.mp4").is_file()
        })
        .await;
        let st = status::load(&night);
        assert!(st.keogram.is_none(), "keogram flag stuck: {:?}", st.keogram);
        assert!(
            st.startrails.is_none(),
            "startrails flag stuck: {:?}",
            st.startrails
        );
        assert!(!night.join("keogram.jpg").exists());
        assert!(!night.join("startrails.jpg").exists());
    }

    #[tokio::test]
    async fn live_frames_flow_while_a_rebuild_is_in_flight() {
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = test_cfg();
        let today = crate::capture::night_date(chrono::Local::now()).to_string();
        // An old night to rebuild, with a slow (2 s) fake ffmpeg.
        let old = "2026-07-01";
        seed_frame(dir.path(), old, "20260701-220000.jpg", [10, 10, 10]);
        std::fs::write(
            dir.path()
                .join("images")
                .join(old)
                .join("fake-ffmpeg-sleep"),
            b"2",
        )
        .unwrap();
        let h = spawn_processing(
            cfg,
            dir.path().to_path_buf(),
            ProcessingConfig {
                ffmpeg: fixture_ffmpeg(),
                dawn_check: std::time::Duration::from_secs(3600),
            },
        );
        h.commands
            .send(Command::Rebuild {
                date: chrono::NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            })
            .await
            .unwrap();
        // Make sure the supervisor has actually picked up the rebuild before
        // sending the live frame: the commands arm synchronously pre-marks the
        // artifacts Generating before spawning the blocking work. Without this,
        // select! could race and process the frame before the rebuild starts,
        // which would let a loop-blocking regression slip through undetected.
        let old_night = dir.path().join("images").join(old);
        wait_for("rebuild picked up", || {
            status::load(&old_night).timelapse == Some(status::ArtifactProgress::Generating)
        })
        .await;
        // While the rebuild sleeps in ffmpeg, a live frame for TODAY must still be processed.
        let started = std::time::Instant::now();
        let f = seed_frame(dir.path(), &today, "20260716-220000.jpg", [30, 30, 30]);
        h.frames.send(f).await.unwrap();
        let today_night = dir.path().join("images").join(&today);
        wait_for("live keogram during rebuild", || {
            today_night.join("keogram.jpg").is_file()
        })
        .await;
        // The live frame must land well inside the 2 s fake-ffmpeg sleep. If the
        // supervisor awaited the rebuild inline, the frame would only drain after
        // the sleep (~2 s), so a 1.5 s bound genuinely discriminates.
        assert!(
            started.elapsed() < std::time::Duration::from_millis(1500),
            "live frame was stalled behind the rebuild ({:?})",
            started.elapsed()
        );
        // Direct proof the rebuild hadn't finished when the live frame landed.
        assert!(
            !old_night.join("timelapse.mp4").exists(),
            "rebuild finished before the live frame — nothing was in flight"
        );
        // Then the rebuild completes normally.
        wait_for("rebuild finishes", || {
            old_night.join("timelapse.mp4").is_file()
        })
        .await;
    }
}
