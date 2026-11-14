//! What a visitor may see: the home page, news, the rules, who is online, and the
//! rankings. Only character names, nations, jobs and levels leave here; the
//! rules of who is left out live in the `Game` queries and are tested.

use crate::cache::PublicCache as NewsCache;
use crate::content;
use crate::http::*;
use crate::markdown;
use crate::model::*;
use crate::state::Shared;
use crate::store::NewsPost;
use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub fn api() -> Router<Shared> {
    Router::new()
        .route("/public/home", get(home))
        .route("/public/news", get(news))
        .route("/public/news/{id}", get(post))
        .route("/public/content/{key}", get(content_block))
        .route("/public/online", get(online))
        .route("/public/rankings/{board}", get(rankings))
}

/// How long a public answer is reused before the database is asked again.
pub const CACHE_SECS: i64 = 30;
/// Rows per page of a ranking.
pub const PER_PAGE: usize = 25;

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

/// The keys of `/api/public/online`, and of each of its players.
pub const ONLINE_KEYS: [&str; 3] = ["count", "players", "updated"];
pub const ONLINE_PLAYER_KEYS: [&str; 8] = ["anonymous", "mjob", "mlvl", "name", "nation", "sjob", "slvl", "zone"];
/// The keys of `/api/public/rankings/{board}`, and of each of its rows.
pub const RANKING_KEYS: [&str; 7] = ["board", "page", "pages", "per_page", "rows", "total", "updated"];
pub const RANK_ROW_KEYS: [&str; 5] = ["job", "name", "nation", "rank", "value"];

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

/// The who-is-online list, at most [`CACHE_SECS`] old, with the time it was read.
/// Only one request refreshes it; the rest wait for that answer.
async fn online_list(state: &Shared) -> ApiResult<(i64, Arc<Vec<PublicPlayer>>)> {
    let fresh = |now: i64| state.public_cache.online.lock().unwrap().as_ref().filter(|(at, _)| now - at < CACHE_SECS).cloned();
    if let Some(hit) = fresh(state.now()) {
        return Ok(hit);
    }
    let gate = state.public_cache.gate("online");
    let _turn = gate.lock().await;
    if let Some(hit) = fresh(state.now()) {
        return Ok(hit);
    }
    let _slot = state.public_cache.slots.acquire().await.map_err(|_| ApiError::not_found())?;
    let list = Arc::new(state.game.public_online().await?);
    let now = state.now();
    *state.public_cache.online.lock().unwrap() = Some((now, list.clone()));
    Ok((now, list))
}

async fn board_rows(state: &Shared, board: Board) -> ApiResult<(i64, Arc<Vec<RankRow>>)> {
    let key = board.key();
    let fresh = |now: i64| state.public_cache.boards.lock().unwrap().get(&key).filter(|(at, _)| now - at < CACHE_SECS).cloned();
    if let Some(hit) = fresh(state.now()) {
        return Ok(hit);
    }
    let gate = state.public_cache.gate(&key);
    let _turn = gate.lock().await;
    if let Some(hit) = fresh(state.now()) {
        return Ok(hit);
    }
    let _slot = state.public_cache.slots.acquire().await.map_err(|_| ApiError::not_found())?;
    let mut rows = state.game.public_ranking(board).await?;
    rows.truncate(RANKING_TOP);
    let rows = Arc::new(rows);
    let now = state.now();
    state.public_cache.boards.lock().unwrap().insert(key, (now, rows.clone()));
    Ok((now, rows))
}

/// The text and news of the home page: the same for everyone, kept 30 seconds, dropped on any edit.
async fn home_content(state: &Shared) -> ApiResult<Value> {
    let gen = state.news_cache.generation();
    if let Some(hit) = state.news_cache.get("home", state.now()) {
        return Ok(hit);
    }
    let stored = state.store.content_all().await?;
    let text = |key: &str| content::effective(key, &stored, &state.cfg).0;
    let (posts, _) = state.store.news_public(1, HOME_POSTS).await?;
    let out = json!({
        "title": text("home_title"),
        "description_html": markdown::render(&text("home_description")),
        "connect_html": markdown::render(&text("connect")),
        "connect_address": content::connect_address(&state.cfg),
        "footer_html": markdown::render(&text("footer")),
        "news": posts.iter().map(summary).collect::<Vec<_>>(),
    });
    remember(&state.news_cache, "home", &out, state.now(), gen);
    Ok(out)
}

async fn home(State(state): State<Shared>, ClientIp(ip): ClientIp) -> ApiResult<Json<Value>> {
    limited(&state, &ip)?;
    // The page still loads while the database is down: it just says so.
    let online = online_list(&state).await.ok().map(|(_, l)| l.len());
    let mut out = home_content(&state).await?;
    let map = out.as_object_mut().expect("an object");
    map.insert("server_name".into(), json!(state.cfg.server_name));
    map.insert("up".into(), json!(online.is_some()));
    map.insert("online".into(), json!(online));
    map.insert("signup_open".into(), json!(state.signup_open().await));
    Ok(Json(out))
}

async fn online(State(state): State<Shared>, ClientIp(ip): ClientIp) -> ApiResult<Json<Value>> {
    limited(&state, &ip)?;
    let (at, list) = online_list(&state).await?;
    Ok(Json(json!({ "updated": at, "count": list.len(), "players": *list })))
}

