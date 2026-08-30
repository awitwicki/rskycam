pub mod digest;
mod encoder;
pub mod jpeg;
pub mod rtp;
pub mod sdp;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use rand::RngCore;
use rtsp_types::{headers, Message, Method, Request, StatusCode as RtspStatusCode, Version};
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, watch, Notify, RwLock};

use crate::capture::LatestFrame;
use crate::settings::ConfigFile;

const MAX_CLIENTS: usize = 4;
const IDLE_STOP_AFTER: Duration = Duration::from_secs(10);
const SSRC: u32 = 0x5253_4b59; // arbitrary, fixed for the process lifetime

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RtspStatus {
    pub enabled: bool,
    pub listening: bool,
    pub port: u16,
    pub username: String,
    pub clients: usize,
    pub encoding: bool,
    pub last_error: Option<String>,
}

pub struct RtspHandle {
    pub status: watch::Receiver<RtspStatus>,
}

struct Counters {
    seq: u16,
    timestamp: u32,
}

struct StreamState {
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
}

struct Shared {
    tx: broadcast::Sender<Bytes>,
    stream: std::sync::Mutex<StreamState>,
    counters: std::sync::Mutex<Counters>,
    /// Connections that have sent at least one `DESCRIBE` and haven't
    /// disconnected yet -- drives the lazy encoder start/stop.
    interest: AtomicUsize,
    /// Sessions currently in `PLAY` state -- what `RtspStatus.clients`
    /// reports and what the 4-client cap checks.
    clients: AtomicUsize,
    want_encoder: Notify,
    status_tx: watch::Sender<RtspStatus>,
    /// Mirrors `settings.rtsp.enabled`, published by `listener_task` (which
    /// is the only thing that reads the setting). Flipping it to `false`
    /// tears down every live connection and stops the encoder; dropping the
    /// listener alone would leave already-connected sessions streaming.
    /// Starts `false` so nothing runs before the listener has looked at the
    /// config -- no connection can exist before it binds, and it only binds
    /// after publishing `true`.
    enabled: watch::Sender<bool>,
}

/// Resolves once `enabled` has been observed as `false` (immediately if it
/// already is). The borrow guard `wait_for` hands back is dropped at the end
/// of this statement, never held across the caller's `.await`.
async fn wait_until_disabled(rx: &mut watch::Receiver<bool>) {
    let _ = rx.wait_for(|enabled| !*enabled).await;
}

fn publish_status(shared: &Shared, edit: impl FnOnce(&mut RtspStatus)) {
    shared.status_tx.send_modify(edit);
}

/// `publish_status` uses `send_modify`, which notifies watchers whether or
/// not anything actually changed. The username republish below runs on the
/// listener's 2s recheck, so it needs the change-gated form -- otherwise the
/// status channel would tick every 2s forever, waking every watcher and
/// destroying any "has the server gone quiet" signal.
fn publish_username(shared: &Shared, username: String) {
    shared.status_tx.send_if_modified(|s| {
        if s.username == username {
            return false;
        }
        s.username = username;
        true
    });
}

fn publish_client_count(shared: &Shared) {
    let n = shared.clients.load(Ordering::SeqCst);
    publish_status(shared, |s| s.clients = n);
}

fn new_random_token() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn spawn_rtsp(
    cfg: Arc<RwLock<ConfigFile>>,
    latest: watch::Receiver<Option<Arc<LatestFrame>>>,
    ffmpeg: PathBuf,
) -> RtspHandle {
    let (status_tx, status_rx) = watch::channel(RtspStatus::default());
    let (tx, _rx) = broadcast::channel::<Bytes>(256);
    let shared = Arc::new(Shared {
        tx,
        stream: std::sync::Mutex::new(StreamState {
            sps: None,
            pps: None,
        }),
        counters: std::sync::Mutex::new(Counters {
            seq: 0,
            timestamp: 0,
        }),
        interest: AtomicUsize::new(0),
        clients: AtomicUsize::new(0),
        want_encoder: Notify::new(),
        status_tx,
        enabled: watch::channel(false).0,
    });

    tokio::spawn(listener_task(cfg.clone(), shared.clone()));
    tokio::spawn(encoder_supervisor_task(cfg, latest, ffmpeg, shared));

    RtspHandle { status: status_rx }
}

/// Runs the encoder for as long as anything is interested in it (see
/// `Shared::interest`), restarting it with exponential backoff when it dies
/// unexpectedly and stopping it (no backoff, no recorded error) once the
/// last interested connection has been gone for `IDLE_STOP_AFTER`.
async fn encoder_supervisor_task(
    cfg: Arc<RwLock<ConfigFile>>,
    latest: watch::Receiver<Option<Arc<LatestFrame>>>,
    ffmpeg: PathBuf,
    shared: Arc<Shared>,
) {
    let mut backoff = Duration::from_secs(1);
    let mut enabled_rx = shared.enabled.subscribe();
    loop {
        wait_until_wanted(&shared, &mut enabled_rx).await;
        publish_status(&shared, |s| s.encoding = true);

        let stop = Arc::new(Notify::new());
        let watcher = tokio::spawn(watch_for_idle_and_stop(shared.clone(), stop.clone()));

        let enc_cfg = {
            let c = cfg.read().await;
            encoder::EncoderConfig {
                ffmpeg: ffmpeg.clone(),
                fps: c.settings.rtsp.fps.max(1),
                output_width: c.settings.rtsp.output_width,
                bitrate_kbps: c.settings.rtsp.bitrate_kbps,
                extra_args: c.settings.rtsp.extra_args.clone(),
            }
        };
        let result = tokio::select! {
            r = encoder::run_encoder(&enc_cfg, latest.clone(), &shared) => r,
            _ = stop.notified() => Ok(()), // deliberate idle stop, not an error
            // `rtsp.enabled` turned off: the connections are being torn down
            // in parallel, so drop the encoder future now (kill_on_drop takes
            // ffmpeg with it) rather than waiting out the idle debounce.
            _ = wait_until_disabled(&mut enabled_rx) => Ok(()),
        };
        watcher.abort();
        publish_status(&shared, |s| s.encoding = false);
        // The parameter sets belong to the process that just exited: a fresh
        // ffmpeg emits its own, and a DESCRIBE answered from stale ones would
        // hand a client an SDP that doesn't match the stream it then gets.
        {
            let mut s = shared.stream.lock().unwrap();
            s.sps = None;
            s.pps = None;
        }

        if shared.interest.load(Ordering::SeqCst) == 0 {
            // Idle stop (or a death that raced with the last client leaving):
            // nothing went wrong, so don't record an error or grow the backoff.
            backoff = Duration::from_secs(1);
            continue;
        }
        match result {
            Ok(()) => {
                backoff = Duration::from_secs(1);
                // A clean exit while something still wants the stream would
                // otherwise respawn as fast as fork+exec allows. `extra_args`
                // is user-editable, and a combination that makes ffmpeg exit 0
                // promptly is entirely reachable -- pace the retry the same way
                // the failure path's first step does.
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => {
                tracing::error!("rtsp encoder failed: {e}");
                publish_status(&shared, |s| s.last_error = Some(format!("encoder: {e}")));
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(30));
            }
        }
    }
}

