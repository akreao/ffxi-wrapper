//! What a visitor may see: the home page's numbers, who is online, and the
//! rankings. Only character names, nations, jobs and levels leave here; the
//! rules of who is left out live in the `Game` queries and are tested.

use crate::http::*;
use crate::model::*;
use crate::state::Shared;
use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub fn api() -> Router<Shared> {
    Router::new()
        .route("/public/home", get(home))
        .route("/public/online", get(online))
        .route("/public/rankings/{board}", get(rankings))
}

/// How long a public answer is reused before the database is asked again.
pub const CACHE_SECS: i64 = 30;
/// Rows per page of a ranking.
pub const PER_PAGE: usize = 25;

/// The keys of `/api/public/home`. A test pins this list, so a new key can't leak by accident.
pub const HOME_KEYS: [&str; 3] = ["online", "server_name", "up"];
/// The keys of `/api/public/online`, and of each of its players.
pub const ONLINE_KEYS: [&str; 3] = ["count", "players", "updated"];
pub const ONLINE_PLAYER_KEYS: [&str; 8] = ["anonymous", "mjob", "mlvl", "name", "nation", "sjob", "slvl", "zone"];
/// The keys of `/api/public/rankings/{board}`, and of each of its rows.
pub const RANKING_KEYS: [&str; 7] = ["board", "page", "pages", "per_page", "rows", "total", "updated"];
pub const RANK_ROW_KEYS: [&str; 5] = ["job", "name", "nation", "rank", "value"];

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

async fn home(State(state): State<Shared>, ClientIp(ip): ClientIp) -> ApiResult<Json<Value>> {
    limited(&state, &ip)?;
    // The page still loads while the database is down: it just says so.
    let online = online_list(&state).await.ok().map(|(_, l)| l.len());
    Ok(Json(json!({ "server_name": state.cfg.server_name, "up": online.is_some(), "online": online })))
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
