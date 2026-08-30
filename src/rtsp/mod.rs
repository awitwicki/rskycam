// Nothing here is reachable from `main.rs` until Task 11 calls `spawn_rtsp`,
// and Tasks 9/10 fill in the encoder plumbing (`tx`, `counters`, `SSRC`,
// `MAX_CLIENTS`, `IDLE_STOP_AFTER`) this skeleton only declares.
#![allow(dead_code)]

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
}

fn publish_status(shared: &Shared, edit: impl FnOnce(&mut RtspStatus)) {
    shared.status_tx.send_modify(edit);
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
    });

    tokio::spawn(listener_task(cfg, shared));
    let _ = (latest, ffmpeg); // wired up by Task 9's encoder supervisor

    RtspHandle { status: status_rx }
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
        if !enabled {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
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
                    let cfg = cfg.clone();
                    let shared = shared.clone();
                    tokio::spawn(async move {
                        let _ = handle_connection(stream, cfg, shared).await;
                    });
                }
                _ = recheck.tick() => {
                    let c = cfg.read().await;
                    if !c.settings.rtsp.enabled || c.settings.rtsp.port != port {
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

    let result = 'outer: loop {
        // Before PLAY, a client that vanishes without a clean TCP close would
        // otherwise pin `interest` forever (nothing is written on this socket
        // between DESCRIBE and PLAY, so the peer's absence is never noticed)
        // and defeat the lazy encoder's idle-stop. Once playing, drop the
        // timeout: RTP delivery doesn't depend on the read side, and a client
        // may legitimately stay quiet for a long time between GET_PARAMETERs.
        let read_result = if state.playing {
            read_half.read(&mut read_buf).await
        } else {
            match tokio::time::timeout(Duration::from_secs(60), read_half.read(&mut read_buf)).await
            {
                Ok(r) => r,
                Err(_elapsed) => break Ok(()), // never reached PLAY within 60s -- treat as abandoned
            }
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
    let _ = out_tx; // Task 10's PLAY handler writes interleaved RTP through it
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

            let mut waited = Duration::ZERO;
            let (sps, pps) = loop {
                let ready = {
                    let s = shared.stream.lock().unwrap();
                    s.sps.clone().zip(s.pps.clone())
                };
                if let Some(pair) = ready {
                    break pair;
                }
                if waited >= Duration::from_secs(1) {
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

        _ => base(RtspStatusCode::MethodNotAllowed)
            .reason_phrase("Method Not Allowed")
            .build(Vec::new()), // SETUP/PLAY/TEARDOWN/GET_PARAMETER added in Task 10
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
}
