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

pub enum StageError {
    /// No newer release is known — nothing to do (HTTP 409).
    NoUpdate,
    /// Download/verify failed; update dir cleaned, service keeps running.
    Failed(String),
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Download + verify the newest release into `<data_dir>/update/` and
/// write the `tag` marker the root apply hook consumes. Does NOT exit —
/// the caller decides when to fire the exit hook.
pub async fn stage(state: &UpdateState, data_dir: &std::path::Path) -> Result<String, StageError> {
    let info = check(state).await;
    if !info.update_available {
        return Err(StageError::NoUpdate);
    }
    let tag = info.latest.ok_or(StageError::NoUpdate)?;
    let cfg = state.config.clone();
    let dir = data_dir.join("update");
    let staged_tag = tag.clone();
    tokio::task::spawn_blocking(move || stage_blocking(&cfg, &dir, &staged_tag))
        .await
        .map_err(|e| StageError::Failed(format!("staging task panicked: {e}")))??;
    Ok(tag)
}

fn stage_blocking(cfg: &UpdateConfig, dir: &std::path::Path, tag: &str) -> Result<(), StageError> {
    let fail = |msg: String| {
        let _ = std::fs::remove_dir_all(dir);
        Err(StageError::Failed(msg))
    };
    let _ = std::fs::remove_dir_all(dir);
    if let Err(e) = std::fs::create_dir_all(dir) {
        return fail(format!("creating {}: {e}", dir.display()));
    }
    let tarball = dir.join("rskycam-aarch64.tar.gz");
    let sha_file = dir.join("rskycam-aarch64.tar.gz.sha256");
    let base = format!("{}/{tag}", cfg.download_base);
    if let Err(e) = curl_download(
        cfg,
        &format!("{base}/rskycam-aarch64.tar.gz"),
        &tarball,
        300,
    ) {
        return fail(e);
    }
    if let Err(e) = curl_download(
        cfg,
        &format!("{base}/rskycam-aarch64.tar.gz.sha256"),
        &sha_file,
        30,
    ) {
        return fail(e);
    }
    let expected = match std::fs::read_to_string(&sha_file) {
        Ok(s) => match s.split_whitespace().next() {
            Some(hex) => hex.to_ascii_lowercase(),
            None => return fail("empty .sha256 asset".into()),
        },
        Err(e) => return fail(format!("reading checksum: {e}")),
    };
    let actual = match std::fs::read(&tarball) {
        Ok(bytes) => sha256_hex(&bytes),
        Err(e) => return fail(format!("reading tarball: {e}")),
    };
    if expected != actual {
        return fail(format!(
            "checksum mismatch: expected {expected}, got {actual}"
        ));
    }
    if let Err(e) = std::fs::write(dir.join("tag"), tag) {
        return fail(format!("writing tag marker: {e}"));
    }
    Ok(())
}

fn curl_download(
    cfg: &UpdateConfig,
    url: &str,
    out: &std::path::Path,
    max_time_s: u32,
) -> Result<(), String> {
    let status = std::process::Command::new(&cfg.curl)
        .args(["-fsSL", "--max-time", &max_time_s.to_string(), "-o"])
        .arg(out)
        .arg(url)
        .status()
        .map_err(|e| format!("running {:?}: {e}", cfg.curl))?;
    if !status.success() {
        return Err(format!(
            "downloading {url} failed (curl exit {:?})",
            status.code()
        ));
    }
    Ok(())
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
