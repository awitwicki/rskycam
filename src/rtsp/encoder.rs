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
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("spawning ffmpeg: {e}"))?;

    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");
    let fps = cfg.fps.max(1);

    let feeder = tokio::spawn(async move {
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
    });

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
    feeder.abort();

    let wait_result = child
        .wait()
        .await
        .map_err(|e| format!("waiting for ffmpeg: {e}"));
    read_result?;
    match wait_result? {
        status if status.success() => Ok(()),
        status => Err(format!("ffmpeg exited with {status}")),
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
        }
    }

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-ffmpeg-h264")
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
