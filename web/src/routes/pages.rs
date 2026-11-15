//! The pages (static HTML from `web/ui/`, drawn by `app.js` from the JSON API),
//! the assets under `/ui/`, and `/healthz`.
//!
//! Page access is decided here, before any HTML goes out: a player is never
//! sent the admin page, so there are no admin links in what a player receives.

use crate::http::*;
use crate::state::Shared;
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use rust_embed::RustEmbed;

/// `web/ui/`: pages, `app.js`, `web.css`, `site.css`, `theme.js`.
#[derive(RustEmbed)]
#[folder = "ui/"]
struct WebUi;

/// The parts of the desktop app's UI the site shares.
#[derive(RustEmbed)]
#[folder = "../desktop/ui/"]
#[include = "tokens.css"]
#[include = "fonts/*"]
#[include = "modern/*"]
struct DesktopUi;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    /// Everyone.
    Anyone,
    /// Any signed-in game account; others go to the sign-in page.
    SignedIn,
    /// Moderator and Owner; a player gets "not found".
    Staff,
    /// The owner alone; everyone else, moderators too, gets "not found".
    Owner,
}

pub fn router() -> Router<Shared> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/", get(|s, v| page(s, v, "home.html", Access::Anyone)))
        .route("/news", get(|s, v| page(s, v, "news.html", Access::Anyone)))
        .route("/news/{id}", get(|s, v, _p: Path<String>| page(s, v, "post.html", Access::Anyone)))
        .route("/rules", get(|s, v| page(s, v, "rules.html", Access::Anyone)))
        .route("/online", get(|s, v| page(s, v, "online.html", Access::Anyone)))
        .route("/rankings", get(|s, v| page(s, v, "rankings.html", Access::Anyone)))
        .route("/signin", get(|s, v| page(s, v, "signin.html", Access::Anyone)))
        .route("/signup", get(|s, v| page(s, v, "signup.html", Access::Anyone)))
        .route("/me", get(|s, v| page(s, v, "me.html", Access::SignedIn)))
        .route("/me/characters/{id}", get(|s, v, _p: Path<String>| page(s, v, "me-character.html", Access::SignedIn)))
        .route("/admin", get(|s, v| page(s, v, "admin.html", Access::Staff)))
        .route("/admin/accounts", get(|s, v| page(s, v, "accounts.html", Access::Staff)))
        .route("/admin/accounts/{id}", get(|s, v, _p: Path<String>| page(s, v, "account.html", Access::Staff)))
        .route("/admin/characters", get(|s, v| page(s, v, "characters.html", Access::Staff)))
        .route("/admin/characters/{id}", get(|s, v, _p: Path<String>| page(s, v, "character.html", Access::Staff)))
        .route("/admin/bans", get(|s, v| page(s, v, "bans.html", Access::Staff)))
        .route("/admin/help-desk", get(|s, v| page(s, v, "help-desk.html", Access::Staff)))
        .route("/admin/log", get(|s, v| page(s, v, "log.html", Access::Staff)))
        .route("/admin/news", get(|s, v| page(s, v, "admin-news.html", Access::Owner)))
        .route("/admin/news/{id}", get(|s, v, _p: Path<String>| page(s, v, "admin-news.html", Access::Owner)))
        .route("/admin/content", get(|s, v| page(s, v, "admin-content.html", Access::Owner)))
        .route("/admin/invites", get(|s, v| page(s, v, "admin-invites.html", Access::Owner)))
        .route("/ui/{*path}", get(asset))
}

async fn healthz(State(state): State<Shared>) -> Response {
    match state.game.ping().await {
        Ok(()) => "ok".into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "the database doesn't answer").into_response(),
    }
}

fn read_ui(state: &Shared, name: &str) -> Option<Vec<u8>> {
    if cfg!(debug_assertions) {
        if let Some(dir) = &state.cfg.ui_dir {
            if let Ok(b) = std::fs::read(dir.join(name)) {
                return Some(b);
            }
        }
    }
    WebUi::get(name).or_else(|| DesktopUi::get(name)).map(|f| f.data.into_owned())
}

async fn page(State(state): State<Shared>, Viewer(id): Viewer, file: &'static str, access: Access) -> Response {
    match (access, &id) {
        (Access::SignedIn, None) => return Redirect::to(&format!("/signin?next={}", "/me")).into_response(),
        (Access::SignedIn, Some(i)) if i.owner_link() => return Redirect::to("/admin").into_response(),
        (Access::Staff, None) => return Redirect::to("/signin?next=/admin").into_response(),
        (Access::Owner, None) => return Redirect::to("/signin?next=/admin").into_response(),
        (Access::Owner, Some(i)) if i.role != crate::auth::Role::Owner => return (StatusCode::NOT_FOUND, "Not found").into_response(),
        (Access::Staff, Some(i)) if !i.role.is_admin() => return (StatusCode::NOT_FOUND, "Not found").into_response(),
        _ => {}
    }
    match read_ui(&state, file) {
        Some(body) => {
            let mut res = (StatusCode::OK, body).into_response();
            let h = res.headers_mut();
            h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
            h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            res
        }
        None => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

async fn asset(State(state): State<Shared>, Path(path): Path<String>) -> Response {
    // Pages are served by their routes, never by file name.
    if path.ends_with(".html") || path.contains("..") {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    match read_ui(&state, &path) {
        Some(body) => {
            let mime = mime_for(&path);
            let mut res = (StatusCode::OK, body).into_response();
            let h = res.headers_mut();
            h.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
            h.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=300"));
            res
        }
        None => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "svg" => "image/svg+xml",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
