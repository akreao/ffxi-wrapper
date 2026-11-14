//! Signing in and out: game accounts, and the owner link.

use crate::auth::{self, HashCheck, Role};
use crate::http::*;
use crate::limits::{self, Limiter, Target};
use crate::model::AccountAuth;
use crate::state::Shared;
use crate::store::{NewAction, NewSession, SessionKind};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

pub fn api() -> Router<Shared> {
    Router::new()
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/me", get(whoami))
}

pub fn pages() -> Router<Shared> {
    Router::new().route("/auth/owner", get(owner_link))
}

#[derive(Deserialize)]
pub struct LoginBody {
    #[serde(default)]
    login: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    otp: Option<String>,
}

/// Logs a sign-in outcome; a failure to write the log never fails the request. An address that is
/// the proxy's (Docker's gateway, loopback) is marked, since it names no one.
pub async fn record(state: &Shared, mut a: NewAction) {
    if let Some(ip) = &a.ip {
        if limits::is_proxy_addr(ip) {
            a.ip = Some(format!("{ip} (proxy)"));
        }
    }
    if let Err(e) = state.store.log(a, state.now()).await {
        tracing::warn!("couldn't write the action log: {e}");
    }
}

pub fn role_name(r: Role) -> &'static str {
    r.as_str()
}

/// Waits for one of the bcrypt slots, but not for long: a site that is swamped says so
/// instead of leaving the browser waiting.
pub async fn bcrypt_slot(state: &Shared) -> ApiResult<tokio::sync::SemaphorePermit<'_>> {
    match tokio::time::timeout(std::time::Duration::from_secs(5), state.bcrypt_slots.acquire()).await {
        Ok(Ok(permit)) => Ok(permit),
        _ => Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "The site is busy. Try again in a moment.")),
    }
}

/// Checks a password against an account's hash. An account that doesn't exist, or whose hash is
/// one the site can't verify (an old `PASSWORD()` hash), is checked against the dummy hash, so
/// the answer and the time are the same as for a wrong password.
pub async fn check_password(state: &Shared, account: Option<&AccountAuth>, password: &str) -> ApiResult<bool> {
    let real = account.filter(|a| auth::is_supported_hash(&a.password));
    let hash = real.map(|a| a.password.clone()).unwrap_or_else(|| state.dummy_hash.clone());
    let pw = password.to_string();
    // Cost 12 takes about 250 ms: keep it off the async threads, and a few at a time.
    let _slot = bcrypt_slot(state).await?;
    let out = tokio::task::spawn_blocking(move || auth::check_password(&pw, &hash)).await.unwrap_or(HashCheck::Mismatch);
    Ok(real.is_some() && out == HashCheck::Match)
}

/// The failed-attempt answer. The attempt was already counted when it was reserved. The log gets
/// the account when the login matched one, never the text typed into the login box (people type
/// their password there).
async fn fail(state: &Shared, ip: &str, account: Option<&AccountAuth>, keys: &limits::SignInKeys, why: &str, locked: bool) -> ApiError {
    let now = state.now();
    record(
        state,
        NewAction {
            actor_kind: "visitor",
            actor_accid: account.map(|a| a.id),
            actor_login: account.map(|a| a.login.clone()),
            actor_role: "visitor",
            ip: Some(ip.to_string()),
            action: "sign_in_failed",
            reason: Some(if account.is_some() { why } else { "unknown login" }.to_string()),
            result: if locked { "refused" } else { "error" },
            ..Default::default()
        },
    )
    .await;
    let left = state.limiter.pair_tries_left(keys, now);
    if locked || left == 0 {
        return ApiError::too_many("Too many tries. Wait 15 minutes and try again.");
    }
    ApiError::new(StatusCode::UNAUTHORIZED, "Wrong login or password.").with(json!({ "tries_left": left }))
}

