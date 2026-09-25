use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::watch;

use super::{rtp, Shared, SSRC};
use crate::capture::LatestFrame;

pub struct EncoderConfig {
    pub ffmpeg: PathBuf,
    pub fps: u32,
    pub output_width: u32,
    pub bitrate_kbps: u32,
    pub extra_args: String,
}

pub fn build_ffmpeg_args(cfg: &EncoderConfig) -> Vec<OsString> {
    let scale = if cfg.output_width == 0 {
        "scale=trunc(iw/2)*2:trunc(ih/2)*2".to_string()
    } else {
        format!("scale={}:-2", cfg.output_width)
    };
    let mut v: Vec<OsString> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostdin".into(),
        "-f".into(),
        "image2pipe".into(),
        // image2pipe infers its input codec from -i's filename extension;
        // stdin ("-") has none, so without an explicit input codec ffmpeg
        // can't tell what it's decoding and aborts before opening the
        // output ("Decoding requested, but no decoder found for: none").
        // This must come BEFORE -i, since -c:v is positional in ffmpeg's
        // argv and applies to whichever input/output it precedes.
        "-c:v".into(),
        "mjpeg".into(),
        "-framerate".into(),
        cfg.fps.to_string().into(),
        "-i".into(),
        "-".into(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "ultrafast".into(),
        "-tune".into(),
        "zerolatency".into(),
        // Forces one slice per frame so a VCL NAL always carries the whole
        // picture -- run_encoder's access-unit grouping relies on exactly
        // one VCL NAL (type 1 or 5) closing out each access unit.
        "-x264-params".into(),
        "slices=1".into(),
        "-bf".into(),
        "0".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-g".into(),
        (cfg.fps * 2).to_string().into(),
        "-b:v".into(),
        format!("{}k", cfg.bitrate_kbps).into(),
        "-vf".into(),
        scale.into(),
    ];
    v.extend(cfg.extra_args.split_whitespace().map(OsString::from));
    v.push("-f".into());
    v.push("h264".into());
    v.push("-".into());
    v
}

/// How much of ffmpeg's stderr is retained for the error message. Enough for
/// the handful of lines `-loglevel error` emits; the reader keeps draining
/// past this so ffmpeg never blocks on a full stderr pipe.
const STDERR_CAP: usize = 1024;

/// Aborts the wrapped task when dropped. `run_encoder`'s whole future can be
/// dropped mid-`.await` by the caller's `tokio::select!` (the idle stop, or
/// `rtsp.enabled` going false), and a bare `JoinHandle` merely *detaches* on
/// drop rather than cancelling. The feeder would then keep looping at `fps`
/// Hz forever whenever `latest` has never carried a frame -- there is no
/// `write_all` to hit EPIPE on the killed child in that state, so one task
/// and one `ChildStdin` fd would leak per stop/start cycle.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Spawns ffmpeg, feeds it the latest `stream_jpeg` at `cfg.fps`, reads its
/// Annex-B stdout, packetizes each completed access unit and broadcasts it.
/// Returns when the process exits (`Err` if it exited non-zero or a spawn
/// step failed) or is dropped mid-`.await` by the caller's `tokio::select!`
/// (the child is killed on drop via `kill_on_drop(true)`).
pub async fn run_encoder(
    cfg: &EncoderConfig,
    latest: watch::Receiver<Option<Arc<LatestFrame>>>,
    shared: &Shared,
) -> Result<(), String> {
    let mut child = Command::new("nice")
        .arg("-n")
        .arg("10")
        .arg(&cfg.ffmpeg)
        .args(build_ffmpeg_args(cfg))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("spawning ffmpeg: {e}"))?;

    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let fps = cfg.fps.max(1);

    // Without this, a non-zero exit is reported as a bare status code with no
    // hint of the cause -- and `nice` turns a missing/unrunnable ffmpeg into
    // an unexplained 127.
    let mut stderr_task = AbortOnDrop(tokio::spawn(async move {
        let mut collected: Vec<u8> = Vec::new();
        let mut buf = [0u8; 512];
        loop {
            match stderr.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if collected.len() < STDERR_CAP {
                        let take = n.min(STDERR_CAP - collected.len());
                        collected.extend_from_slice(&buf[..take]);
                    }
                }
            }
        }
        String::from_utf8_lossy(&collected).trim().to_string()
    }));

    let feeder = AbortOnDrop(tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / f64::from(fps)));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let frame = latest.borrow().clone();
            let Some(frame) = frame else { continue };
            if stdin.write_all(&frame.stream_jpeg).await.is_err() {
                return; // ffmpeg exited -- the reader loop below will notice via EOF
            }
        }
    }));

    let mut splitter = rtp::NalSplitter::new();
    let mut pending: Vec<Vec<u8>> = Vec::new();
    let mut buf = [0u8; 65536];
    let read_result: Result<(), String> = loop {
        match stdout.read(&mut buf).await {
            Ok(0) => break Ok(()), // EOF
            Ok(n) => {
                for nal in splitter.feed(&buf[..n]) {
                    handle_nal(nal, &mut pending, shared, fps);
                }
            }
            Err(e) => break Err(format!("reading ffmpeg stdout: {e}")),
        }
    };
    if let Some(nal) = splitter.flush() {
        handle_nal(nal, &mut pending, shared, fps);
    }
    drop(feeder); // AbortOnDrop -- stops it whichever way we leave this future

    let wait_result = child
        .wait()
        .await
        .map_err(|e| format!("waiting for ffmpeg: {e}"));
    // The child has exited by now, so its stderr is at EOF and this joins
    // promptly rather than waiting on a live pipe.
    let stderr_text = (&mut stderr_task.0).await.unwrap_or_default();
    read_result?;
    match wait_result? {
        status if status.success() => Ok(()),
        status if stderr_text.is_empty() => Err(format!("ffmpeg exited with {status}")),
        status => Err(format!("ffmpeg exited with {status}: {stderr_text}")),
    }
}

