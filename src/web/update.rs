//! Self-update endpoints: version/release check and apply.

use axum::extract::State;
use axum::response::Json;

use super::AppState;

pub async fn get_update(State(state): State<AppState>) -> Json<crate::update::UpdateInfo> {
    Json(crate::update::check(&state.update).await)
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
        h.state.update = Arc::new(crate::update::UpdateState::with_exit_hook(
            crate::update::UpdateConfig {
                curl,
                api_url: "http://gh.invalid/releases/latest".into(),
                download_base: "http://gh.invalid/releases/download".into(),
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
}
