//! The admin panel's API: Moderator and Owner only.

use crate::auth::Role;
use crate::http::*;
use crate::model::*;
use crate::state::Shared;
use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub fn api() -> Router<Shared> {
    Router::new().route("/admin/dashboard", get(dashboard)).route("/admin/log", get(log))
}

#[derive(Serialize)]
struct OnlineOut {
    #[serde(flatten)]
    row: OnlineRow,
    /// Owners only.
    #[serde(skip_serializing_if = "Option::is_none")]
    address: Option<String>,
}

/// The busiest zones among those online, most first (ties by name), at most five.
pub fn busiest_zones(online: &[OnlineRow]) -> Vec<Value> {
    let mut by_zone: BTreeMap<String, i64> = BTreeMap::new();
    for r in online {
        *by_zone.entry(r.zone.clone().unwrap_or_else(|| "Unknown".into())).or_default() += 1;
    }
    let mut zones: Vec<(String, i64)> = by_zone.into_iter().collect();
    zones.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    zones.into_iter().take(5).map(|(zone, count)| json!({ "zone": zone, "count": count })).collect()
}

async fn dashboard(State(state): State<Shared>, Staff(id): Staff) -> ApiResult<Json<Value>> {
    let counts = state.game.counts().await?;
    let online = state.game.online().await?;
    let zones = busiest_zones(&online);
    let services = state.probe.services(true).await;
    let actions = state.store.actions(None, 8, true).await?;
    let owner = id.role == Role::Owner;
    let online: Vec<OnlineOut> = online
        .into_iter()
        .map(|row| {
            let address = owner.then(|| decode_ip(row.client_addr).to_string());
            OnlineOut { row, address }
        })
        .collect();
    Ok(Json(json!({
        "counts": counts,
        "online": online,
        "zones": zones,
        "services": services,
        "actions": actions,
    })))
}

#[derive(Deserialize)]
struct LogQuery {
    before: Option<i64>,
    per: Option<i64>,
}

async fn log(State(state): State<Shared>, Staff(_): Staff, Query(q): Query<LogQuery>) -> ApiResult<Json<Value>> {
    let per = q.per.unwrap_or(50).clamp(1, 200);
    let items = state.store.actions(q.before, per, false).await?;
    Ok(Json(json!({ "items": items, "total": state.store.action_count().await? })))
}