/// SPS(7)/PPS(8) are latched for the SDP as they arrive; any NAL is buffered
/// into the pending access unit, and a VCL slice NAL (type 1 or 5) closes it
/// out -- `-x264-params slices=1` guarantees exactly one VCL NAL per frame.
fn handle_nal(nal: Vec<u8>, pending: &mut Vec<Vec<u8>>, shared: &Shared, fps: u32) {
    let ty = rtp::nal_type(&nal);
    if ty == 7 || ty == 8 {
        let mut s = shared.stream.lock().unwrap();
        if ty == 7 {
            s.sps = Some(nal.clone());
        } else {
            s.pps = Some(nal.clone());
        }
    }
    let is_vcl = ty == 1 || ty == 5;
    pending.push(nal);
    if !is_vcl {
        return;
    }
    let packets = {
        let mut c = shared.counters.lock().unwrap();
        let packets = rtp::packetize_access_unit(pending, c.seq, c.timestamp, SSRC);
        c.seq = c.seq.wrapping_add(packets.len() as u16);
        c.timestamp = c.timestamp.wrapping_add(90_000 / fps);
        packets
    };
    for p in &packets {
        let _ = shared.tx.send(rtp::interleave(0, p));
    }
    pending.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tokio::sync::Notify;

    fn fixture_shared() -> Shared {
        Shared {
            tx: tokio::sync::broadcast::channel(64).0,
            stream: std::sync::Mutex::new(super::super::StreamState {
                sps: None,
                pps: None,
            }),
            counters: std::sync::Mutex::new(super::super::Counters {
                seq: 0,
                timestamp: 0,
            }),
            interest: AtomicUsize::new(1),
            clients: AtomicUsize::new(0),
            want_encoder: Notify::new(),
            status_tx: watch::channel(super::super::RtspStatus::default()).0,
            enabled: watch::channel(true).0,
        }
    }

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-ffmpeg-h264")
    }

    /// A real ffmpeg, if this machine has one. Deliberately not a hard
    /// requirement: the Pi's own checkout and a bare dev machine may not have
    /// it installed, and the test that uses it skips rather than fails there.
    fn real_ffmpeg() -> Option<PathBuf> {
        [
            "/opt/homebrew/bin/ffmpeg",
            "/usr/local/bin/ffmpeg",
            "/usr/bin/ffmpeg",
        ]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
    }

    fn capture_shaped_frame() -> Arc<LatestFrame> {
        let jpeg = std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/frame-64x48.jpg"),
        )
        .unwrap();
        let bytes = bytes::Bytes::from(jpeg);
        Arc::new(LatestFrame {
            jpeg: bytes.clone(),
            persist_jpeg: bytes.clone(),
            raw_jpeg: bytes.clone(),
            stream_jpeg: bytes,
            raw_width: 64,
            raw_height: 48,
            meta: crate::capture::FrameMeta {
                timestamp: "2026-08-30T00:00:00Z".to_string(),
                exposure_us: 1000,
                gain: 1.0,
                is_night: true,
                metered_mean: 0.0,
                metered_area_pct: 100.0,
            },
        })
    }

    #[test]
    fn build_ffmpeg_args_uses_native_scale_when_output_width_is_zero() {
        let cfg = EncoderConfig {
            ffmpeg: "ffmpeg".into(),
            fps: 5,
            output_width: 0,
            bitrate_kbps: 2000,
            extra_args: String::new(),
        };
        let args = build_ffmpeg_args(&cfg);
        let joined: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(joined.contains(&"scale=trunc(iw/2)*2:trunc(ih/2)*2".to_string()));
        assert!(joined.contains(&"5".to_string())); // -framerate 5 and -g 10 both appear
        assert!(joined.contains(&"10".to_string()));
        assert!(joined.contains(&"2000k".to_string()));
    }

    #[test]
    fn build_ffmpeg_args_uses_explicit_width_when_set() {
        let cfg = EncoderConfig {
            ffmpeg: "ffmpeg".into(),
            fps: 5,
            output_width: 960,
            bitrate_kbps: 2000,
            extra_args: "-an".to_string(),
        };
        let args = build_ffmpeg_args(&cfg);
        let joined: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(joined.contains(&"scale=960:-2".to_string()));
        assert!(joined.contains(&"-an".to_string()));
    }

    /// `-c:v` is positional in ffmpeg's argv: the one before `-i` selects the
    /// *input* decoder, the one after it selects the output encoder. Without
    /// the input-side pair, `image2pipe` has no filename extension to infer a
    /// decoder from and ffmpeg aborts before ever opening the output. Assert
    /// the exact position, not just presence -- `-c:v libx264` alone would
    /// satisfy a "contains" check while leaving the input undecodable.
    #[test]
    fn build_ffmpeg_args_declares_the_input_codec_before_the_input() {
        let cfg = EncoderConfig {
            ffmpeg: "ffmpeg".into(),
            fps: 5,
            output_width: 0,
            bitrate_kbps: 2000,
            extra_args: String::new(),
        };
        let joined: Vec<String> = build_ffmpeg_args(&cfg)
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let input_at = joined.iter().position(|a| a == "-i").expect("an -i flag");
        assert_eq!(joined[input_at + 1], "-", "input must be stdin");
        let input_codec_at = joined[..input_at]
            .iter()
            .position(|a| a == "-c:v")
            .expect("an input-side -c:v before -i");
        assert_eq!(joined[input_codec_at + 1], "mjpeg");
        // ...and the output encoder still follows the input, unaffected.
        let output_codec_at = input_at
            + 1
            + joined[input_at + 1..]
                .iter()
                .position(|a| a == "-c:v")
                .expect("an output-side -c:v after -i");
        assert_eq!(joined[output_codec_at + 1], "libx264");
    }

    /// The only encoder test that runs the argv `build_ffmpeg_args` actually
    /// produces against a real ffmpeg. Every other one uses
    /// `tests/fixtures/fake-ffmpeg-h264`, which ignores argv entirely -- which
    /// is exactly how a missing input-side `-c:v` shipped undetected.
    #[tokio::test]
    async fn run_encoder_broadcasts_rtp_from_a_real_ffmpeg() {
        let Some(ffmpeg) = real_ffmpeg() else {
            eprintln!("skipping: no ffmpeg installed on this machine");
            return;
        };
        let shared = fixture_shared();
        let (latest_tx, latest_rx) = watch::channel(None);
        latest_tx.send(Some(capture_shaped_frame())).unwrap();
        let mut rx = shared.tx.subscribe();
        let cfg = EncoderConfig {
            ffmpeg,
            fps: 10,
            output_width: 0,
            bitrate_kbps: 500,
            extra_args: String::new(),
        };
        let mut run = Box::pin(run_encoder(&cfg, latest_rx, &shared));
        let framed = tokio::select! {
            r = &mut run => panic!("real ffmpeg exited before emitting a frame: {r:?}"),
            recv = tokio::time::timeout(Duration::from_secs(15), rx.recv()) => {
                recv.expect("timed out waiting for RTP from real ffmpeg").unwrap()
            }
        };
        assert_eq!(framed[0], b'$'); // interleaved RTP framing
        assert_eq!(framed[4 + 1] & 0x7f, 96); // dynamic H.264 payload type
                                              // Real x264 emits its parameter sets at the head of the stream, so
                                              // both must be latched by the time the first access unit lands.
        assert!(
            shared.stream.lock().unwrap().sps.is_some(),
            "no SPS latched"
        );
        assert!(
            shared.stream.lock().unwrap().pps.is_some(),
            "no PPS latched"
        );
        drop(run);
    }

    /// The idle-stop / disable path drops `run_encoder`'s whole future rather
    /// than letting it return, and `JoinHandle::drop` detaches instead of
    /// cancelling. With `latest` never carrying a frame the feeder has no
    /// `write_all` to fail on, so nothing else would ever stop it.
    #[tokio::test]
    async fn feeder_task_is_cancelled_when_the_run_encoder_future_is_dropped() {
        let shared = fixture_shared();
        let (latest_tx, latest_rx) = watch::channel(None); // stays None for the whole test
        let cfg = EncoderConfig {
            ffmpeg: fixture_path(),
            fps: 20,
            output_width: 0,
            bitrate_kbps: 2000,
            extra_args: String::new(),
        };
        let mut run = Box::pin(run_encoder(&cfg, latest_rx, &shared));
        // Poll it long enough to have spawned the feeder, which owns the only
        // `latest` receiver -- so the receiver count is a direct, external
        // read on whether that task is still alive.
        tokio::select! {
            r = &mut run => panic!("encoder exited before the feeder could start: {r:?}"),
            _ = tokio::time::sleep(Duration::from_millis(300)) => {}
        }
        assert_eq!(latest_tx.receiver_count(), 1, "feeder never started");

        drop(run);
        let mut gone = false;
        for _ in 0..100 {
            if latest_tx.receiver_count() == 0 {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            gone,
            "feeder task leaked after run_encoder's future was dropped"
        );
    }

    #[tokio::test]
    async fn run_encoder_latches_sps_pps_and_broadcasts_packets() {
        let shared = fixture_shared();
        let (_latest_tx, latest_rx) = watch::channel(None);
        let mut rx = shared.tx.subscribe();
        let cfg = EncoderConfig {
            ffmpeg: fixture_path(),
            fps: 5,
            output_width: 0,
            bitrate_kbps: 2000,
            extra_args: String::new(),
        };
        // `run_encoder` borrows `&Shared`, so it runs pinned in this task
        // rather than spawned onto a separate ('static-bound) one; racing
        // it against the broadcast receive is enough to prove it emits a
        // frame without needing the whole call to return first. `Box::pin`
        // rather than `tokio::pin!` so the `drop` below drops the *future*
        // (and with it the child) -- `tokio::pin!` rebinds the name to a
        // `Pin<&mut _>`, which would make that drop a no-op.
        let mut run = Box::pin(run_encoder(&cfg, latest_rx, &shared));
        let framed = tokio::select! {
            r = &mut run => panic!("encoder exited before emitting a frame: {r:?}"),
            recv = rx.recv() => recv.unwrap(),
        };
        assert_eq!(framed[0], b'$'); // interleaved RTP framing
        assert!(shared.stream.lock().unwrap().sps.is_some());
        assert!(shared.stream.lock().unwrap().pps.is_some());
        drop(run); // kills the still-running fixture child via kill_on_drop
    }

    #[tokio::test]
    async fn fixture_process_emits_sps_pps_idr_over_stdout_while_draining_stdin() {
        let mut child = tokio::process::Command::new(fixture_path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let feeder = tokio::spawn(async move {
            for _ in 0..20 {
                if stdin.write_all(b"junk").await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });
        let mut splitter = rtp::NalSplitter::new();
        let mut buf = [0u8; 4096];
        let mut types = Vec::new();
        loop {
            let n = stdout.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            for nal in splitter.feed(&buf[..n]) {
                types.push(rtp::nal_type(&nal));
            }
        }
        if let Some(nal) = splitter.flush() {
            types.push(rtp::nal_type(&nal));
        }
        feeder.await.unwrap();
        child.wait().await.unwrap();
        assert!(types.contains(&7)); // SPS
        assert!(types.contains(&8)); // PPS
        assert!(types.contains(&5)); // IDR
    }
}
