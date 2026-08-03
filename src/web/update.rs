//! Self-update endpoints: version/release check and apply.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};

use super::AppState;

pub async fn get_update(State(state): State<AppState>) -> Json<crate::update::UpdateInfo> {
    Json(crate::update::check(&state.update).await)
}

pub async fn post_apply(State(state): State<AppState>) -> Response {
    // The exit-then-restart dance only works if systemd will actually run
    // the root apply hook on restart (new unit + hook installed). Without
    // it, staging+exiting leaves the camera down until someone manually
    // restarts the service — so refuse before even downloading anything.
    if !state.update.config.hook_path.is_file() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "self-update hook not installed on this host — re-run the installer (see DEVELOPMENT.md § Self-update) before using Update",
        )
            .into_response();
    }
    match crate::update::stage(&state.update, &state.data_dir).await {
        Ok(tag) => {
            let update = state.update.clone();
            // Respond first; exit shortly after so the 202 flushes.
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                tracing::info!("update to {tag} staged; exiting so systemd applies it");
                (update.exit_hook)();
            });
            StatusCode::ACCEPTED.into_response()
        }
        Err(crate::update::StageError::NoUpdate) => {
            (StatusCode::CONFLICT, "no newer release known").into_response()
        }
        Err(crate::update::StageError::Failed(msg)) => {
            tracing::error!("staging update: {msg}");
            (StatusCode::BAD_GATEWAY, msg).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    use crate::web::testing::{harness, login_cookie};

    /// Minimal curl stand-in. Logs every URL to calls.log; serves
    /// latest.json for the API URL and copies asset files for -o
    /// downloads. Assets dir is baked in at write time.
    pub(crate) fn write_fake_curl(dir: &Path, assets: &Path) -> PathBuf {
        let path = dir.join("fake-curl");
        let script = format!(
            r#"#!/bin/sh
out=""; prev=""; url=""
for a in "$@"; do
  [ "$prev" = "-o" ] && out="$a"
  prev="$a"; url="$a"
done
echo "$url" >> "{assets}/calls.log"
case "$url" in
  *releases/latest) cat "{assets}/latest.json" ;;
  *.tar.gz.sha256) cp "{assets}/asset.sha256" "$out" ;;
  *.tar.gz) cp "{assets}/asset.tar.gz" "$out" ;;
  *) exit 22 ;;
