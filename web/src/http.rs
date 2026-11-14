//! Shared HTTP plumbing: errors, who is asking, cookies, and the guards every
//! request passes (security headers; Origin, content type, CSRF and rate limit
//! for changes).

use crate::auth::{self, Role};
use crate::game::DbError;
use crate::state::Shared;
use crate::store::{Session, SessionKind};
use axum::body::Body;
use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::{header, request::Parts, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::net::SocketAddr;

pub const COOKIE: &str = "web_session";
pub const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; frame-ancestors 'none'";

// ---- errors ----

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
    pub extra: Option<serde_json::Value>,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> ApiError {
        ApiError { status, message: message.into(), extra: None }
    }
    pub fn with(mut self, extra: serde_json::Value) -> ApiError {
        self.extra = Some(extra);
        self
    }
    pub fn bad_request(m: &str) -> ApiError { ApiError::new(StatusCode::BAD_REQUEST, m) }
    pub fn unauthorized() -> ApiError { ApiError::new(StatusCode::UNAUTHORIZED, "Sign in first.") }
    pub fn forbidden() -> ApiError { ApiError::new(StatusCode::FORBIDDEN, "You can't do that.") }
    pub fn not_found() -> ApiError { ApiError::new(StatusCode::NOT_FOUND, "Not found.") }
    pub fn too_many(m: &str) -> ApiError { ApiError::new(StatusCode::TOO_MANY_REQUESTS, m) }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({ "error": self.message });
        if let (Some(extra), Some(map)) = (self.extra, body.as_object_mut()) {
            if let Some(e) = extra.as_object() {
                for (k, v) in e {
                    map.insert(k.clone(), v.clone());
                }
            }
        }
        (self.status, Json(body)).into_response()
    }
}

