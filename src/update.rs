//! Self-update: GitHub release check (this file also gains staging in
//! the apply flow). Network I/O shells out to curl — same injectable-
//! binary pattern as ffmpeg/rpicam — so tests use fixture scripts.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

const OK_TTL: Duration = Duration::from_secs(3600);
const ERR_TTL: Duration = Duration::from_secs(300);

#[derive(Clone)]
pub struct UpdateConfig {
    pub curl: PathBuf,
    pub api_url: String,
    pub download_base: String,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        UpdateConfig {
            curl: "curl".into(),
            api_url: "https://api.github.com/repos/awitwicki/rskycam/releases/latest".into(),
            download_base: "https://github.com/awitwicki/rskycam/releases/download".into(),
        }
    }
}

struct CacheEntry {
    at: Instant,
    tag: Option<String>,
    error: Option<String>,
}

pub struct UpdateState {
    pub config: UpdateConfig,
    cache: Mutex<Option<CacheEntry>>,
    /// Called after an update is staged; production exits the process so
    /// systemd restarts through the root apply hook. Tests inject a flag.
    pub exit_hook: Box<dyn Fn() + Send + Sync>,
}

impl UpdateState {
    pub fn new(config: UpdateConfig) -> Self {
        Self::with_exit_hook(config, Box::new(|| std::process::exit(0)))
    }

    pub fn with_exit_hook(config: UpdateConfig, exit_hook: Box<dyn Fn() + Send + Sync>) -> Self {
        UpdateState {
            config,
            cache: Mutex::new(None),
            exit_hook,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current: String,
    pub latest: Option<String>,
    pub update_available: bool,
    pub error: Option<String>,
}

fn info_from(current: String, tag: &Option<String>, error: &Option<String>) -> UpdateInfo {
    let update_available = tag
        .as_deref()
        .is_some_and(|t| crate::version::update_available(&current, t));
    UpdateInfo {
        current,
        latest: tag.clone(),
        update_available,
        error: error.clone(),
    }
}

/// Latest-release check, lazily cached (1 h on success, 5 min on failure
/// so an offline Pi doesn't run curl on every page load).
pub async fn check(state: &UpdateState) -> UpdateInfo {
    let current = crate::version::full().to_string();
    {
        let cache = state.cache.lock().expect("update cache poisoned");
        if let Some(e) = cache.as_ref() {
            let ttl = if e.error.is_none() { OK_TTL } else { ERR_TTL };
            if e.at.elapsed() < ttl {
                return info_from(current, &e.tag, &e.error);
            }
        }
    }
    let cfg = state.config.clone();
    let fetched = tokio::task::spawn_blocking(move || fetch_latest_tag(&cfg))
        .await
        .unwrap_or_else(|e| Err(format!("update check panicked: {e}")));
    let (tag, error) = match fetched {
        Ok(tag) => (Some(tag), None),
        Err(e) => (None, Some(e)),
    };
    let info = info_from(current, &tag, &error);
    *state.cache.lock().expect("update cache poisoned") = Some(CacheEntry {
        at: Instant::now(),
        tag,
        error,
    });
    info
}

fn fetch_latest_tag(cfg: &UpdateConfig) -> Result<String, String> {
    let out = std::process::Command::new(&cfg.curl)
        .args(["-fsSL", "--max-time", "10", &cfg.api_url])
        .output()
        .map_err(|e| format!("running {:?}: {e}", cfg.curl))?;
    if !out.status.success() {
        return Err(format!(
            "update check failed (curl exit {:?})",
            out.status.code()
        ));
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("parsing GitHub response: {e}"))?;
    v.get("tag_name")
        .and_then(|t| t.as_str())
        .map(str::to_owned)
        .ok_or_else(|| "no tag_name in GitHub response".into())
}
