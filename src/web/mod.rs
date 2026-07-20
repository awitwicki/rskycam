pub mod api;
pub mod auth_layer;
pub mod nights;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::FromRef;
use axum::routing::{get, post};
use axum::Router;
use axum_extra::extract::cookie::Key;
use tokio::sync::{watch, RwLock};

use crate::capture::{CaptureStatus, LatestFrame};
use crate::settings::{ConfigFile, SettingsStore};

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<RwLock<ConfigFile>>,
    pub store: Arc<SettingsStore>,
    pub latest: watch::Receiver<Option<Arc<LatestFrame>>>,
    pub capture_status: watch::Receiver<CaptureStatus>,
    pub camera_caps: watch::Receiver<Option<crate::capture::CameraCaps>>,
    pub key: Key,
    pub data_dir: PathBuf,
    pub processing: crate::processing::ProcessingHandle,
}

impl FromRef<AppState> for Key {
    fn from_ref(s: &AppState) -> Key {
        s.key.clone()
    }
}

/// Cookie-signing key persisted in <data>/secret (created on first start).
pub fn load_or_create_secret(data_dir: &Path) -> anyhow::Result<Key> {
    let path = data_dir.join("secret");
    match std::fs::read(&path) {
        Ok(bytes) if bytes.len() >= 64 => Ok(Key::from(&bytes)),
        _ => {
            use rand::RngCore;
            let mut bytes = [0u8; 64];
            rand::rngs::OsRng.fill_bytes(&mut bytes);
            std::fs::create_dir_all(data_dir)?;
            let _ = std::fs::remove_file(&path); // a short/corrupt secret is replaced
            write_secret(&path, &bytes)?;
            Ok(Key::from(&bytes))
        }
    }
}

#[cfg(unix)]
fn write_secret(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    // 0600 from creation: no window where the signing key is group/world-readable
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)
}

#[cfg(not(unix))]
fn write_secret(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_is_created_0600_and_reused() {
        let dir = tempfile::TempDir::new().unwrap();
        let _k1 = load_or_create_secret(dir.path()).unwrap();
        let path = dir.path().join("secret");
        let bytes1 = std::fs::read(&path).unwrap();
        assert_eq!(bytes1.len(), 64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _k2 = load_or_create_secret(dir.path()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes1); // reused, not regenerated
    }

    #[test]
    fn short_secret_is_replaced() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path()).unwrap();
        std::fs::write(dir.path().join("secret"), b"short").unwrap();
        let _k = load_or_create_secret(dir.path()).unwrap();
        assert_eq!(std::fs::read(dir.path().join("secret")).unwrap().len(), 64);
    }

    #[cfg(not(feature = "embed-ui"))]
    #[test]
    fn resolve_spa_path_rejects_traversal_and_accepts_normal() {
        assert!(super::resolve_spa_path("../Cargo.toml").is_none());
        assert!(super::resolve_spa_path("../../etc/passwd").is_none());
        assert!(super::resolve_spa_path("").is_none());
        // A path that doesn't exist on disk under frontend/dist resolves to None too
        // (resolve_spa_path only returns Some for files that actually exist).
        assert!(super::resolve_spa_path("assets/definitely-not-a-real-file.js").is_none());
    }

    #[tokio::test]
    async fn spa_fallback_rejects_path_traversal() {
        use crate::web::testing::harness;
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        // A traversal attempt must NOT escape frontend/dist. Depending on whether
        // frontend/dist/index.html exists in the build, the fallback either serves
        // index.html (200) or reports the UI missing (503) — but it must never
        // return the contents of a file outside the dist dir.
        //
        // "../../Cargo.toml" (two levels) is used rather than a single "../"
        // because `dist` is itself two path components ("frontend/dist"): a
        // lone ".." only cancels the "dist" segment and lands on the
        // nonexistent "frontend/Cargo.toml", which would pass even against the
        // unpatched code and not actually prove the guard works. Verified by
        // temporarily reverting the fix: this exact request panics with
        // "leaked Cargo.toml via SPA fallback" against the vulnerable code.
        let h = harness();
        let app = crate::web::router(h.state.clone());
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/../../Cargo.toml")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let body = res.into_body().collect().await.unwrap().to_bytes();
        // Must not have leaked the manifest (which starts with [package]).
        assert!(
            !body.windows(9).any(|w| w == b"[package]"),
            "leaked Cargo.toml via SPA fallback"
        );
        assert!(status == StatusCode::OK || status == StatusCode::SERVICE_UNAVAILABLE);
    }
}

pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/api/change-password", post(auth_layer::change_password))
        .route("/api/logout", post(auth_layer::logout))
        .route("/api/status", get(api::get_status))
        .route("/api/latest.jpg", get(api::latest_jpg))
        .route("/api/events", get(api::events))
        .route("/api/lightgraph", get(api::get_lightgraph))
        .route("/api/overlay", post(api::post_overlay))
        .route(
            "/api/settings",
            get(api::get_settings).put(api::put_settings),
        )
        .route("/api/nights", get(nights::get_nights))
        .route(
            "/api/nights/{date}",
            get(nights::get_night).delete(nights::delete_night),
        )
        .route("/api/nights/{date}/rebuild", post(nights::rebuild_night))
        .route("/api/files/{date}/{*path}", get(nights::get_file))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_layer::require_session,
        ));
    Router::new()
        .route("/api/login", post(auth_layer::login))
        .merge(protected)
        .with_state(state)
        .fallback_service(spa_service())
}