async fn login(
    State(state): State<Shared>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> ApiResult<Response> {
    let now = state.now();
    if !auth::valid_login(&body.login) || !auth::valid_password(&body.password) {
        let keys = Limiter::sign_in_keys(&ip, &Target::unknown(""));
        if !state.limiter.reserve_sign_in(&keys, now) {
            return Err(fail(&state, &ip, None, &keys, "locked out", true).await);
        }
        return Err(fail(&state, &ip, None, &keys, "malformed", false).await);
    }
    let account = state.game.account_by_login(&body.login).await?;
    // The attempt is counted before the password is looked at, all buckets at once, so a burst
    // of simultaneous tries can't all pass the check.
    let target = match &account {
        Some(a) => Target::Account(a.id),
        None => Target::unknown(&body.login),
    };
    let keys = Limiter::sign_in_keys(&ip, &target);
    if !state.limiter.reserve_sign_in(&keys, now) {
        return Err(fail(&state, &ip, account.as_ref(), &keys, "locked out", true).await);
    }
    if !check_password(&state, account.as_ref(), &body.password).await? {
        return Err(fail(&state, &ip, account.as_ref(), &keys, "wrong password", false).await);
    }
    let account = account.expect("a matching hash means the account exists");
    if account.status & 1 == 0 {
        return Err(fail(&state, &ip, Some(&account), &keys, "account can't sign in", false).await.with_message("This account can't sign in."));
    }

    // Two-step sign-in, when the account has it switched on. Asking for the code counts as an
    // attempt (it already told the caller the password was right), so a password can't be tried
    // without paying for it.
    if let Some(t) = state.game.totp(account.id).await?.filter(|t| t.validated) {
        let Some(code) = body.otp.as_deref().filter(|c| !c.trim().is_empty()) else {
            return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Enter the code from your authenticator app.").with(json!({ "need_otp": true })));
        };
        let step = auth::check_totp(&t.secret, code, now as u64);
        let fresh = match step {
            Some(s) => {
                let mut seen = state.totp_seen.lock().unwrap();
                let ok = seen.get(&account.id).is_none_or(|last| s > *last);
                if ok {
                    seen.insert(account.id, s);
                }
                ok
            }
            None => false,
        };
        if !fresh {
            let e = fail(&state, &ip, Some(&account), &keys, "wrong code", false).await;
            return Err(if e.status == StatusCode::UNAUTHORIZED {
                ApiError::new(StatusCode::UNAUTHORIZED, "That code didn't work. Try the next one.").with(json!({ "need_otp": true }))
            } else {
                e
            });
        }
    }

    state.limiter.sign_in_succeeded(&keys);
    let gm = state.game.standing(account.id).await?.map(|s| s.gm).unwrap_or(0);
    let role = auth::role_for_gm(gm);
    let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok());
    let lifetime = auth::new_session_lifetime(role, &SessionKind::Account);
    let (cookie, csrf) = state
        .store
        .create_session(NewSession { kind: SessionKind::Account, accid: Some(account.id), login: Some(&account.login), ip: &ip, user_agent: ua, now, lifetime })
        .await?;
    record(
        &state,
        NewAction {
            actor_kind: "account",
            actor_accid: Some(account.id),
            actor_login: Some(account.login.clone()),
            actor_role: role_name(role),
            ip: Some(ip),
            action: "sign_in",
            result: "ok",
            ..Default::default()
        },
    )
    .await;
    let mut res = Json(json!({ "ok": true, "role": role.as_str(), "csrf": csrf, "next": "/me" })).into_response();
    let cookie_header = if role.is_admin() {
        // Staff sessions end with the browser session as well.
        let secure = if state.cfg.secure_cookies() { "; Secure" } else { "" };
        axum::http::HeaderValue::from_str(&format!("{COOKIE}={cookie}; HttpOnly; SameSite=Strict; Path=/{secure}")).unwrap()
    } else {
        set_cookie(&state, &cookie, lifetime)
    };
    res.headers_mut().insert(header::SET_COOKIE, cookie_header);
    Ok(res)
}

impl ApiError {
    fn with_message(mut self, m: &str) -> ApiError {
        if self.status == StatusCode::UNAUTHORIZED {
            self.message = m.to_string();
        }
        self
    }
}

