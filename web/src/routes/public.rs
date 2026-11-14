//! What a visitor may see. Phase 1 has only the home page's numbers.

use crate::http::*;
use crate::state::Shared;
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};

pub fn api() -> Router<Shared> {
    Router::new().route("/public/home", get(home))
}

/// The keys of `/api/public/home`. A test pins this list, so a new key can't leak by accident.
pub const HOME_KEYS: [&str; 3] = ["online", "server_name", "up"];

async fn home(State(state): State<Shared>, ClientIp(ip): ClientIp) -> ApiResult<Json<Value>> {
    if !state.limiter.public_get_allowed(&ip, state.now()) {
        return Err(ApiError::too_many("Too many requests. Try again in a minute."));
    }
    // The page still loads while the database is down: it just says so.
    let online = state.game.public_online().await.ok();
    Ok(Json(json!({ "server_name": state.cfg.server_name, "up": online.is_some(), "online": online })))
}