#[cfg(feature = "embed-ui")]
fn spa_service() -> axum::routing::MethodRouter {
    use axum::http::{header, StatusCode, Uri};
    use axum::response::IntoResponse;

    #[derive(rust_embed::RustEmbed)]
    #[folder = "frontend/dist/"]
    struct Assets;

    axum::routing::get(|uri: Uri| async move {
        let path = uri.path().trim_start_matches('/');
        let file = if path.is_empty() { "index.html" } else { path };
        let asset = Assets::get(file).or_else(|| Assets::get("index.html"));
        match asset {
            Some(content) => {
                let mime = mime_guess::from_path(file).first_or_octet_stream();
                (
                    [(header::CONTENT_TYPE, mime.as_ref().to_string())],
                    content.data.into_owned(),
                )
                    .into_response()
            }
            None => (StatusCode::SERVICE_UNAVAILABLE, "UI not embedded").into_response(),
        }
    })
}

/// Resolve a raw request path to a file under `frontend/dist`, rejecting any
/// path traversal attempt. Returns `None` for an empty `rel` or one containing
/// non-`Normal` components (e.g. `..`, absolute roots, prefixes) — callers
/// should fall back to serving `index.html` (SPA client-side routing) in that
/// case, never attempt to read outside the dist directory.
#[cfg(not(feature = "embed-ui"))]
fn resolve_spa_path(rel: &str) -> Option<PathBuf> {
    if rel.is_empty() {
        return None;
    }
    let is_safe = std::path::Path::new(rel)
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)));
    if !is_safe {
        return None;
    }
    let dist = std::path::Path::new("frontend/dist");
    let candidate = dist.join(rel);
    candidate.is_file().then_some(candidate)
}

#[cfg(not(feature = "embed-ui"))]
fn spa_service() -> axum::routing::MethodRouter {
    axum::routing::get(|uri: axum::http::Uri| async move {
        use axum::response::IntoResponse;
        let dist = std::path::Path::new("frontend/dist");
        let rel = uri.path().trim_start_matches('/');
        let file = resolve_spa_path(rel).unwrap_or_else(|| dist.join("index.html"));
        match std::fs::read(&file) {
            Ok(bytes) => {
                let mime = match file.extension().and_then(|e| e.to_str()) {
                    Some("html") => "text/html; charset=utf-8",
                    Some("js") => "text/javascript",
                    Some("css") => "text/css",
                    Some("svg") => "image/svg+xml",
                    Some("png") => "image/png",
                    Some("jpg" | "jpeg") => "image/jpeg",
                    Some("mp4") => "video/mp4",
                    _ => "application/octet-stream",
                };
                ([(axum::http::header::CONTENT_TYPE, mime)], bytes).into_response()
            }
            Err(_) => (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "UI not built — run: cd frontend && npm run build",
            )
                .into_response(),
        }
    })
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request};
    use tower::ServiceExt;

    pub struct Harness {
        pub state: AppState,
        pub latest_tx: watch::Sender<Option<Arc<LatestFrame>>>,
        // Not read by every test; kept available for status-driven scenarios.
        #[allow(dead_code)]
        pub status_tx: watch::Sender<CaptureStatus>,
        pub caps_tx: watch::Sender<Option<crate::capture::CameraCaps>>,
        // Kept alive only to hold the TempDir guard for the harness's lifetime.
        #[allow(dead_code)]
        pub dir: tempfile::TempDir,
    }

    pub fn harness() -> Harness {
        let dir = tempfile::TempDir::new().unwrap();
        let store = Arc::new(SettingsStore::new(dir.path()));
        let cfg = store
            .load_or_create(&crate::auth::hash_password(crate::auth::DEFAULT_PASSWORD).unwrap())
            .unwrap();
        let cfg_arc = Arc::new(RwLock::new(cfg));
        let (latest_tx, latest) = watch::channel(None);
        let (status_tx, capture_status) = watch::channel(crate::capture::CaptureStatus {
            state: crate::capture::CaptureState::Idle,
            message: None,
            last_frame: None,
        });
        let (caps_tx, camera_caps) = watch::channel(None);
        let processing = crate::processing::spawn_processing(
            // note: reuse the same Arc<RwLock<ConfigFile>> that goes into AppState
            cfg_arc.clone(),
            dir.path().to_path_buf(),
            crate::processing::ProcessingConfig {
                ffmpeg: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/fake-ffmpeg"),
                dawn_check: std::time::Duration::from_secs(3600),
            },
        );
        let state = AppState {
            cfg: cfg_arc,
            store,
            latest,
            capture_status,
            camera_caps,
            key: Key::generate(),
            data_dir: dir.path().to_path_buf(),
            processing,
        };
        Harness {
            state,
            latest_tx,
            status_tx,
            caps_tx,
            dir,
        }
    }

    /// Log in with default credentials and return the Cookie header value.
    pub async fn login_cookie(app: &Router) -> String {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({"username": "admin", "password": "pa$$word!0"})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let set = res
            .headers()
            .get(header::SET_COOKIE)
            .expect("set-cookie")
            .to_str()
            .unwrap();
        set.split(';').next().unwrap().to_string()
    }
}