async fn logout(State(state): State<Shared>, Signed(id): Signed) -> ApiResult<Response> {
    state.store.end_session(&id.session.id_hash).await?;
    let mut res = Json(json!({ "ok": true })).into_response();
    res.headers_mut().insert(header::SET_COOKIE, set_cookie(&state, "", 0));
    Ok(res)
}

/// Who is asking, what they may see, and the token their changes need.
async fn whoami(State(state): State<Shared>, Viewer(id): Viewer) -> ApiResult<Json<serde_json::Value>> {
    let server_name = &state.cfg.server_name;
    Ok(Json(match id {
        None => json!({ "role": "visitor", "server_name": server_name }),
        Some(id) => json!({
            "role": id.role.as_str(),
            "login": id.login(),
            "accid": id.accid(),
            "owner_link": id.owner_link(),
            "csrf": id.session.csrf,
            "server_name": server_name,
        }),
    }))
}

#[derive(Deserialize)]
struct OwnerQuery {
    t: Option<String>,
}

fn owner_page(status: StatusCode, message: &str) -> Response {
    let body = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>Owner link</title><script src=\"/ui/theme.js\"></script><link rel=\"stylesheet\" href=\"/ui/tokens.css\"><link rel=\"stylesheet\" href=\"/ui/web.css\"></head>\
         <body class=\"signin\"><div class=\"card signin-card\"><h2>Owner link</h2><p>{message}</p><p><a href=\"/signin\">Sign in with a game account</a></p></div></body></html>"
    );
    (status, Html(body)).into_response()
}

/// `GET /auth/owner?t=v1....`: a one-minute link signed by the desktop app.
async fn owner_link(State(state): State<Shared>, ClientIp(ip): ClientIp, headers: HeaderMap, Query(q): Query<OwnerQuery>) -> Response {
    // No usable secret: the route doesn't exist.
    if wrapper_core::owner_link::parse_secret(&state.cfg.admin_secret).is_err() {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    let now = state.now();
    let verified = match wrapper_core::owner_link::verify(&state.cfg.admin_secret, q.t.as_deref().unwrap_or(""), now as u64) {
        Ok(v) => v,
        Err(e) => {
            record(&state, NewAction { actor_kind: "visitor", actor_role: "visitor", ip: Some(ip), action: "owner_link", reason: Some(e.to_string()), result: "refused", ..Default::default() }).await;
            return owner_page(StatusCode::FORBIDDEN, "This link has expired or isn't valid. Open the website again from the desktop app.");
        }
    };
    match state.store.use_nonce(&verified.nonce, verified.exp as i64).await {
        Ok(true) => {}
        Ok(false) => {
            record(&state, NewAction { actor_kind: "visitor", actor_role: "visitor", ip: Some(ip), action: "owner_link", reason: Some("link used twice".into()), result: "refused", ..Default::default() }).await;
            return owner_page(StatusCode::FORBIDDEN, "This link was already used. Open the website again from the desktop app.");
        }
        Err(e) => return ApiError::from(e).into_response(),
    }
    let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok());
    let lifetime = auth::new_session_lifetime(Role::Owner, &SessionKind::Owner);
    let (cookie, _) = match state.store.create_session(NewSession { kind: SessionKind::Owner, accid: None, login: None, ip: &ip, user_agent: ua, now, lifetime }).await {
        Ok(c) => c,
        Err(e) => return ApiError::from(e).into_response(),
    };
    record(&state, NewAction { actor_kind: "owner", actor_role: "owner", ip: Some(ip), action: "sign_in", result: "ok", ..Default::default() }).await;
    let secure = if state.cfg.secure_cookies() { "; Secure" } else { "" };
    let mut res = Redirect::to("/admin").into_response();
    res.headers_mut().insert(
        header::SET_COOKIE,
        axum::http::HeaderValue::from_str(&format!("{COOKIE}={cookie}; HttpOnly; SameSite=Strict; Path=/{secure}")).unwrap(),
    );
    res
}