esac
"#,
            assets = assets.display()
        );
        std::fs::write(&path, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn update_state_with_fake_curl(
        h: &mut crate::web::testing::Harness,
        latest_json: &str,
    ) -> PathBuf {
        let assets = h.state.data_dir.join("gh-assets");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(assets.join("latest.json"), latest_json).unwrap();
        let curl = write_fake_curl(&h.state.data_dir, &assets);
        // Reuse whatever hook_path the harness already set up (a file that
        // genuinely exists), so apply tests exercise the real "staged and
        // ready to apply" path rather than tripping the hook-missing 503.
        let hook_path = h.state.update.config.hook_path.clone();
        h.state.update = Arc::new(crate::update::UpdateState::with_exit_hook(
            crate::update::UpdateConfig {
                curl,
                api_url: "http://gh.invalid/releases/latest".into(),
                download_base: "http://gh.invalid/releases/download".into(),
                hook_path,
            },
            Box::new(|| {}),
        ));
        assets
    }

    async fn get_update_body(app: &axum::Router, cookie: &str) -> serde_json::Value {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/update")
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = http_body_util::BodyExt::collect(res.into_body())
            .await
            .unwrap()
            .to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn update_endpoint_requires_a_session() {
        let h = harness();
        let app = crate::web::router(h.state.clone());
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/api/update")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn reports_newer_release_and_caches_the_check() {
        let mut h = harness();
        let assets = update_state_with_fake_curl(&mut h, r#"{"tag_name":"v99.0.0.1"}"#);
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;

        let body = get_update_body(&app, &cookie).await;
        assert_eq!(body["latest"], "v99.0.0.1");
        assert_eq!(body["updateAvailable"], true);
        assert_eq!(body["error"], serde_json::Value::Null);
        assert_eq!(body["current"], crate::version::full());

        // Second request must come from the cache: exactly one curl call.
        let _ = get_update_body(&app, &cookie).await;
        let calls = std::fs::read_to_string(assets.join("calls.log")).unwrap();
        assert_eq!(calls.lines().count(), 1, "check was not cached");
    }

    #[tokio::test]
    async fn check_failure_surfaces_as_error_not_update() {
        let h = harness(); // harness curl path doesn't exist -> check fails
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;
        let body = get_update_body(&app, &cookie).await;
        assert_eq!(body["updateAvailable"], false);
        assert_eq!(body["latest"], serde_json::Value::Null);
        assert!(body["error"].as_str().unwrap().contains("no-curl"));
    }

    /// Create a real mini tarball containing an executable `rskycam`
    /// stub, plus its correct .sha256 asset, in the assets dir.
    fn make_release_assets(assets: &Path) {
        std::fs::write(assets.join("rskycam"), b"#!/bin/sh\necho fake rskycam\n").unwrap();
        let ok = std::process::Command::new("tar")
            .args(["-czf", "asset.tar.gz", "rskycam"])
            .current_dir(assets)
            .status()
            .unwrap()
            .success();
        assert!(ok, "tar failed");
        let bytes = std::fs::read(assets.join("asset.tar.gz")).unwrap();
        let hex = crate::update::sha256_hex(&bytes);
        std::fs::write(
            assets.join("asset.sha256"),
            format!("{hex}  rskycam-aarch64.tar.gz\n"),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn apply_stages_the_release_and_fires_the_exit_hook() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let mut h = harness();
        let assets = update_state_with_fake_curl(&mut h, r#"{"tag_name":"v99.0.0.1"}"#);
        make_release_assets(&assets);
        let exited = Arc::new(AtomicBool::new(false));
        let flag = exited.clone();
        // Rebuild state with a flag-setting exit hook but the same config.
        let cfg = h.state.update.config.clone();
        h.state.update = Arc::new(crate::update::UpdateState::with_exit_hook(
            cfg,
            Box::new(move || flag.store(true, Ordering::SeqCst)),
        ));
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/update/apply")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);

        let update_dir = h.state.data_dir.join("update");
        assert!(update_dir.join("rskycam-aarch64.tar.gz").is_file());
        assert_eq!(
            std::fs::read_to_string(update_dir.join("tag")).unwrap(),
            "v99.0.0.1"
        );
        // Exit hook fires ~500ms after the response.
        for _ in 0..40 {
            if exited.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("exit hook never fired");
    }

    #[tokio::test]
    async fn apply_with_no_newer_release_is_409() {
        let mut h = harness();
        // Unparseable tag -> update_available false.
        update_state_with_fake_curl(&mut h, r#"{"tag_name":"garbage"}"#);
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/update/apply")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn checksum_mismatch_is_502_and_cleans_the_staging_dir() {
        let mut h = harness();
        let assets = update_state_with_fake_curl(&mut h, r#"{"tag_name":"v99.0.0.1"}"#);
        make_release_assets(&assets);
        std::fs::write(
            assets.join("asset.sha256"),
            "deadbeef  rskycam-aarch64.tar.gz\n",
        )
        .unwrap();
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/update/apply")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_GATEWAY);
        assert!(
            !h.state.data_dir.join("update").exists(),
            "staging dir not cleaned"
        );
    }

    #[tokio::test]
    async fn apply_endpoint_requires_a_session() {
        let h = harness();
        let app = crate::web::router(h.state.clone());
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/update/apply")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn apply_is_503_when_the_root_hook_is_not_installed() {
        let mut h = harness();
        let assets = update_state_with_fake_curl(&mut h, r#"{"tag_name":"v99.0.0.1"}"#);
        make_release_assets(&assets);
        // Point hook_path at a path that does not exist on disk — simulates
        // a host where the binary was updated by some other means (e.g.
        // scripts/deploy-pi.sh) without the installer ever (re-)installing
        // the systemd unit + hook.
        let cfg = crate::update::UpdateConfig {
            hook_path: h.state.data_dir.join("no-such-hook"),
            ..h.state.update.config.clone()
        };
        h.state.update = Arc::new(crate::update::UpdateState::with_exit_hook(
            cfg,
            Box::new(|| panic!("exit hook must not fire when the apply hook is missing")),
        ));
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/update/apply")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        // Nothing should have been downloaded/staged either — the check
        // happens before `stage()` is even called.
        assert!(
            !h.state.data_dir.join("update").exists(),
            "should not have staged anything when the hook is missing"
        );
    }

    #[tokio::test]
    async fn apply_succeeds_when_the_root_hook_is_installed() {
        // Confirms the new hook_path precondition doesn't false-positive
        // block a real apply when a genuinely-existing hook file is
        // configured (as the harness's default and production both do).
        use std::sync::atomic::{AtomicBool, Ordering};
        let mut h = harness();
        let assets = update_state_with_fake_curl(&mut h, r#"{"tag_name":"v99.0.0.1"}"#);
        make_release_assets(&assets);
        let hook_path = h.state.data_dir.join("fake-hook");
        std::fs::write(&hook_path, b"#!/bin/sh\nexit 0\n").unwrap();
        assert!(hook_path.is_file());
        let exited = Arc::new(AtomicBool::new(false));
        let flag = exited.clone();
        let cfg = crate::update::UpdateConfig {
            hook_path,
            ..h.state.update.config.clone()
        };
        h.state.update = Arc::new(crate::update::UpdateState::with_exit_hook(
            cfg,
            Box::new(move || flag.store(true, Ordering::SeqCst)),
        ));
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/update/apply")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);
        assert!(h.state.data_dir.join("update/tag").is_file());
        for _ in 0..40 {
            if exited.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("exit hook never fired");
    }
}
