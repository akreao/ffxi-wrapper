//! The admin panel's API: Moderator and Owner only.
//!
//! Every change to an account is allowed by website role (never on an account at your own
//! role or above), checked under `AppState::role_gate` until the change is written, is
//! refused or done in the action log, and needs a reason where it shuts someone out or
//! changes who has power. In-game GM levels give no website powers.

use crate::auth::{self, Role};
use crate::http::*;
use crate::model::*;
use crate::roles;
use crate::routes::session::record;
use crate::state::Shared;
use crate::store::{LogQuery, NewAction};
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub fn api() -> Router<Shared> {
    Router::new()
        .route("/admin/dashboard", get(dashboard))
        .route("/admin/log", get(log))
        .route("/admin/accounts", get(accounts))
        .route("/admin/accounts/{id}", get(account))
        .route("/admin/accounts/{id}/ban", post(ban))
        .route("/admin/accounts/{id}/unban", post(unban))
        .route("/admin/accounts/{id}/password", post(password))
        .route("/admin/accounts/{id}/totp-reset", post(totp_reset))
        .route("/admin/accounts/{id}/role", post(site_role))
        .route("/admin/characters", get(characters))
        .route("/admin/characters/{charid}", get(character))
        .route("/admin/characters/{charid}/inventory", get(inventory))
        .route("/admin/characters/{charid}/gm-level", post(gm_level))
        .route("/admin/bans", get(bans))
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
    let owner = id.role == Role::Owner;
    let actions = state.store.actions(None, 8, true, owner).await?;
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

// ---- lists ----

const PER_DEFAULT: i64 = 25;

/// `page` from 1, `per` from 10 to 100.
fn paging(page: Option<i64>, per: Option<i64>) -> (i64, i64) {
    (page.unwrap_or(1).clamp(1, 1_000_000), per.unwrap_or(PER_DEFAULT).clamp(10, 100))
}

/// A search box's text: trimmed, at most 64 characters.
fn search(q: &Option<String>) -> String {
    q.as_deref().unwrap_or("").trim().chars().take(64).collect()
}

#[derive(Deserialize)]
struct LogParams {
    q: Option<String>,
    actor: Option<String>,
    action: Option<String>,
    /// Only the last this many days.
    days: Option<i64>,
    page: Option<i64>,
    per: Option<i64>,
}

async fn log(State(state): State<Shared>, Staff(id): Staff, Query(p): Query<LogParams>) -> ApiResult<Json<Value>> {
    let (page, per) = paging(p.page, p.per);
    let nonempty = |s: Option<String>| s.filter(|s| !s.is_empty());
    let q = LogQuery {
        q: search(&p.q),
        actor: nonempty(p.actor),
        action: nonempty(p.action),
        since: p.days.filter(|d| *d > 0).map(|d| state.now() - d.min(36_500) * 86_400),
        owner: id.role == Role::Owner,
        page,
        per,
    };
    let (items, total) = state.store.log_page(&q).await?;
    let (actors, actions) = state.store.log_facets(q.owner).await?;
    Ok(Json(json!({
        "items": items,
        "total": total, "page": page, "per": per,
        "actors": actors, "actions": actions,
    })))
}

#[derive(Deserialize)]
struct AccountParams {
    q: Option<String>,
    /// all | online | banned
    status: Option<String>,
    /// login | id | created | last_login
    sort: Option<String>,
    /// asc | desc
    dir: Option<String>,
    page: Option<i64>,
    per: Option<i64>,
}

async fn accounts(State(state): State<Shared>, Staff(_): Staff, Query(p): Query<AccountParams>) -> ApiResult<Json<Value>> {
    let (page, per) = paging(p.page, p.per);
    let query = AccountQuery {
        q: search(&p.q),
        filter: match p.status.as_deref() {
            Some("online") => AccountFilter::Online,
            Some("banned") => AccountFilter::Banned,
            _ => AccountFilter::All,
        },
        sort: match p.sort.as_deref() {
            Some("id") => AccountSort::Id,
            Some("created") => AccountSort::Created,
            Some("last_login") => AccountSort::LastLogin,
            _ => AccountSort::Login,
        },
        desc: p.dir.as_deref() == Some("desc"),
        page,
        per,
    };
    let list = state.game.admin_accounts(&query).await?;
    let counts = state.game.counts().await?;
    roles::ensure_seeded(&state).await?;
    let stored = state.store.site_roles().await?;
    let items: Vec<Value> = list
        .items
        .into_iter()
        .map(|a| {
            let row = stored.iter().find(|r| r.accid == a.id).map(|r| (r.role.as_str(), r.login.as_str()));
            let role = auth::stored_role(row, &a.login);
            let mut v = json!(a);
            v["role"] = json!(role.as_str());
            v
        })
        .collect();
    Ok(Json(json!({ "items": items, "total": list.total, "page": page, "per": per, "counts": counts })))
}

#[derive(Deserialize)]
struct CharParams {
    q: Option<String>,
    online: Option<String>,
    /// name | account | playtime | last_logout
    sort: Option<String>,
    dir: Option<String>,
    page: Option<i64>,
    per: Option<i64>,
}

async fn characters(State(state): State<Shared>, Staff(_): Staff, Query(p): Query<CharParams>) -> ApiResult<Json<Value>> {
    let (page, per) = paging(p.page, p.per);
    let query = CharQuery {
        q: search(&p.q),
        online: matches!(p.online.as_deref(), Some("1" | "true")),
        sort: match p.sort.as_deref() {
            Some("account") => CharSort::Account,
            Some("playtime") => CharSort::Playtime,
            Some("last_logout") => CharSort::LastLogout,
            _ => CharSort::Name,
        },
        desc: p.dir.as_deref() == Some("desc"),
        page,
        per,
    };
    let list = state.game.admin_characters(&query).await?;
    Ok(Json(json!({ "items": list.items, "total": list.total, "page": page, "per": per })))
}

#[derive(Deserialize)]
struct BanParams {
    q: Option<String>,
    page: Option<i64>,
    per: Option<i64>,
}

async fn bans(State(state): State<Shared>, Staff(_): Staff, Query(p): Query<BanParams>) -> ApiResult<Json<Value>> {
    let (page, per) = paging(p.page, p.per);
    let list = state.game.admin_bans(&search(&p.q), page, per).await?;
    Ok(Json(json!({ "items": list.items, "total": list.total, "page": page, "per": per, "now": state.now() })))
}

// ---- one account, one character ----

async fn account(State(state): State<Shared>, Staff(id): Staff, Path(accid): Path<i64>) -> ApiResult<Json<Value>> {
    let acct = state.game.admin_account(accid).await?.ok_or_else(ApiError::not_found)?;
    let mut targets = vec![("account", accid)];
    targets.extend(acct.characters.iter().map(|c| ("character", c.summary.charid)));
    let history = state.store.history(&targets, 50).await?;
    let role = roles::role_of(&state, accid, &acct.login).await?;
    let can_act = auth::can_act_on(id.actor(), role);
    let own = id.accid() == Some(accid);
    Ok(Json(json!({ "account": acct, "role": role.as_str(), "history": history, "can_act": can_act, "own": own })))
}

async fn character(State(state): State<Shared>, Staff(id): Staff, Path(charid): Path<i64>) -> ApiResult<Json<Value>> {
    let c = state.game.admin_character(charid).await?.ok_or_else(ApiError::not_found)?;
    let history = state.store.history(&[("character", charid), ("account", c.accid)], 50).await?;
    let role = roles::role_of(&state, c.accid, &c.login).await?;
    let can_act = auth::can_act_on(id.actor(), role);
    let own = id.accid() == Some(c.accid);
    Ok(Json(json!({ "character": c, "role": role.as_str(), "history": history, "can_act": can_act, "own": own })))
}

async fn inventory(State(state): State<Shared>, Staff(_): Staff, Path(charid): Path<i64>) -> ApiResult<Json<Value>> {
    let items = state.game.admin_inventory(charid).await?.ok_or_else(ApiError::not_found)?;
    Ok(Json(json!({ "items": items })))
}

// ---- changes ----

/// The base of an action-log row for something `id` did to an account or character.
fn act(id: &Identity, ip: &str, action: &'static str, kind: &'static str, target: i64, name: &str) -> NewAction {
    NewAction {
        actor_kind: if id.owner_link() { "owner" } else { "account" },
        actor_accid: id.accid(),
        actor_login: id.login().map(String::from),
        actor_role: id.role.as_str(),
        ip: Some(ip.to_string()),
        action,
        target_kind: Some(kind),
        target_id: Some(target),
        target_name: Some(name.to_string()),
        result: "ok",
        ..Default::default()
    }
}

fn clean_reason(reason: Option<&str>, required: bool) -> ApiResult<Option<String>> {
    let r = reason.unwrap_or("").trim();
    if r.is_empty() {
        return if required { Err(ApiError::bad_request("Say why: a reason is needed, and it goes in the admin log.")) } else { Ok(None) };
    }
    if r.chars().count() > 500 {
        return Err(ApiError::bad_request("Keep the reason to 500 characters."));
    }
    if r.chars().any(|c| c.is_control() && c != '\n') {
        return Err(ApiError::bad_request("The reason can't contain control characters."));
    }
    Ok(Some(r.to_string()))
}

fn below_you() -> ApiError {
    ApiError::new(StatusCode::FORBIDDEN, "You can only change accounts whose website role is below yours.")
}

fn owners_only(id: &Identity) -> ApiResult<()> {
    if id.role == Role::Owner { Ok(()) } else { Err(ApiError::forbidden()) }
}

/// Settles what `Game` said: logs a refusal and turns the outcome into an error.
async fn settle(state: &Shared, outcome: Acted, row: NewAction, none: &str) -> ApiResult<()> {
    match outcome {
        Acted::Done => {
            record(state, row).await;
            Ok(())
        }
        Acted::NotFound => Err(ApiError::not_found()),
        Acted::NoChange => Err(ApiError::new(StatusCode::CONFLICT, none)),
    }
}

#[derive(Deserialize)]
struct BanBody {
    reason: Option<String>,
    /// Unix seconds; absent means until someone lifts it.
    until: Option<i64>,
}

async fn ban(
    State(state): State<Shared>,
    Staff(id): Staff,
    ClientIp(ip): ClientIp,
    Path(accid): Path<i64>,
    Json(body): Json<BanBody>,
) -> ApiResult<Json<Value>> {
    let reason = clean_reason(body.reason.as_deref(), true)?.unwrap_or_default();
    let now = state.now();
    if let Some(u) = body.until {
        // At least a minute ahead, at most ten years.
        if u < now + 60 || u > now + 10 * 365 * 86_400 {
            return Err(ApiError::bad_request("A ban has to end between a minute and ten years from now, or have no end."));
        }
    }
    let acct = state.game.admin_account(accid).await?.ok_or_else(ApiError::not_found)?;
    let mut row = act(&id, &ip, "ban", "account", accid, &acct.login);
    row.reason = Some(reason.clone());
    row.detail = Some(json!({ "until": body.until }).to_string());
    let _gate = state.role_gate.lock().await;
    if !auth::can_act_on(id.actor(), roles::role_of(&state, accid, &acct.login).await?) {
        row.result = "refused";
        record(&state, row).await;
        return Err(below_you());
    }
    let outcome = state.game.ban_account(accid, body.until, &reason).await?;
    settle(&state, outcome, row, "").await?;
    // A banned account's web sessions end at its next request anyway; end them now.
    state.store.end_account_sessions(accid, None).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct ReasonBody {
    reason: Option<String>,
}

async fn unban(
    State(state): State<Shared>,
    Staff(id): Staff,
    ClientIp(ip): ClientIp,
    Path(accid): Path<i64>,
    Json(body): Json<ReasonBody>,
) -> ApiResult<Json<Value>> {
    let reason = clean_reason(body.reason.as_deref(), true)?;
    let acct = state.game.admin_account(accid).await?.ok_or_else(ApiError::not_found)?;
    let mut row = act(&id, &ip, "unban", "account", accid, &acct.login);
    row.reason = reason;
    let _gate = state.role_gate.lock().await;
    if !auth::can_act_on(id.actor(), roles::role_of(&state, accid, &acct.login).await?) {
        row.result = "refused";
        record(&state, row).await;
        return Err(below_you());
    }
    let outcome = state.game.unban_account(accid).await?;
    settle(&state, outcome, row, "That account isn't banned.").await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct PasswordBody {
    /// Absent: the site makes one up and shows it once.
    new_password: Option<String>,
    reason: Option<String>,
}

/// 16 characters in four groups, without lookalikes: `kf7m-2xqp-9hdc-wr4t`.
fn random_password() -> String {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789"; // 31 characters
    let mut out = String::with_capacity(19);
    let mut n = 0;
    while n < 16 {
        let mut b = [0u8; 1];
        getrandom::fill(&mut b).expect("the system random source works");
        // 248 = 31 * 8: no bias.
        if b[0] < 248 {
            if n > 0 && n % 4 == 0 {
                out.push('-');
            }
            out.push(ALPHABET[(b[0] % 31) as usize] as char);
            n += 1;
        }
    }
    out
}

async fn password(
    State(state): State<Shared>,
    Staff(id): Staff,
    ClientIp(ip): ClientIp,
    Path(accid): Path<i64>,
    Json(body): Json<PasswordBody>,
) -> ApiResult<(StatusCode, [(header::HeaderName, &'static str); 1], Json<Value>)> {
    owners_only(&id)?;
    let reason = clean_reason(body.reason.as_deref(), false)?;
    let typed = body.new_password.filter(|p| !p.is_empty());
    if let Some(p) = &typed {
        auth::valid_new_password(p).map_err(ApiError::bad_request)?;
    }
    let acct = state.game.admin_account(accid).await?.ok_or_else(ApiError::not_found)?;
    let mut row = act(&id, &ip, "password_reset", "account", accid, &acct.login);
    row.reason = reason;
    let _gate = state.role_gate.lock().await;
    if !auth::can_act_on(id.actor(), roles::role_of(&state, accid, &acct.login).await?) {
        row.result = "refused";
        record(&state, row).await;
        return Err(below_you());
    }
    let shown = typed.is_none().then(random_password);
    let new = typed.or_else(|| shown.clone()).expect("one of the two");
    let cost = state.bcrypt_cost;
    let hash = tokio::task::spawn_blocking(move || auth::hash_password(&new, cost))
        .await
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong on the server."))?
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong on the server."))?;
    let outcome = state.game.reset_password(accid, &hash).await?;
    settle(&state, outcome, row, "").await?;
    // The old password's web sessions end with it.
    state.store.end_account_sessions(accid, None).await?;
    Ok((StatusCode::OK, [(header::CACHE_CONTROL, "no-store")], Json(json!({ "ok": true, "password": shown }))))
}

async fn totp_reset(
    State(state): State<Shared>,
    Staff(id): Staff,
    ClientIp(ip): ClientIp,
    Path(accid): Path<i64>,
    Json(body): Json<ReasonBody>,
) -> ApiResult<Json<Value>> {
    owners_only(&id)?;
    let reason = clean_reason(body.reason.as_deref(), true)?;
    let acct = state.game.admin_account(accid).await?.ok_or_else(ApiError::not_found)?;
    let mut row = act(&id, &ip, "totp_reset", "account", accid, &acct.login);
    row.reason = reason;
    let _gate = state.role_gate.lock().await;
    if !auth::can_act_on(id.actor(), roles::role_of(&state, accid, &acct.login).await?) {
        row.result = "refused";
        record(&state, row).await;
        return Err(below_you());
    }
    if !acct.has_totp {
        return Err(ApiError::new(StatusCode::CONFLICT, "That account has no two-step sign-in."));
    }
    let outcome = state.game.reset_totp(accid).await?;
    settle(&state, outcome, row, "").await?;
    state.store.end_account_sessions(accid, None).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct GmBody {
    level: i64,
    reason: Option<String>,
}

async fn gm_level(
    State(state): State<Shared>,
    Staff(id): Staff,
    ClientIp(ip): ClientIp,
    Path(charid): Path<i64>,
    Json(body): Json<GmBody>,
) -> ApiResult<Json<Value>> {
    owners_only(&id)?;
    if !(0..=5).contains(&body.level) {
        return Err(ApiError::bad_request("GM levels run from 0 to 5."));
    }
    let reason = clean_reason(body.reason.as_deref(), true)?;
    let c = state.game.admin_character(charid).await?.ok_or_else(ApiError::not_found)?;
    let mut row = act(&id, &ip, "gm_level", "character", charid, &c.detail.summary.name);
    row.reason = reason;
    row.detail = Some(json!({ "before": c.gm, "after": body.level }).to_string());
    // GM levels are in-game only, but still an Owner's power: one Owner can't change the
    // characters of another Owner's account (the check is the account's role, not that
    // one character's level). Their own account's characters are theirs to change.
    let _gate = state.role_gate.lock().await;
    let own = id.accid() == Some(c.accid);
    if !own && !auth::can_act_on(id.actor(), roles::role_of(&state, c.accid, &c.login).await?) {
        row.result = "refused";
        record(&state, row).await;
        return Err(below_you());
    }
    if c.detail.summary.online {
        return Err(ApiError::new(StatusCode::CONFLICT, "That character is in game: the page is read-only until they log out."));
    }
    if c.gm == body.level {
        return Err(ApiError::bad_request("That is already their level."));
    }
    let outcome = state.game.set_gm_level(charid, body.level).await?;
    settle(&state, outcome, row, "").await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct RoleBody {
    role: String,
    reason: Option<String>,
}

/// Sets an account's website role. Owners only; never their own account, and never an
/// account at Owner already (only the owner link can take Owner away).
async fn site_role(
    State(state): State<Shared>,
    Staff(id): Staff,
    ClientIp(ip): ClientIp,
    Path(accid): Path<i64>,
    Json(body): Json<RoleBody>,
) -> ApiResult<Json<Value>> {
    owners_only(&id)?;
    let new = Role::assignable(&body.role).ok_or_else(|| ApiError::bad_request("Pick player, moderator or owner."))?;
    let reason = clean_reason(body.reason.as_deref(), true)?;
    let acct = state.game.admin_account(accid).await?.ok_or_else(ApiError::not_found)?;
    let _gate = state.role_gate.lock().await;
    let before = roles::role_of(&state, accid, &acct.login).await?;
    let mut row = act(&id, &ip, "site_role", "account", accid, &acct.login);
    row.reason = reason;
    row.detail = Some(json!({ "before": before.as_str(), "after": new.as_str() }).to_string());
    if id.accid() == Some(accid) {
        row.result = "refused";
        record(&state, row).await;
        return Err(ApiError::new(StatusCode::FORBIDDEN, "You can't change your own website role."));
    }
    if !auth::can_act_on(id.actor(), before) {
        row.result = "refused";
        record(&state, row).await;
        return Err(below_you());
    }
    if before == new {
        return Err(ApiError::bad_request("That is already their role."));
    }
    let who = id.login().map(String::from).unwrap_or_else(|| "owner".into());
    state.store.set_site_role(accid, &acct.login, new.as_str(), &who, state.now()).await?;
    record(&state, row).await;
    // Their web sessions start over with the new role.
    state.store.end_account_sessions(accid, None).await?;
    Ok(Json(json!({ "ok": true, "role": new.as_str() })))
}

// ---- timed bans ----

/// Lifts every ban whose end has passed and logs each as the site's own `unban`.
/// LSB never does this, so the service runs it every minute.
pub async fn lift_expired_bans(state: &Shared) -> usize {
    match state.game.lift_expired_bans(state.now()).await {
        Ok(lifted) => {
            for (accid, login) in &lifted {
                record(
                    state,
                    NewAction {
                        actor_kind: "system",
                        actor_role: "system",
                        action: "unban",
                        target_kind: Some("account"),
                        target_id: Some(*accid),
                        target_name: Some(login.clone()),
                        reason: Some("The ban ran out.".into()),
                        result: "ok",
                        ..Default::default()
                    },
                )
                .await;
            }
            lifted.len()
        }
        Err(e) => {
            tracing::warn!("couldn't lift expired bans: {e:?}");
            0
        }
    }
}