#[derive(Deserialize)]
struct RankingQuery {
    job: Option<String>,
    skill: Option<String>,
    page: Option<String>,
}

/// `level`, `job`, `craft` or `playtime`. A job is its three-letter column ("war"), a
/// craft its skill id (48-57); both are looked up in fixed lists, never passed on.
fn parse_board(name: &str, q: &RankingQuery) -> ApiResult<Board> {
    match name {
        "level" => Ok(Board::Level),
        "playtime" => Ok(Board::Playtime),
        "job" => {
            let want = q.job.as_deref().unwrap_or("war").to_ascii_lowercase();
            JOBS.iter().position(|j| *j == want).map(Board::Job).ok_or_else(|| ApiError::bad_request("Unknown job."))
        }
        "craft" => {
            let want = match q.skill.as_deref() {
                None => 49,
                Some(s) => s.parse::<i64>().map_err(|_| ApiError::bad_request("Unknown craft."))?,
            };
            CRAFTS.iter().find(|(id, _)| *id == want).map(|(id, _)| Board::Craft(*id)).ok_or_else(|| ApiError::bad_request("Unknown craft."))
        }
        _ => Err(ApiError::not_found()),
    }
}

async fn rankings(State(state): State<Shared>, ClientIp(ip): ClientIp, Path(name): Path<String>, Query(q): Query<RankingQuery>) -> ApiResult<Json<Value>> {
    limited(&state, &ip)?;
    let board = parse_board(&name, &q)?;
    let page = match q.page.as_deref() {
        None => 1,
        Some(p) => p.parse::<usize>().ok().filter(|p| *p >= 1).ok_or_else(|| ApiError::bad_request("Unknown page."))?,
    };
    let (at, rows) = board_rows(&state, board).await?;
    let total = rows.len();
    let pages = total.div_ceil(PER_PAGE).max(1);
    let shown: Vec<&RankRow> = rows.iter().skip((page - 1).saturating_mul(PER_PAGE)).take(PER_PAGE).collect();
    Ok(Json(json!({ "board": name, "updated": at, "total": total, "page": page, "pages": pages, "per_page": PER_PAGE, "rows": shown })))
}

// ---- news and rules ----

fn cached(state: &Shared, key: &str) -> Option<Json<Value>> {
    state.news_cache.get(key, state.now()).map(Json)
}

fn remember(cache: &NewsCache, key: &str, v: &Value, now: i64, gen: u64) {
    cache.put(key, v, now, gen);
}

fn summary(p: &NewsPost) -> Value {
    json!({
        "id": p.id, "title": p.title, "author": p.author,
        "published": p.published, "pinned": p.pinned,
        "excerpt": markdown::excerpt(&p.body, EXCERPT_CHARS),
    })
}

#[derive(Deserialize)]
struct NewsParams {
    page: Option<i64>,
}

async fn news(State(state): State<Shared>, ClientIp(ip): ClientIp, Query(p): Query<NewsParams>) -> ApiResult<Json<Value>> {
    limited(&state, &ip)?;
    let gen = state.news_cache.generation();
    let page = p.page.unwrap_or(1).clamp(1, MAX_PAGE);
    let key = format!("news:{page}");
    if let Some(hit) = cached(&state, &key) {
        return Ok(hit);
    }
    let (posts, total) = state.store.news_public(page, NEWS_PER_PAGE).await?;
    let out = json!({ "items": posts.iter().map(summary).collect::<Vec<_>>(), "page": page, "per": NEWS_PER_PAGE, "total": total });
    // An empty page past the end isn't kept: it would let a crawler fill the cache.
    if !posts.is_empty() || page == 1 {
        remember(&state.news_cache, &key, &out, state.now(), gen);
    }
    Ok(Json(out))
}

async fn post(State(state): State<Shared>, ClientIp(ip): ClientIp, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    limited(&state, &ip)?;
    let gen = state.news_cache.generation();
    let key = format!("post:{id}");
    if let Some(hit) = cached(&state, &key) {
        return Ok(hit);
    }
    let p = state.store.news_published_post(id).await?.ok_or_else(ApiError::not_found)?;
    let out = json!({
        "id": p.id, "title": p.title, "author": p.author, "published": p.published, "pinned": p.pinned,
        "body_html": markdown::render(&p.body),
    });
    remember(&state.news_cache, &key, &out, state.now(), gen);
    Ok(Json(out))
}

/// Only the rules are served by name; the other blocks travel with `/api/public/home`.
async fn content_block(State(state): State<Shared>, ClientIp(ip): ClientIp, Path(key): Path<String>) -> ApiResult<Json<Value>> {
    limited(&state, &ip)?;
    let gen = state.news_cache.generation();
    if key != "rules" {
        return Err(ApiError::not_found());
    }
    let ck = format!("content:{key}");
    if let Some(hit) = cached(&state, &ck) {
        return Ok(hit);
    }
    let stored = state.store.content_all().await?;
    let out = json!({ "html": markdown::render(&content::effective(&key, &stored, &state.cfg).0) });
    remember(&state.news_cache, &ck, &out, state.now(), gen);
    Ok(Json(out))
}