/// Parks until RTSP is enabled *and* at least one connection has expressed
/// interest. `Notify` stores a permit when nobody is waiting, so a `DESCRIBE`
/// that lands between the load and the `await` still wakes this up rather
/// than being lost, and a dropped `Notified` hands its wakeup on rather than
/// swallowing it -- so losing the `select!` race below never loses a permit.
async fn wait_until_wanted(shared: &Shared, enabled_rx: &mut watch::Receiver<bool>) {
    loop {
        let enabled = *enabled_rx.borrow(); // guard dropped before the awaits below
        if enabled && shared.interest.load(Ordering::SeqCst) > 0 {
            return;
        }
        tokio::select! {
            _ = shared.want_encoder.notified() => {}
            _ = enabled_rx.changed() => {}
        }
    }
}

/// Notifies `stop` once interest has stayed at zero for `IDLE_STOP_AFTER`,
/// so a client that reconnects promptly keeps the same running encoder
/// instead of paying for a fresh ffmpeg start.
async fn watch_for_idle_and_stop(shared: Arc<Shared>, stop: Arc<Notify>) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if shared.interest.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(IDLE_STOP_AFTER).await;
            if shared.interest.load(Ordering::SeqCst) == 0 {
                stop.notify_one();
                return;
            }
        }
    }
}

async fn listener_task(cfg: Arc<RwLock<ConfigFile>>, shared: Arc<Shared>) {
    loop {
        let (enabled, port, username) = {
            let c = cfg.read().await;
            (
                c.settings.rtsp.enabled,
                c.settings.rtsp.port,
                c.rtsp_username.clone(),
            )
        };
        publish_status(&shared, |s| {
            s.enabled = enabled;
            s.port = port;
            s.username = username;
            if !enabled {
                s.listening = false;
            }
        });
        // Published *before* the bind below, so every connection this listener
        // goes on to accept observes `true` and none is torn down by the
        // disable path the instant it connects after a re-enable.
        shared
            .enabled
            .send_if_modified(|v| std::mem::replace(v, enabled) != enabled);
        if !enabled {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("rtsp: binding :{port} failed: {e}; retrying in 10s");
                publish_status(&shared, |s| {
                    s.listening = false;
                    s.last_error = Some(format!("binding :{port}: {e}"));
                });
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
        };
        // The bound port can differ from the configured one when the
        // config asks for port 0 (OS-assigned -- tests use this to avoid
        // port collisions); publish the port actually listening on.
        let bound_port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
        publish_status(&shared, |s| {
            s.listening = true;
            s.port = bound_port;
            s.last_error = None;
        });
        let mut recheck = tokio::time::interval(Duration::from_secs(2));
        recheck.tick().await; // consume the immediate first tick
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let Ok((stream, _peer)) = accepted else { continue };
                    // RTP is interleaved on this same control connection, so
                    // Nagle would coalesce (and delay) the small RTP writes.
                    // Advisory: an OS that refuses it just costs latency.
                    let _ = stream.set_nodelay(true);
                    let cfg = cfg.clone();
                    let shared = shared.clone();
                    tokio::spawn(async move {
                        let _ = handle_connection(stream, cfg, shared).await;
                    });
                }
                _ = recheck.tick() => {
                    let (still_enabled, cfg_port, username) = {
                        let c = cfg.read().await;
                        (
                            c.settings.rtsp.enabled,
                            c.settings.rtsp.port,
                            c.rtsp_username.clone(),
                        )
                    };
                    // The Settings page builds its copyable rtsp:// URL from
                    // this; without republishing here it would show the old
                    // username until the listener happened to rebind.
                    publish_username(&shared, username);
                    // Tears down live sessions and the encoder; the listener
                    // itself is dropped by breaking out of this loop.
                    shared
                        .enabled
                        .send_if_modified(|v| std::mem::replace(v, still_enabled) != still_enabled);
                    if !still_enabled || cfg_port != port {
                        break; // settings changed -- drop this listener and rebind above
                    }
                }
            }
        }
    }
}

#[derive(Default)]
struct ConnState {
    nonce: Option<String>,
    authenticated: bool,
    session: Option<String>,
    playing: bool,
    counted_interest: bool,
    play_task: Option<tokio::task::JoinHandle<()>>,
}

async fn handle_connection(
    stream: TcpStream,
    cfg: Arc<RwLock<ConfigFile>>,
    shared: Arc<Shared>,
) -> std::io::Result<()> {
    let (mut read_half, write_half) = stream.into_split();
    let (out_tx, mut out_rx) = mpsc::channel::<Bytes>(64);

    let writer = tokio::spawn(async move {
        let mut write_half = write_half;
        while let Some(bytes) = out_rx.recv().await {
            if write_half.write_all(&bytes).await.is_err() {
                break;
            }
        }
    });

    let mut buf: Vec<u8> = Vec::new();
    let mut read_buf = [0u8; 4096];
    let mut state = ConnState::default();
    let mut enabled_rx = shared.enabled.subscribe();

    let result = 'outer: loop {
        // Before PLAY, a client that vanishes without a clean TCP close would
        // otherwise pin `interest` forever (nothing is written on this socket
        // between DESCRIBE and PLAY, so the peer's absence is never noticed)
        // and defeat the lazy encoder's idle-stop. Once playing, drop the
        // timeout: RTP delivery doesn't depend on the read side, and a client
        // may legitimately stay quiet for a long time between GET_PARAMETERs.
        // `rtsp.enabled` going false must TEARDOWN every live session, not
        // just stop the listener accepting new ones -- otherwise RTP keeps
        // flowing and `interest` stays pinned above zero. Breaking out here
        // runs the same cleanup a client disconnect does (counters returned,
        // play task aborted, socket closed). `AsyncReadExt::read` is
        // cancel-safe, so losing this race never drops buffered bytes.
        let read_result = tokio::select! {
            biased;
            _ = wait_until_disabled(&mut enabled_rx) => break Ok(()),
            r = async {
                if state.playing {
                    read_half.read(&mut read_buf).await.map(Some)
                } else {
                    match tokio::time::timeout(
                        Duration::from_secs(60),
                        read_half.read(&mut read_buf),
                    )
                    .await
                    {
                        Ok(r) => r.map(Some),
                        // never reached PLAY within 60s -- treat as abandoned
                        Err(_elapsed) => Ok(None),
                    }
                }
            } => r,
        };
        let read_result = match read_result {
            Ok(Some(n)) => Ok(n),
            Ok(None) => break Ok(()),
            Err(e) => Err(e),
        };
        let n = match read_result {
            Ok(0) => break Ok(()), // client closed
            Ok(n) => n,
            Err(e) => break Err(e),
        };
        buf.extend_from_slice(&read_buf[..n]);
        if buf.len() > 64 * 1024 {
            break 'outer Ok(()); // no legitimate RTSP request is this large
        }
        loop {
            match Message::<Vec<u8>>::parse(&buf) {
                Ok((Message::Request(req), consumed)) => {
                    buf.drain(0..consumed);
                    let response = handle_request(&req, &cfg, &shared, &out_tx, &mut state).await;
                    let mut out = Vec::new();
                    if Message::from(response).write(&mut out).is_err() {
                        break 'outer Ok(());
                    }
                    if out_tx.send(Bytes::from(out)).await.is_err() {
                        break 'outer Ok(());
                    }
                }
                Ok((_, consumed)) => {
                    buf.drain(0..consumed.max(1)); // unexpected Response/Data from a client -- skip it
                }
                Err(rtsp_types::ParseError::Incomplete(_)) => break, // need more bytes
                Err(_) => break 'outer Ok(()), // malformed request -- close the connection
            }
        }
    };

    if let Some(task) = state.play_task.take() {
        task.abort();
    }
    if state.playing {
        shared.clients.fetch_sub(1, Ordering::SeqCst);
        publish_client_count(&shared);
    }
    if state.counted_interest {
        shared.interest.fetch_sub(1, Ordering::SeqCst);
    }
    drop(out_tx);
    let _ = writer.await;
    result
}

