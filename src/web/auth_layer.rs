use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum_extra::extract::cookie::{Cookie, SameSite, SignedCookieJar};
use serde::Deserialize;

use crate::web::AppState;

pub const SESSION_COOKIE: &str = "rskycam_session";
pub const ADMIN_USERNAME: &str = "admin";

fn session_cookie() -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, ADMIN_USERNAME))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(time::Duration::days(7))
        .build()
}

#[derive(Deserialize)]
pub struct LoginBody {
    username: String,
    password: String,
}

pub async fn login(
    State(state): State<AppState>,
    jar: SignedCookieJar,
    Json(body): Json<LoginBody>,
) -> Response {
    if body.username != ADMIN_USERNAME {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let hash = state.cfg.read().await.password_hash.clone();
    let password = body.password;
    // Argon2 verification costs tens–hundreds of ms; keep it off the async
    // workers so it can't stall the capture loop or SSE ticks.
    let ok = tokio::task::spawn_blocking(move || crate::auth::verify_password(&password, &hash))
        .await
        .unwrap_or(false);
    if !ok {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    (jar.add(session_cookie()), StatusCode::NO_CONTENT).into_response()
}

pub async fn logout(jar: SignedCookieJar) -> Response {
    let removal = Cookie::build(SESSION_COOKIE).path("/").build();
    (jar.remove(removal), StatusCode::NO_CONTENT).into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangePasswordBody {
    old_password: String,
    new_password: String,
}

pub async fn change_password(
    State(state): State<AppState>,
    Json(body): Json<ChangePasswordBody>,
) -> Response {
    // Verify the old password and hash the new one off the async runtime —
    // argon2 costs tens–hundreds of ms and must not stall other workers —
    // and hold the write lock only long enough to adopt the saved result.
    let current_hash = state.cfg.read().await.password_hash.clone();
    let old = body.old_password;
    let verified =
        tokio::task::spawn_blocking(move || crate::auth::verify_password(&old, &current_hash))
            .await
            .unwrap_or(false);
    if !verified {
        return StatusCode::FORBIDDEN.into_response();
    }
    let new_pw = body.new_password;
    let new_hash =
        match tokio::task::spawn_blocking(move || crate::auth::hash_password(&new_pw)).await {
            Ok(Ok(h)) => h,
            Ok(Err(e)) => {
                tracing::error!("hashing new password: {e:#}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            Err(e) => {
                tracing::error!("hashing new password task panicked: {e:#}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
    // Build + persist the candidate off the lock, then adopt under a brief lock.
    let mut candidate = state.cfg.read().await.clone();
    candidate.password_hash = new_hash;
    let store = state.store.clone();
    let to_save = candidate.clone();
    let saved = tokio::task::spawn_blocking(move || store.save(&to_save)).await;
    if !matches!(saved, Ok(Ok(()))) {
        tracing::error!("persisting new password failed");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    // Adopt only our field so a concurrent settings write can't be clobbered
    // by our (possibly stale) snapshot of the rest of the config.
    state.cfg.write().await.password_hash = candidate.password_hash;
    StatusCode::NO_CONTENT.into_response()
}

pub async fn require_session(jar: SignedCookieJar, req: Request, next: Next) -> Response {
    match jar.get(SESSION_COOKIE) {
        Some(c) if c.value() == ADMIN_USERNAME => next.run(req).await,
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    use crate::web::testing::{harness, login_cookie};

    fn req(
        method: &str,
        uri: &str,
        cookie: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> Request<Body> {
        let mut b = Request::builder().method(method).uri(uri);
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c);
        }
        match body {
            Some(v) => b
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(v.to_string()))
                .unwrap(),
            None => b.body(Body::empty()).unwrap(),
        }
    }

    #[tokio::test]
    async fn login_rejects_wrong_and_accepts_default_credentials() {
        let h = harness();
        let app = crate::web::router(h.state.clone());
        let bad = app
            .clone()
            .oneshot(req(
                "POST",
                "/api/login",
                None,
                Some(serde_json::json!({"username": "admin", "password": "nope"})),
            ))
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::UNAUTHORIZED);
        let ok = app
            .oneshot(req(
                "POST",
                "/api/login",
                None,
                Some(serde_json::json!({"username": "admin", "password": "pa$$word!0"})),
            ))
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::NO_CONTENT);
        assert!(ok.headers().get(header::SET_COOKIE).is_some());
    }

    #[tokio::test]
    async fn protected_routes_require_a_valid_session() {
        let h = harness();
        let app = crate::web::router(h.state.clone());
        let anon = app
            .clone()
            .oneshot(req("GET", "/api/status", None, None))
            .await
            .unwrap();
        assert_eq!(anon.status(), StatusCode::UNAUTHORIZED);
        let forged = app
            .clone()
            .oneshot(req(
                "GET",
                "/api/status",
                Some("rskycam_session=admin"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(forged.status(), StatusCode::UNAUTHORIZED); // unsigned cookie must fail
        let cookie = login_cookie(&app).await;
        let authed = app
            .oneshot(req("GET", "/api/status", Some(&cookie), None))
            .await
            .unwrap();
        assert_ne!(authed.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn change_password_requires_old_password_and_persists() {
        let h = harness();
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;
        let wrong = app
            .clone()
            .oneshot(req(
                "POST",
                "/api/change-password",
                Some(&cookie),
                Some(serde_json::json!({"oldPassword": "bad", "newPassword": "hunter2"})),
            ))
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::FORBIDDEN);
        let ok = app
            .clone()
            .oneshot(req(
                "POST",
                "/api/change-password",
                Some(&cookie),
                Some(serde_json::json!({"oldPassword": "pa$$word!0", "newPassword": "hunter2"})),
            ))
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::NO_CONTENT);
        let relogin = app
            .oneshot(req(
                "POST",
                "/api/login",
                None,
                Some(serde_json::json!({"username": "admin", "password": "hunter2"})),
            ))
            .await
            .unwrap();
        assert_eq!(relogin.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn logout_instructs_the_client_to_drop_the_cookie() {
        let h = harness();
        let app = crate::web::router(h.state.clone());
        let cookie = login_cookie(&app).await;
        let res = app
            .oneshot(req("POST", "/api/logout", Some(&cookie), None))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
        let set = res
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(set.starts_with("rskycam_session="));
        let lower = set.to_lowercase();
        assert!(lower.contains("max-age=0") || lower.contains("expires="));
        assert!(lower.contains("path=/"));
    }
}
