//! My account: the signed-in player's own account and characters. Every query
//! binds the account id from the session, never from the request.

use crate::auth::{self, HashCheck};
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
    if !state.limiter.sign_in_allowed(&ip, &login, now) {
        return Err(ApiError::too_many("Too many tries. Wait 15 minutes and try again."));
    }
    auth::valid_new_password(&body.new).map_err(ApiError::bad_request)?;
    let account = state.game.account_by_id(accid).await?.ok_or_else(ApiError::not_found)?;
    if !auth::valid_password(&body.current) || check_password(&state, Some(&account), &body.current).await != HashCheck::Match {
        state.limiter.sign_in_failed(&ip, &login, now);
        record(&state, NewAction { actor_kind: "account", actor_accid: Some(accid), actor_login: Some(login), actor_role: id.role.as_str(), ip: Some(ip), action: "password_change", result: "refused", ..Default::default() }).await;
        return Err(ApiError::new(axum::http::StatusCode::FORBIDDEN, "Your current password isn't right."));
    }
    let new = body.new.clone();
    let cost = state.bcrypt_cost;
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
