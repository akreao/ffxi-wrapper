//! What a visitor may see: the home page, news, the rules. Every answer is built from a
//! fixed list of fields (pinned by tests), cached for 30 seconds and dropped on any edit.

use crate::cache::PublicCache;
use crate::content;
use crate::http::*;
use crate::markdown;
use crate::state::Shared;
use crate::store::NewsPost;
use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn api() -> Router<Shared> {
    Router::new()
        .route("/public/home", get(home))
        .route("/public/news", get(news))
        .route("/public/news/{id}", get(post))
        .route("/public/content/{key}", get(content_block))
}

/// The keys of `/api/public/home`. A test pins this list, so a new key can't leak by accident.
pub const HOME_KEYS: [&str; 10] =
    ["connect_address", "connect_html", "description_html", "footer_html", "news", "online", "server_name", "signup_open", "title", "up"];
/// The keys of each post in a list (home and `/api/public/news`).
pub const POST_LIST_KEYS: [&str; 6] = ["author", "excerpt", "id", "pinned", "published", "title"];
/// The keys of one post (`/api/public/news/{id}`).
pub const POST_KEYS: [&str; 6] = ["author", "body_html", "id", "pinned", "published", "title"];
/// The keys of `/api/public/news`.
pub const NEWS_KEYS: [&str; 4] = ["items", "page", "per", "total"];
/// The keys of `/api/public/content/{key}`.
pub const CONTENT_KEYS: [&str; 1] = ["html"];

const HOME_POSTS: i64 = 5;
const NEWS_PER_PAGE: i64 = 10;
const EXCERPT_CHARS: usize = 280;
/// A page number past this is an error, so a crawler can't make endless distinct cache keys.
const MAX_PAGE: i64 = 10_000;

fn limited(state: &Shared, ip: &str) -> ApiResult<()> {
    if state.limiter.public_get_allowed(ip, state.now()) {
        Ok(())
    } else {
        Err(ApiError::too_many("Too many requests. Try again in a minute."))
    }
}

fn cached(state: &Shared, key: &str) -> Option<Json<Value>> {
    state.public_cache.get(key, state.now()).map(Json)
}

fn remember(cache: &PublicCache, key: &str, v: &Value, now: i64) {
    cache.put(key, v, now);
}

fn summary(p: &NewsPost) -> Value {
    json!({
        "id": p.id, "title": p.title, "author": p.author,
        "published": p.published, "pinned": p.pinned,
        "excerpt": markdown::excerpt(&p.body, EXCERPT_CHARS),
    })
}

async fn home(State(state): State<Shared>, ClientIp(ip): ClientIp) -> ApiResult<Json<Value>> {
    limited(&state, &ip)?;
    if let Some(hit) = cached(&state, "home") {
        return Ok(hit);
    }
    // The page still loads while the database is down: it just says so.
    let online = state.game.public_online().await.ok();
    let stored = state.store.content_all().await?;
    let text = |key: &str| content::effective(key, &stored, &state.cfg).0;
    let (posts, _) = state.store.news_public(1, HOME_POSTS).await?;
    let out = json!({
        "server_name": state.cfg.server_name,
        "up": online.is_some(),
        "online": online,
        // The later sign-up PR turns this on.
        "signup_open": false,
        "title": text("home_title"),
        "description_html": markdown::render(&text("home_description")),
        "connect_html": markdown::render(&text("connect")),
        "connect_address": content::connect_address(&state.cfg),
        "footer_html": markdown::render(&text("footer")),
        "news": posts.iter().map(summary).collect::<Vec<_>>(),
    });
    remember(&state.public_cache, "home", &out, state.now());
    Ok(Json(out))
}

#[derive(Deserialize)]
struct NewsParams {
    page: Option<i64>,
}

async fn news(State(state): State<Shared>, ClientIp(ip): ClientIp, Query(p): Query<NewsParams>) -> ApiResult<Json<Value>> {
    limited(&state, &ip)?;
    let page = p.page.unwrap_or(1).clamp(1, MAX_PAGE);
    let key = format!("news:{page}");
    if let Some(hit) = cached(&state, &key) {
        return Ok(hit);
    }
    let (posts, total) = state.store.news_public(page, NEWS_PER_PAGE).await?;
    let out = json!({ "items": posts.iter().map(summary).collect::<Vec<_>>(), "page": page, "per": NEWS_PER_PAGE, "total": total });
    // An empty page past the end isn't kept: it would let a crawler fill the cache.
    if !posts.is_empty() || page == 1 {
        remember(&state.public_cache, &key, &out, state.now());
    }
    Ok(Json(out))
}

async fn post(State(state): State<Shared>, ClientIp(ip): ClientIp, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    limited(&state, &ip)?;
    let key = format!("post:{id}");
    if let Some(hit) = cached(&state, &key) {
        return Ok(hit);
    }
    let p = state.store.news_published_post(id).await?.ok_or_else(ApiError::not_found)?;
    let out = json!({
        "id": p.id, "title": p.title, "author": p.author, "published": p.published, "pinned": p.pinned,
        "body_html": markdown::render(&p.body),
    });
    remember(&state.public_cache, &key, &out, state.now());
    Ok(Json(out))
}

/// Only the rules are served by name; the other blocks travel with `/api/public/home`.
async fn content_block(State(state): State<Shared>, ClientIp(ip): ClientIp, Path(key): Path<String>) -> ApiResult<Json<Value>> {
    limited(&state, &ip)?;
    if key != "rules" {
        return Err(ApiError::not_found());
    }
    let ck = format!("content:{key}");
    if let Some(hit) = cached(&state, &ck) {
        return Ok(hit);
    }
    let stored = state.store.content_all().await?;
    let out = json!({ "html": markdown::render(&content::effective(&key, &stored, &state.cfg).0) });
    remember(&state.public_cache, &ck, &out, state.now());
    Ok(Json(out))
}
