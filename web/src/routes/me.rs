//! My account: the signed-in player's own account and characters. Every query
//! binds the account id from the session, never from the request.

use crate::auth;
use crate::limits::{Limiter, Target};
use crate::http::*;
use crate::model::*;
use crate::routes::session::{check_password, record};
use crate::state::Shared;
use crate::store::NewAction;
use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn api() -> Router<Shared> {
    Router::new()
        .route("/me/account", get(account))
        .route("/me/characters/{charid}", get(character))
        .route("/me/characters/{charid}/inventory", get(inventory))
        .route("/me/characters/{charid}/bots", get(bots).post(set_bots))
        .route("/me/password", post(change_password))
}

async fn account(State(state): State<Shared>, Player(_, accid): Player) -> ApiResult<Json<Value>> {
    let acct = state.game.my_account(accid).await?.ok_or_else(ApiError::not_found)?;
    let characters = state.game.my_characters(accid).await?;
    Ok(Json(json!({ "account": acct, "characters": characters })))
}

async fn character(State(state): State<Shared>, Player(_, accid): Player, Path(charid): Path<i64>) -> ApiResult<Json<CharDetail>> {
    // A character on another account answers exactly like a missing one.
    state.game.my_character(accid, charid).await?.map(Json).ok_or_else(ApiError::not_found)
}

async fn inventory(State(state): State<Shared>, Player(_, accid): Player, Path(charid): Path<i64>) -> ApiResult<Json<Value>> {
    let items = state.game.my_inventory(accid, charid).await?.ok_or_else(ApiError::not_found)?;
    Ok(Json(json!({ "items": items })))
}

/// The character's AI adventurers switch. `enabled` is whether the server has player bots
/// on at all (its settings); the page shows the switch only then.
async fn bots(State(state): State<Shared>, Player(_, accid): Player, Path(charid): Path<i64>) -> ApiResult<Json<Value>> {
    let hidden = state.game.my_bots_hidden(accid, charid).await?.ok_or_else(ApiError::not_found)?;
    let enabled = crate::lsb_settings::read_async(state.cfg.lsb_settings.clone()).await.bots_enabled;
    Ok(Json(json!({ "enabled": enabled, "hidden": hidden })))
}

#[derive(Deserialize)]
struct BotsBody {
    hidden: bool,
}

/// Writes the same char var as `!bots off` / `!bots on`. The map server reads it when the
/// character logs in, so a change made while in game counts from the next login.
async fn set_bots(
    State(state): State<Shared>,
    Player(_, accid): Player,
    Path(charid): Path<i64>,
    Json(body): Json<BotsBody>,
) -> ApiResult<Json<Value>> {
    state.game.set_my_bots_hidden(accid, charid, body.hidden).await?.ok_or_else(ApiError::not_found)?;
    Ok(Json(json!({ "hidden": body.hidden })))
}

#[derive(Deserialize)]
struct PasswordBody {
    #[serde(default)]
    current: String,
    #[serde(default)]
    new: String,
}

async fn change_password(
    State(state): State<Shared>,
    Player(id, accid): Player,
    ClientIp(ip): ClientIp,
    Json(body): Json<PasswordBody>,
) -> ApiResult<Json<Value>> {
    let login = id.login().unwrap_or("").to_string();
    let now = state.now();
    auth::valid_new_password(&body.new).map_err(ApiError::bad_request)?;
    // Guessing the current password is limited like signing in, and counted before it is checked.
    let keys = Limiter::sign_in_keys(&ip, &Target::Account(accid));
    if !state.limiter.reserve_sign_in(&keys, now) {
        return Err(ApiError::too_many("Too many tries. Wait 15 minutes and try again."));
    }
    let account = state.game.account_by_id(accid).await?.ok_or_else(ApiError::not_found)?;
    if !auth::valid_password(&body.current) || !check_password(&state, Some(&account), &body.current).await? {
        record(&state, NewAction { actor_kind: "account", actor_accid: Some(accid), actor_login: Some(login), actor_role: id.role.as_str(), ip: Some(ip), action: "password_change", result: "refused", ..Default::default() }).await;
        return Err(ApiError::new(axum::http::StatusCode::FORBIDDEN, "Your current password isn't right."));
    }
    state.limiter.sign_in_succeeded(&keys);
    let new = body.new.clone();
    let cost = state.bcrypt_cost;
    let _slot = crate::routes::session::bcrypt_slot(&state).await?;
    let hash = tokio::task::spawn_blocking(move || auth::hash_password(&new, cost))
        .await
        .map_err(|_| ApiError::new(axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong on the server."))?
        .map_err(|_| ApiError::new(axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong on the server."))?;
    state.game.set_password(accid, &hash).await?;
    // Every other sign-in of this account ends; this one stays.
    state.store.end_account_sessions(accid, Some(&id.session.id_hash)).await?;
    record(&state, NewAction { actor_kind: "account", actor_accid: Some(accid), actor_login: Some(login), actor_role: id.role.as_str(), ip: Some(ip), action: "password_change", result: "ok", ..Default::default() }).await;
    Ok(Json(json!({ "ok": true })))
}