impl From<DbError> for ApiError {
    fn from(e: DbError) -> ApiError {
        match e {
            DbError::NotReady => ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "The game database isn't ready yet. If this lasts, the database update failed: check the server's log.",
            ),
            DbError::Failed(m) => {
                tracing::error!("database error: {m}");
                ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong on the server.")
            }
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> ApiError {
        tracing::error!("site store error: {e}");
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong on the server.")
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

// ---- who is asking ----

/// The address of the TCP peer. (Phase 4 teaches this about the proxy.)
pub struct ClientIp(pub String);

impl FromRequestParts<Shared> for ClientIp {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(parts: &mut Parts, _: &Shared) -> Result<Self, Self::Rejection> {
        Ok(ClientIp(client_ip(parts)))
    }
}

pub fn client_ip(parts: &Parts) -> String {
    parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// A signed-in request, with the role worked out afresh from the database.
#[derive(Debug, Clone)]
pub struct Identity {
    pub session: Session,
    pub role: Role,
    /// Highest GM level among the account's characters (5 for an owner link).
    pub gm: i64,
}

impl Identity {
    pub fn owner_link(&self) -> bool {
        self.session.kind == SessionKind::Owner
    }
    pub fn accid(&self) -> Option<i64> {
        self.session.accid
    }
    pub fn login(&self) -> Option<&str> {
        self.session.login.as_deref()
    }
    pub fn actor(&self) -> auth::Actor {
        auth::Actor { role: self.role, gm: self.gm, owner_link: self.owner_link() }
    }
}

pub fn cookie_value(headers: &HeaderMap) -> Option<String> {
    for h in headers.get_all(header::COOKIE) {
        let Ok(s) = h.to_str() else { continue };
        for part in s.split(';') {
            if let Some(v) = part.trim().strip_prefix(&format!("{COOKIE}=")) {
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

pub fn set_cookie(state: &Shared, value: &str, max_age: i64) -> HeaderValue {
    let secure = if state.cfg.secure_cookies() { "; Secure" } else { "" };
    HeaderValue::from_str(&format!("{COOKIE}={value}; HttpOnly; SameSite=Strict; Path=/; Max-Age={max_age}{secure}"))
        .expect("cookie values are base64url")
}

/// Looks up the session and re-reads the account, so a ban or demotion takes effect at once.
pub async fn resolve(state: &Shared, headers: &HeaderMap) -> ApiResult<Option<Identity>> {
    let Some(cookie) = cookie_value(headers) else { return Ok(None) };
    let Some(session) = state.store.session(&cookie).await? else { return Ok(None) };
    let now = state.now();
    let (role, gm) = match session.kind {
        SessionKind::Owner => (Role::Owner, 5),
        SessionKind::Account => {
            let accid = session.accid.unwrap_or(0);
            match state.game.standing(accid).await? {
                Some(s) if s.status & 1 == 1 => (auth::role_for_gm(s.gm), s.gm),
                _ => {
                    // Gone, banned or disabled: the session ends.
                    state.store.end_session(&session.id_hash).await?;
                    return Ok(None);
                }
            }
        }
    };
    if !auth::session_alive(role, &session, now) {
        state.store.end_session(&session.id_hash).await?;
        return Ok(None);
    }
    state.store.touch(&session, now).await?;
    Ok(Some(Identity { session, role, gm }))
}

/// Anyone: the session if there is one.
pub struct Viewer(pub Option<Identity>);

impl FromRequestParts<Shared> for Viewer {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &Shared) -> Result<Self, ApiError> {
        Ok(Viewer(resolve(state, &parts.headers).await?))
    }
}

/// Any signed-in session.
pub struct Signed(pub Identity);

impl FromRequestParts<Shared> for Signed {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &Shared) -> Result<Self, ApiError> {
        resolve(state, &parts.headers).await?.map(Signed).ok_or_else(ApiError::unauthorized)
    }
}

/// A signed-in game account (Player and up). An owner-link session has no account: 404.
pub struct Player(pub Identity, pub i64);

impl FromRequestParts<Shared> for Player {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &Shared) -> Result<Self, ApiError> {
        let id = resolve(state, &parts.headers).await?.ok_or_else(ApiError::unauthorized)?;
        match id.accid() {
            Some(accid) => Ok(Player(id, accid)),
            None => Err(ApiError::not_found()),
        }
    }
}

/// Moderator or Owner.
pub struct Staff(pub Identity);

impl FromRequestParts<Shared> for Staff {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &Shared) -> Result<Self, ApiError> {
        let id = resolve(state, &parts.headers).await?.ok_or_else(ApiError::unauthorized)?;
        if id.role.is_admin() { Ok(Staff(id)) } else { Err(ApiError::forbidden()) }
    }
}

// ---- guards ----

/// The scheme and any `://` stripped from an Origin: `http://host:54080` -> `host:54080`.
fn origin_host(origin: &str) -> Option<&str> {
    origin.split_once("://").map(|(_, h)| h.trim_end_matches('/'))
}

/// Does the Origin header (when there is one) name this request's own Host?
pub fn origin_matches(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else { return true };
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
    match (origin.to_str().ok().and_then(origin_host), host) {
        (Some(o), Some(h)) => o.eq_ignore_ascii_case(h),
        _ => false,
    }
}

fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case("application/json"))
}

/// Paths that change things without a session yet: they get the Origin check only.
fn needs_no_csrf(path: &str) -> bool {
    matches!(path, "/api/auth/login" | "/api/public/signup")
}

/// Every `POST`, `PUT`, `DELETE` and `PATCH` needs a JSON body, a matching Origin and,
/// for anything past sign-in, the session's CSRF token; and no more than 30 a minute.
pub async fn guard_changes(State(state): State<Shared>, req: Request, next: Next) -> Response {
    if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(req).await;
    }
    let h = req.headers();
    if !origin_matches(h) {
        return ApiError::forbidden().into_response();
    }
    if !is_json(h) {
        return ApiError::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Send JSON.").into_response();
    }
    // Inside the nested /api router the URI has lost its prefix; the original keeps it.
    let path = req.extensions().get::<axum::extract::OriginalUri>().map(|u| u.0.path()).unwrap_or_else(|| req.uri().path());
    if !needs_no_csrf(path) {
        let Some(cookie) = cookie_value(h) else { return ApiError::unauthorized().into_response() };
        let session = match state.store.session(&cookie).await {
            Ok(Some(s)) => s,
            Ok(None) => return ApiError::unauthorized().into_response(),
            Err(e) => return ApiError::from(e).into_response(),
        };
        let sent = h.get("x-csrf-token").and_then(|v| v.to_str().ok()).unwrap_or("");
        if !constant_time_eq(sent.as_bytes(), session.csrf.as_bytes()) {
            return ApiError::new(StatusCode::FORBIDDEN, "The page is out of date. Reload it and try again.").into_response();
        }
        let key: String = session.id_hash.iter().map(|b| format!("{b:02x}")).collect();
        if !state.limiter.write_allowed(&key, state.now()) {
            return ApiError::too_many("Slow down: too many changes in a minute.").into_response();
        }
    }
    next.run(req).await
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Headers every response carries.
pub async fn secure_headers(req: Request, next: Next) -> Response<Body> {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    res
}