async fn handle_request(
    req: &Request<Vec<u8>>,
    cfg: &Arc<RwLock<ConfigFile>>,
    shared: &Arc<Shared>,
    out_tx: &mpsc::Sender<Bytes>,
    state: &mut ConnState,
) -> rtsp_types::Response<Vec<u8>> {
    let cseq = req
        .header(&headers::CSEQ)
        .map(|v| v.as_str().to_string())
        .unwrap_or_default();
    let base = |status: RtspStatusCode| {
        rtsp_types::Response::builder(Version::V1_0, status).header(headers::CSEQ, cseq.clone())
    };

    match req.method() {
        Method::Options => base(RtspStatusCode::Ok)
            .reason_phrase("OK")
            .header(
                headers::PUBLIC,
                "OPTIONS, DESCRIBE, SETUP, PLAY, TEARDOWN, GET_PARAMETER",
            )
            .build(Vec::new()),

        Method::Describe => {
            if !state.counted_interest {
                state.counted_interest = true;
                shared.interest.fetch_add(1, Ordering::SeqCst);
                shared.want_encoder.notify_one();
            }

            let (auth_enabled, username, ha1) = {
                let c = cfg.read().await;
                (
                    c.settings.rtsp.auth_enabled,
                    c.rtsp_username.clone(),
                    c.rtsp_password_ha1.clone(),
                )
            };
            if auth_enabled && !state.authenticated {
                let ok = req
                    .header(&headers::AUTHORIZATION)
                    .and_then(|v| digest::parse_authorization(v.as_str()))
                    .zip(state.nonce.clone())
                    .is_some_and(|(params, expected_nonce)| {
                        params.nonce == expected_nonce
                            && digest::verify(&params, &username, &ha1, &expected_nonce, "DESCRIBE")
                    });
                if ok {
                    state.authenticated = true;
                } else {
                    let fresh = new_random_token();
                    state.nonce = Some(fresh.clone());
                    return base(RtspStatusCode::Unauthorized)
                        .header(
                            headers::WWW_AUTHENTICATE,
                            digest::challenge(digest::REALM, &fresh),
                        )
                        .build(Vec::new());
                }
            }

            // A cold encoder start is a fresh ffmpeg fork+exec plus encoding
            // the first real captured frame -- measured ~5.7s on Pi hardware
            // at 1280x960, so 1s was too tight and made a bare `ffprobe`
            // against a freshly-enabled or just-reconnected stream fail.
            let mut waited = Duration::ZERO;
            let (sps, pps) = loop {
                let ready = {
                    let s = shared.stream.lock().unwrap();
                    s.sps.clone().zip(s.pps.clone())
                };
                if let Some(pair) = ready {
                    break pair;
                }
                if waited >= Duration::from_secs(8) {
                    return base(RtspStatusCode::ServiceUnavailable).build(Vec::new());
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
                waited += Duration::from_millis(50);
            };

            // Content-Base must be the full request URI (scheme+host+port+path),
            // not just the path -- a=control:streamid=0 in the SDP resolves
            // against it per RFC 3986, and a bare "rtsp://<path>/" would send
            // a real client's SETUP at a bogus host named after the path.
            let content_base = req
                .request_uri()
                .map(|u| {
                    let s = u.to_string();
                    if s.ends_with('/') {
                        s
                    } else {
                        format!("{s}/")
                    }
                })
                .unwrap_or_default();
            let body = sdp::build(&sps, &pps, "streamid=0");
            base(RtspStatusCode::Ok)
                .reason_phrase("OK")
                .header(headers::CONTENT_TYPE, "application/sdp")
                .header(headers::CONTENT_BASE, content_base)
                .build(body.into_bytes())
        }

        Method::Setup => {
            let auth_enabled = cfg.read().await.settings.rtsp.auth_enabled;
            if auth_enabled && !state.authenticated {
                return base(RtspStatusCode::Unauthorized).build(Vec::new());
            }
            let transport = req
                .header(&headers::TRANSPORT)
                .map(|v| v.as_str().to_string())
                .unwrap_or_default();
            if !transport.contains("TCP") {
                return base(RtspStatusCode::UnsupportedTransport).build(Vec::new());
            }
            let session_id = new_random_token();
            state.session = Some(session_id.clone());
            base(RtspStatusCode::Ok)
                .reason_phrase("OK")
                .header(headers::TRANSPORT, "RTP/AVP/TCP;unicast;interleaved=0-1")
                .header(headers::SESSION, session_id)
                .build(Vec::new())
        }

        Method::Play => {
            if state.session.is_none() {
                return base(RtspStatusCode::MethodNotValidInThisState).build(Vec::new());
            }
            // SETUP has no precondition on a prior DESCRIBE, so with auth
            // disabled a connection can reach PLAY without ever having raised
            // `interest` -- it would then count against the client cap while
            // contributing nothing to the thing that keeps the encoder alive
            // (no encoder at all for a lone such client, or one stopped out
            // from under it when whichever other connection's DESCRIBE started
            // it goes idle). `counted_interest` makes this the same idempotent
            // per-connection bump DESCRIBE does, safe to run from both.
            if !state.counted_interest {
                state.counted_interest = true;
                shared.interest.fetch_add(1, Ordering::SeqCst);
                shared.want_encoder.notify_one();
            }
            // A repeated PLAY on a session that is already playing must not
            // take a *second* slot out of the 4-client cap: the teardown and
            // disconnect paths only ever hand one back, so the counter would
            // leak upward permanently and eventually refuse every client.
            if !state.playing {
                if shared.clients.load(Ordering::SeqCst) >= MAX_CLIENTS {
                    return base(RtspStatusCode::NotEnoughBandwidth).build(Vec::new());
                }
                shared.clients.fetch_add(1, Ordering::SeqCst);
                publish_client_count(shared);
                state.playing = true;
            }
            // Likewise, never leave an older forwarding task alive alongside
            // the new one -- it would duplicate every RTP packet on the wire.
            if let Some(task) = state.play_task.take() {
                task.abort();
            }
            let mut rx = shared.tx.subscribe();
            let out_tx = out_tx.clone();
            state.play_task = Some(tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(frame) => {
                            if out_tx.send(frame).await.is_err() {
                                break;
                            }
                        }
                        // A lagging subscriber only drops old broadcast values for
                        // itself -- it never blocks the encoder or other sessions
                        // (the encoder's send() to the broadcast channel doesn't
                        // wait on slow receivers), so skipping ahead and resuming
                        // from the latest frame is enough; no need to also tear
                        // down the RTSP session over it.
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }));
            base(RtspStatusCode::Ok)
                .reason_phrase("OK")
                .header(headers::SESSION, state.session.clone().unwrap_or_default())
                .build(Vec::new())
        }

        Method::Teardown => {
            if let Some(task) = state.play_task.take() {
                task.abort();
            }
            if state.playing {
                shared.clients.fetch_sub(1, Ordering::SeqCst);
                publish_client_count(shared);
                state.playing = false;
            }
            base(RtspStatusCode::Ok)
                .reason_phrase("OK")
                .build(Vec::new())
        }

        Method::GetParameter => base(RtspStatusCode::Ok)
            .reason_phrase("OK")
            .build(Vec::new()),

        _ => base(RtspStatusCode::MethodNotAllowed)
            .reason_phrase("Method Not Allowed")
            .build(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpStream as ClientStream;

    fn test_cfg() -> Arc<RwLock<ConfigFile>> {
        let dir = tempfile::TempDir::new().unwrap();
        let store = crate::settings::SettingsStore::new(dir.path());
        let mut cfg = store
            .load_or_create(&crate::auth::hash_password("unused").unwrap())
            .unwrap();
        cfg.settings.rtsp.enabled = true;
        // Port 0 = OS-assigned, which avoids test port collisions.
        cfg.settings.rtsp.port = 0;
        // `RtspSettings::auth_enabled` defaults to *true*; the test that
        // exercises the digest flow turns it back on explicitly.
        cfg.settings.rtsp.auth_enabled = false;
        std::mem::forget(dir); // keep the tempdir alive for the test's duration
        Arc::new(RwLock::new(cfg))
    }

    async fn send_and_read(stream: &mut ClientStream, req: &str) -> String {
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap();
        String::from_utf8_lossy(&buf[..n]).to_string()
    }

    #[tokio::test]
    async fn options_and_unauthenticated_describe() {
        let cfg = test_cfg();
        let port = cfg.read().await.settings.rtsp.port;
        let shared = Arc::new(Shared {
            tx: broadcast::channel(16).0,
            stream: std::sync::Mutex::new(StreamState {
                sps: None,
                pps: None,
            }),
            counters: std::sync::Mutex::new(Counters {
                seq: 0,
                timestamp: 0,
            }),
            interest: AtomicUsize::new(0),
            clients: AtomicUsize::new(0),
            want_encoder: Notify::new(),
            status_tx: watch::channel(RtspStatus::default()).0,
            // These tests drive `handle_connection` directly, with no
            // `listener_task` to publish the real setting.
            enabled: watch::channel(true).0,
        });
        let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
        let bound_port = listener.local_addr().unwrap().port();
        {
            let mut c = cfg.write().await;
            c.settings.rtsp.port = bound_port;
        }
        let accept_cfg = cfg.clone();
        let accept_shared = shared.clone();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let _ = handle_connection(stream, accept_cfg, accept_shared).await;
        });

        let mut client = ClientStream::connect(("127.0.0.1", bound_port))
            .await
            .unwrap();
        let options_resp = send_and_read(
            &mut client,
            "OPTIONS rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 1\r\n\r\n",
        )
        .await;
        assert!(options_resp.starts_with("RTSP/1.0 200 OK\r\n"));
        assert!(options_resp.contains("CSeq: 1\r\n"));
        assert!(options_resp.contains("DESCRIBE"));

        // No SPS/PPS latched yet (no encoder running in this test) and
        // `test_cfg` turned auth off, so this exercises the "not ready yet"
        // 503, not the 401 path.
        let describe_resp = send_and_read(
            &mut client,
            "DESCRIBE rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 2\r\n\r\n",
        )
        .await;
        assert!(describe_resp.starts_with("RTSP/1.0 503"));
        assert_eq!(shared.interest.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn describe_challenges_and_accepts_correct_digest() {
        let cfg = test_cfg();
        {
            let mut c = cfg.write().await;
            c.settings.rtsp.auth_enabled = true;
        }
        let (username, ha1) = {
            let c = cfg.read().await;
            (c.rtsp_username.clone(), c.rtsp_password_ha1.clone())
        };
        let shared = Arc::new(Shared {
            tx: broadcast::channel(16).0,
            stream: std::sync::Mutex::new(StreamState {
                sps: Some(vec![0x67, 0x42, 0x00, 0x1e]),
                pps: Some(vec![0x68, 0xCE, 0x3C, 0x80]),
            }),
            counters: std::sync::Mutex::new(Counters {
                seq: 0,
                timestamp: 0,
            }),
            interest: AtomicUsize::new(0),
            clients: AtomicUsize::new(0),
            want_encoder: Notify::new(),
            status_tx: watch::channel(RtspStatus::default()).0,
            // These tests drive `handle_connection` directly, with no
            // `listener_task` to publish the real setting.
            enabled: watch::channel(true).0,
        });
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let bound_port = listener.local_addr().unwrap().port();
        let accept_cfg = cfg.clone();
        let accept_shared = shared.clone();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let _ = handle_connection(stream, accept_cfg, accept_shared).await;
        });

        let mut client = ClientStream::connect(("127.0.0.1", bound_port))
            .await
            .unwrap();
        let unauth = send_and_read(
            &mut client,
            "DESCRIBE rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 1\r\n\r\n",
        )
        .await;
        assert!(unauth.starts_with("RTSP/1.0 401"));
        let nonce_line = unauth
            .lines()
            .find(|l| l.starts_with("WWW-Authenticate:"))
            .unwrap();
        let nonce = nonce_line
            .split("nonce=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_string();

        let response = digest::expected_response(
            &ha1,
            &nonce,
            "00000001",
            "abc",
            "auth",
            "DESCRIBE",
            "rtsp://127.0.0.1/allsky",
        );
        let auth_header = format!(
            "Digest username=\"{username}\", realm=\"rskycam\", nonce=\"{nonce}\", uri=\"rtsp://127.0.0.1/allsky\", response=\"{response}\", nc=00000001, cnonce=\"abc\", qop=auth"
        );
        let authed = send_and_read(
            &mut client,
            &format!(
                "DESCRIBE rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 2\r\nAuthorization: {auth_header}\r\n\r\n"
            ),
        )
        .await;
        assert!(authed.starts_with("RTSP/1.0 200 OK\r\n"));
        assert!(authed.contains("Content-Type: application/sdp\r\n"));
        assert!(authed.contains("m=video 0 RTP/AVP 96"));
    }

    /// The header live555 (VLC 3.x's RTSP stack) actually sends: five fields,
    /// no `nc`/`cnonce`/`qop`, response over `MD5(HA1:nonce:HA2)`. The server
    /// advertises `qop="auth"` but the client is free to ignore it, so the
    /// whole DESCRIBE path -- parse *and* verify -- has to accept this form or
    /// such a client is 401'd forever.
    #[tokio::test]
    async fn describe_accepts_the_rfc2069_header_vlc_sends() {
        let cfg = test_cfg();
        {
            let mut c = cfg.write().await;
            c.settings.rtsp.auth_enabled = true;
        }
        let (username, ha1) = {
            let c = cfg.read().await;
            (c.rtsp_username.clone(), c.rtsp_password_ha1.clone())
        };
        let shared = Arc::new(Shared {
            tx: broadcast::channel(16).0,
            stream: std::sync::Mutex::new(StreamState {
                sps: Some(vec![0x67, 0x42, 0x00, 0x1e]),
                pps: Some(vec![0x68, 0xCE, 0x3C, 0x80]),
            }),
            counters: std::sync::Mutex::new(Counters {
                seq: 0,
                timestamp: 0,
            }),
            interest: AtomicUsize::new(0),
            clients: AtomicUsize::new(0),
            want_encoder: Notify::new(),
            status_tx: watch::channel(RtspStatus::default()).0,
            enabled: watch::channel(true).0,
        });
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let bound_port = listener.local_addr().unwrap().port();
        let accept_cfg = cfg.clone();
        let accept_shared = shared.clone();
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let c = accept_cfg.clone();
                let s = accept_shared.clone();
                tokio::spawn(async move {
                    let _ = handle_connection(stream, c, s).await;
                });
            }
        });

        let mut client = ClientStream::connect(("127.0.0.1", bound_port))
            .await
            .unwrap();
        let unauth = send_and_read(
            &mut client,
            "DESCRIBE rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 1\r\n\r\n",
        )
        .await;
        assert!(unauth.starts_with("RTSP/1.0 401"), "{unauth}");
        assert!(
            unauth.contains("qop=\"auth\""),
            "challenge still advertises qop"
        );
        let nonce = unauth
            .lines()
            .find(|l| l.starts_with("WWW-Authenticate:"))
            .unwrap()
            .split("nonce=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_string();

        let response =
            digest::expected_response_rfc2069(&ha1, &nonce, "DESCRIBE", "rtsp://127.0.0.1/allsky");
        let auth_header = format!(
            "Digest username=\"{username}\", realm=\"rskycam\", nonce=\"{nonce}\", uri=\"rtsp://127.0.0.1/allsky\", response=\"{response}\""
        );
        let authed = send_and_read(
            &mut client,
            &format!(
                "DESCRIBE rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 2\r\nAuthorization: {auth_header}\r\n\r\n"
            ),
        )
        .await;
        assert!(authed.starts_with("RTSP/1.0 200 OK\r\n"), "{authed}");

        // A wrong password in that same form is still refused -- accepting the
        // shape must not mean accepting the credential.
        let mut liar = ClientStream::connect(("127.0.0.1", bound_port))
            .await
            .unwrap();
        let challenge = send_and_read(
            &mut liar,
            "DESCRIBE rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 1\r\n\r\n",
        )
        .await;
        let liar_nonce = challenge
            .lines()
            .find(|l| l.starts_with("WWW-Authenticate:"))
            .unwrap()
            .split("nonce=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_string();
        let bad = digest::expected_response_rfc2069(
            &digest::ha1(&username, digest::REALM, "not-the-password"),
            &liar_nonce,
            "DESCRIBE",
            "rtsp://127.0.0.1/allsky",
        );
        let refused = send_and_read(
            &mut liar,
            &format!(
                "DESCRIBE rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 2\r\nAuthorization: Digest username=\"{username}\", realm=\"rskycam\", nonce=\"{liar_nonce}\", uri=\"rtsp://127.0.0.1/allsky\", response=\"{bad}\"\r\n\r\n"
            ),
        )
        .await;
        assert!(refused.starts_with("RTSP/1.0 401"), "{refused}");
    }

    fn fixture_ffmpeg_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-ffmpeg-h264")
    }

    /// Blocks until `spawn_rtsp`'s listener has bound, and reports the port
    /// it settled on (these tests configure port 0, so the OS picks one).
    async fn wait_for_listening_port(status: &mut watch::Receiver<RtspStatus>) -> u16 {
        loop {
            status.changed().await.unwrap();
            let s = status.borrow().clone();
            if s.listening {
                return s.port;
            }
        }
    }

    /// Parses zero or more complete `$ channel len[u16 BE] payload`
    /// interleaved frames starting at the first `$` found in `data`
    /// (skipping the leading plain-text RTSP response that precedes them
    /// on the same socket). Returns `None` only when no `$` has arrived at
    /// all yet; once the first one is seen it returns however many
    /// complete frames are present so far -- an empty `Vec` if even that
    /// first frame is still incomplete.
    fn parse_interleaved_frames(data: &[u8]) -> Option<Vec<(u8, Vec<u8>)>> {
        let start = data.iter().position(|&b| b == b'$')?;
        let mut frames = Vec::new();
        let mut i = start;
        while i + 4 <= data.len() {
            if data[i] != b'$' {
                break;
            }
            let channel = data[i + 1];
            let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
            if i + 4 + len > data.len() {
                break; // this frame hasn't fully arrived yet
            }
            frames.push((channel, data[i + 4..i + 4 + len].to_vec()));
            i += 4 + len;
        }
        Some(frames)
    }

    #[tokio::test]
    async fn full_session_lifecycle_over_a_real_socket() {
        let cfg = test_cfg();
        {
            let mut c = cfg.write().await;
            c.settings.rtsp.auth_enabled = false; // isolate lifecycle from the auth path (covered separately)
        }
        let (_latest_tx, latest_rx) = watch::channel(None);
        let handle = spawn_rtsp(cfg.clone(), latest_rx, fixture_ffmpeg_path());
        // Binding our own listener isn't possible here since `spawn_rtsp`
        // owns the bind -- instead read back the port it settled on.
        let mut status = handle.status;
        let port = wait_for_listening_port(&mut status).await;

        let mut client = ClientStream::connect(("127.0.0.1", port)).await.unwrap();
        let describe = send_and_read(
            &mut client,
            "DESCRIBE rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 1\r\n\r\n",
        )
        .await;
        // The fixture encoder needs a beat to spawn and emit its first
        // access unit; DESCRIBE's own 8s poll loop covers that, but a
        // freshly-bound listener plus process spawn can occasionally need
        // a second attempt on a loaded machine -- retry once before failing.
        let describe = if describe.starts_with("RTSP/1.0 503") {
            send_and_read(
                &mut client,
                "DESCRIBE rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 2\r\n\r\n",
            )
            .await
        } else {
            describe
        };
        assert!(describe.starts_with("RTSP/1.0 200 OK\r\n"), "{describe}");

        let setup = send_and_read(
            &mut client,
            "SETUP rtsp://127.0.0.1/allsky/streamid=0 RTSP/1.0\r\nCSeq: 3\r\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n\r\n",
        )
        .await;
        assert!(setup.starts_with("RTSP/1.0 200 OK\r\n"), "{setup}");
        assert!(setup.contains("interleaved=0-1"));

        client
            .write_all(b"PLAY rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 4\r\n\r\n")
            .await
            .unwrap();
        // PLAY's own response and the interleaved RTP frames that follow it
        // both arrive on the same socket -- collect at least two full
        // interleaved frames (a single access unit already packetizes to
        // 3: SPS, PPS, IDR) so payload type and sequence progression can be
        // checked directly, the way a real client would see them.
        let mut collected = Vec::new();
        let frames = tokio::time::timeout(Duration::from_secs(10), async {
            let mut buf = [0u8; 4096];
            loop {
                let n = client.read(&mut buf).await.unwrap();
                assert!(n > 0, "server closed the connection during PLAY");
                collected.extend_from_slice(&buf[..n]);
                if let Some(frames) = parse_interleaved_frames(&collected) {
                    if frames.len() >= 2 {
                        return frames;
                    }
                }
            }
        })
        .await
        .expect("timed out waiting for two interleaved RTP frames");
        assert!(collected.starts_with(b"RTSP/1.0 200 OK\r\n"));
        for (channel, rtp) in &frames {
            assert_eq!(*channel, 0);
            assert_eq!(rtp[1] & 0x7f, 96); // payload type 96, marker bit masked off
        }
        let seq0 = u16::from_be_bytes([frames[0].1[2], frames[0].1[3]]);
        let seq1 = u16::from_be_bytes([frames[1].1[2], frames[1].1[3]]);
        assert_eq!(seq1, seq0.wrapping_add(1));
        assert_eq!(status.borrow().clients, 1);

        client
            .write_all(b"TEARDOWN rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 5\r\n\r\n")
            .await
            .unwrap();
        // RTP frames already queued behind the writer keep arriving until
        // the TEARDOWN response catches up with them, so scan the stream for
        // the status line rather than assuming it is the very next read.
        let needle: &[u8] = b"RTSP/1.0 200 OK\r\n";
        let mut tail = Vec::new();
        let saw_ok = tokio::time::timeout(Duration::from_secs(10), async {
            let mut buf = [0u8; 4096];
            loop {
                let n = client.read(&mut buf).await.unwrap();
                if n == 0 {
                    return false; // closed without ever answering
                }
                tail.extend_from_slice(&buf[..n]);
                if tail.windows(needle.len()).any(|w| w == needle) {
                    return true;
                }
            }
        })
        .await
        .expect("timed out waiting for the TEARDOWN response");
        assert!(saw_ok, "connection closed before the TEARDOWN response");
        // TEARDOWN gives the client slot back exactly once.
        assert_eq!(status.borrow().clients, 0);
    }

    #[tokio::test]
    async fn udp_setup_is_rejected_and_a_fifth_client_is_capped() {
        let cfg = test_cfg();
        {
            let mut c = cfg.write().await;
            c.settings.rtsp.auth_enabled = false;
        }
        let (_latest_tx, latest_rx) = watch::channel(None);
        let handle = spawn_rtsp(cfg, latest_rx, fixture_ffmpeg_path());
        let mut status = handle.status;
        let port = wait_for_listening_port(&mut status).await;

        let mut udp_client = ClientStream::connect(("127.0.0.1", port)).await.unwrap();
        let setup_udp = send_and_read(
            &mut udp_client,
            "SETUP rtsp://127.0.0.1/allsky/streamid=0 RTSP/1.0\r\nCSeq: 1\r\nTransport: RTP/AVP;unicast;client_port=5000-5001\r\n\r\n",
        )
        .await;
        assert!(setup_udp.starts_with("RTSP/1.0 461"), "{setup_udp}");

        // Wait for the encoder to be ready, then fill all 4 client slots and
        // confirm a 5th is refused.
        for _ in 0..2 {
            let _ = send_and_read(
                &mut udp_client,
                "DESCRIBE rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 2\r\n\r\n",
            )
            .await;
        }
        let mut clients = Vec::new();
        for i in 0..4 {
            let mut c = ClientStream::connect(("127.0.0.1", port)).await.unwrap();
            let _ = send_and_read(
                &mut c,
                &format!("DESCRIBE rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: {i}\r\n\r\n"),
            )
            .await;
            let _ = send_and_read(
                &mut c,
                &format!(
                    "SETUP rtsp://127.0.0.1/allsky/streamid=0 RTSP/1.0\r\nCSeq: {i}\r\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n\r\n"
                ),
            )
            .await;
            let play = send_and_read(
                &mut c,
                &format!("PLAY rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: {i}\r\n\r\n"),
            )
            .await;
            assert!(
                play.starts_with("RTSP/1.0 200 OK\r\n"),
                "client {i}: {play}"
            );
            clients.push(c);
        }
        let mut fifth = ClientStream::connect(("127.0.0.1", port)).await.unwrap();
        let _ = send_and_read(
            &mut fifth,
            "DESCRIBE rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 9\r\n\r\n",
        )
        .await;
        let _ = send_and_read(
            &mut fifth,
            "SETUP rtsp://127.0.0.1/allsky/streamid=0 RTSP/1.0\r\nCSeq: 9\r\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n\r\n",
        )
        .await;
        let fifth_play = send_and_read(
            &mut fifth,
            "PLAY rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 9\r\n\r\n",
        )
        .await;
        assert!(fifth_play.starts_with("RTSP/1.0 453"), "{fifth_play}");
        assert_eq!(status.borrow().clients, MAX_CLIENTS);
    }

    #[tokio::test]
    async fn wrong_password_is_rejected_with_401() {
        let cfg = test_cfg();
        {
            let mut c = cfg.write().await;
            c.settings.rtsp.auth_enabled = true;
        }
        let (_latest_tx, latest_rx) = watch::channel(None);
        let handle = spawn_rtsp(cfg, latest_rx, fixture_ffmpeg_path());
        let mut status = handle.status;
        let port = wait_for_listening_port(&mut status).await;
        let mut client = ClientStream::connect(("127.0.0.1", port)).await.unwrap();
        let unauth = send_and_read(
            &mut client,
            "DESCRIBE rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 1\r\n\r\n",
        )
        .await;
        assert!(unauth.starts_with("RTSP/1.0 401"), "{unauth}");
        let nonce = unauth
            .lines()
            .find(|l| l.starts_with("WWW-Authenticate:"))
            .unwrap()
            .split("nonce=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_string();
        let bad_response = digest::expected_response(
            "0000000000000000000000000000000", // wrong HA1
            &nonce,
            "00000001",
            "abc",
            "auth",
            "DESCRIBE",
            "rtsp://127.0.0.1/allsky",
        );
        let auth_header = format!(
            "Digest username=\"admin\", realm=\"rskycam\", nonce=\"{nonce}\", uri=\"rtsp://127.0.0.1/allsky\", response=\"{bad_response}\", nc=00000001, cnonce=\"abc\", qop=auth"
        );
        let still_unauth = send_and_read(
            &mut client,
            &format!(
                "DESCRIBE rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 2\r\nAuthorization: {auth_header}\r\n\r\n"
            ),
        )
        .await;
        assert!(still_unauth.starts_with("RTSP/1.0 401"), "{still_unauth}");
        // And a client can't sidestep the challenge by jumping straight to
        // SETUP: without an authenticated connection it is refused too.
        let setup = send_and_read(
            &mut client,
            "SETUP rtsp://127.0.0.1/allsky/streamid=0 RTSP/1.0\r\nCSeq: 3\r\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n\r\n",
        )
        .await;
        assert!(setup.starts_with("RTSP/1.0 401"), "{setup}");
    }

    #[tokio::test]
    async fn encoder_crash_is_recovered_with_backoff() {
        let cfg = test_cfg();
        {
            let mut c = cfg.write().await;
            c.settings.rtsp.auth_enabled = false;
        }
        let (_latest_tx, latest_rx) = watch::channel(None);
        // An encoder binary that can't be executed at all is the simplest
        // deterministic "crash": `nice` reports 127 and `run_encoder` turns
        // that into an `Err`, exactly like a mid-run ffmpeg failure would.
        // (Deliberately *not* the fixture's `fake-ffmpeg-h264-fail` marker:
        // that one is keyed off the process-wide cwd, and mutating the cwd
        // here would make every other test's encoder fixture fail too.)
        let handle = spawn_rtsp(
            cfg,
            latest_rx,
            PathBuf::from("/nonexistent/rskycam-no-such-ffmpeg"),
        );
        let mut status = handle.status;
        let port = wait_for_listening_port(&mut status).await;
        // A DESCRIBE is what actually raises `interest` and wakes the encoder
        // supervisor -- without one the encoder never even tries to spawn.
        // `interest` is bumped synchronously as soon as the request is parsed,
        // well before the handler waits on SPS/PPS, so this only needs to
        // land the request -- reading the response would tie this test's
        // timing to that wait (now up to 8s) for no reason, since the
        // response itself is never used.
        let mut client = ClientStream::connect(("127.0.0.1", port)).await.unwrap();
        client
            .write_all(b"DESCRIBE rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 1\r\n\r\n")
            .await
            .unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let mut saw_error = false;
        while tokio::time::Instant::now() < deadline {
            if status.borrow().last_error.is_some() {
                saw_error = true;
                break;
            }
            let _ = tokio::time::timeout(Duration::from_millis(200), status.changed()).await;
        }
        assert!(
            saw_error,
            "expected a recorded encoder error after a simulated crash"
        );
        let recorded = status.borrow().last_error.clone().unwrap();
        assert!(recorded.starts_with("encoder: "), "{recorded}");

        // The supervisor keeps retrying instead of wedging: every retry cycle
        // republishes status (encoding on, encoding off, error again), so a
        // continuing trickle of updates is what "recovered with backoff"
        // looks like from the outside. With a 1s then 2s backoff there are
        // two more cycles inside this window, six updates in all.
        let mut updates = 0;
        let watch_until = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < watch_until {
            if tokio::time::timeout(Duration::from_millis(250), status.changed())
                .await
                .is_ok()
            {
                updates += 1;
            }
        }
        assert!(
            updates >= 3,
            "supervisor stopped retrying after the first failure ({updates} status updates)"
        );
    }

    /// The point of the whole `interest` mechanism: once the last connection
    /// that asked for the stream is gone, the supervisor must park instead of
    /// respawning ffmpeg forever on an idle Pi.
    ///
    /// Note this exercises the "encoder exited while nothing wants it" park at
    /// the top of the supervisor's post-run block, *not* the 10s
    /// `watch_for_idle_and_stop` debounce: the fixture encoder self-terminates
    /// after ~1s, so the park is always reached first. The debounce only
    /// matters for a real, long-running ffmpeg.
    #[tokio::test]
    async fn encoder_stops_once_the_last_interested_client_disconnects() {
        let cfg = test_cfg();
        {
            let mut c = cfg.write().await;
            c.settings.rtsp.auth_enabled = false;
        }
        let (_latest_tx, latest_rx) = watch::channel(None);
        let handle = spawn_rtsp(cfg, latest_rx, fixture_ffmpeg_path());
        let mut status = handle.status;
        let port = wait_for_listening_port(&mut status).await;

        let mut client = ClientStream::connect(("127.0.0.1", port)).await.unwrap();
        let _ = send_and_read(
            &mut client,
            "DESCRIBE rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 1\r\n\r\n",
        )
        .await;
        drop(client); // interest drops back to 0

        // "Settled" = a stretch with no status updates at all that ends with
        // `encoding` false; a supervisor still cycling the fixture would keep
        // publishing every second and never produce such a stretch.
        let settled = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let quiet = tokio::time::timeout(Duration::from_secs(3), status.changed()).await;
                if quiet.is_err() {
                    return !status.borrow().encoding;
                }
            }
        })
        .await
        .expect("supervisor never went quiet after the last client left");
        assert!(settled, "encoder was left running with nothing interested");
    }

    /// With auth disabled, SETUP has no precondition on a prior DESCRIBE, so a
    /// client can reach PLAY without ever having raised `interest`. PLAY must
    /// raise it itself, otherwise the encoder never starts for such a client
    /// (or gets stopped out from under it) and no RTP is ever delivered.
    #[tokio::test]
    async fn play_without_a_prior_describe_still_starts_the_encoder() {
        let cfg = test_cfg();
        {
            let mut c = cfg.write().await;
            c.settings.rtsp.auth_enabled = false;
        }
        let (_latest_tx, latest_rx) = watch::channel(None);
        let handle = spawn_rtsp(cfg, latest_rx, fixture_ffmpeg_path());
        let mut status = handle.status;
        let port = wait_for_listening_port(&mut status).await;

        let mut client = ClientStream::connect(("127.0.0.1", port)).await.unwrap();
        let setup = send_and_read(
            &mut client,
            "SETUP rtsp://127.0.0.1/allsky/streamid=0 RTSP/1.0\r\nCSeq: 1\r\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n\r\n",
        )
        .await;
        assert!(setup.starts_with("RTSP/1.0 200 OK\r\n"), "{setup}");

        client
            .write_all(b"PLAY rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 2\r\n\r\n")
            .await
            .unwrap();
        let mut collected = Vec::new();
        let frames = tokio::time::timeout(Duration::from_secs(10), async {
            let mut buf = [0u8; 4096];
            loop {
                let n = client.read(&mut buf).await.unwrap();
                assert!(n > 0, "server closed the connection during PLAY");
                collected.extend_from_slice(&buf[..n]);
                if let Some(frames) = parse_interleaved_frames(&collected) {
                    if !frames.is_empty() {
                        return frames;
                    }
                }
            }
        })
        .await
        .expect("PLAY alone never started the encoder -- no RTP arrived");
        assert!(collected.starts_with(b"RTSP/1.0 200 OK\r\n"));
        assert_eq!(frames[0].0, 0); // interleaved channel 0
        assert_eq!(status.borrow().clients, 1);
    }

    /// Per the spec's error-handling table, `enabled` toggled off with clients
    /// connected must TEARDOWN all, stop the encoder and drop the listener --
    /// not merely stop accepting new connections while the existing ones keep
    /// streaming RTP and pinning `interest` above zero forever.
    #[tokio::test]
    async fn disabling_rtsp_disconnects_a_playing_client() {
        let cfg = test_cfg();
        {
            let mut c = cfg.write().await;
            c.settings.rtsp.auth_enabled = false;
        }
        let (_latest_tx, latest_rx) = watch::channel(None);
        let handle = spawn_rtsp(cfg.clone(), latest_rx, fixture_ffmpeg_path());
        let mut status = handle.status;
        let port = wait_for_listening_port(&mut status).await;

        let mut client = ClientStream::connect(("127.0.0.1", port)).await.unwrap();
        let _ = send_and_read(
            &mut client,
            "SETUP rtsp://127.0.0.1/allsky/streamid=0 RTSP/1.0\r\nCSeq: 1\r\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n\r\n",
        )
        .await;
        client
            .write_all(b"PLAY rtsp://127.0.0.1/allsky RTSP/1.0\r\nCSeq: 2\r\n\r\n")
            .await
            .unwrap();
        // Wait until it is genuinely playing (RTP arriving), so the teardown
        // below is exercised against a live session rather than a half-open one.
        let mut collected = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            let mut buf = [0u8; 4096];
            loop {
                let n = client.read(&mut buf).await.unwrap();
                assert!(n > 0, "server closed the connection during PLAY");
                collected.extend_from_slice(&buf[..n]);
                if parse_interleaved_frames(&collected).is_some_and(|f| !f.is_empty()) {
                    return;
                }
            }
        })
        .await
        .expect("client never reached a playing state");
        assert_eq!(status.borrow().clients, 1);

        {
            let mut c = cfg.write().await;
            c.settings.rtsp.enabled = false;
        }

        // The listener rechecks every 2s, so allow a few cycles.
        let closed = tokio::time::timeout(Duration::from_secs(15), async {
            let mut buf = [0u8; 4096];
            loop {
                match client.read(&mut buf).await {
                    Ok(0) | Err(_) => return, // EOF or reset -- the session was torn down
                    Ok(_) => {}               // RTP still queued behind the close; keep draining
                }
            }
        })
        .await;
        assert!(
            closed.is_ok(),
            "a playing client kept streaming after rtsp.enabled went false"
        );

        // And the counters came back, so a re-enable starts from a clean slate
        // rather than a permanently-pinned client slot.
        let settled = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let s = status.borrow().clone();
                if !s.listening && !s.encoding && s.clients == 0 {
                    return;
                }
                let _ = tokio::time::timeout(Duration::from_millis(200), status.changed()).await;
            }
        })
        .await;
        assert!(
            settled.is_ok(),
            "after disabling: {:?}",
            status.borrow().clone()
        );

        // Re-enabling must bind again and serve a fresh client -- the disable
        // path must not have left the supervisor or listener wedged.
        {
            let mut c = cfg.write().await;
            c.settings.rtsp.enabled = true;
            c.settings.rtsp.port = port; // reuse the port the OS handed out
        }
        let reopened = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if status.borrow().listening {
                    return;
                }
                let _ = tokio::time::timeout(Duration::from_millis(200), status.changed()).await;
            }
        })
        .await;
        assert!(
            reopened.is_ok(),
            "listener never came back after re-enabling"
        );
        let mut again = ClientStream::connect(("127.0.0.1", port)).await.unwrap();
        let setup = send_and_read(
            &mut again,
            "SETUP rtsp://127.0.0.1/allsky/streamid=0 RTSP/1.0\r\nCSeq: 1\r\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n\r\n",
        )
        .await;
        assert!(
            setup.starts_with("RTSP/1.0 200 OK\r\n"),
            "a client reconnecting right after a re-enable was refused: {setup}"
        );
    }
}
