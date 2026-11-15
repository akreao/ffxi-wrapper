//! LSB's audit logs, read-only, for staff: chat, GM commands, trades, bazaar sales and
//! delivery box sends. Tells are the owner's alone. LSB writes these tables only for what
//! the desktop app's Game settings switch on, and never deletes from them.

use crate::auth::Role;
use crate::http::*;
use crate::model::*;
use crate::routes::items::fold_search;
use crate::routes::session::record;
use crate::state::Shared;
use crate::store::NewAction;
use axum::extract::{Query, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

pub fn api() -> Router<Shared> {
    Router::new().route("/admin/audit", get(audit))
}

/// The oldest rows the site shows. Older ones stay in the database.
pub const AGE_LIMIT_DAYS: i64 = 90;
/// The last page a search can reach: past it, narrow the search.
pub const PAGE_MAX: i64 = 40;
/// Longest text search.
const Q_MAX: usize = 64;
/// LSB's character names are at most 15 characters.
const NAME_MAX: usize = 15;
/// Searches that may run at once, site-wide; the database pool has 8 connections.
pub const RUNNING_MAX: usize = 2;
/// How long a search waits for one of those places before giving up.
const WAIT_SECS: u64 = 3;

#[derive(Deserialize)]
struct AuditParams {
    /// chat | gm | trade | bazaar | dbox
    kind: Option<String>,
    /// A chat channel (see `CHAT_CHANNELS`); empty for any.
    channel: Option<String>,
    /// A character name.
    who: Option<String>,
    /// An account id: rows for any of its characters.
    account: Option<i64>,
    q: Option<String>,
    /// Unix seconds; `from` included, `to` not.
    from: Option<i64>,
    to: Option<i64>,
    page: Option<i64>,
    per: Option<i64>,
}

async fn audit(
    State(state): State<Shared>,
    Staff(id): Staff,
    ClientIp(ip): ClientIp,
    Query(p): Query<AuditParams>,
) -> ApiResult<impl IntoResponse> {
    if !state.limiter.audit_read_allowed(&id.rate_key(), state.now()) {
        return Err(ApiError::too_many("Slow down: too many log searches in a minute."));
    }
    let bad = || ApiError::bad_request("That filter isn't one of the choices.");
    let kind = match p.kind.as_deref().unwrap_or("chat") {
        "chat" => AuditKind::Chat,
        "gm" => AuditKind::Gm,
        "trade" => AuditKind::Trade,
        "bazaar" => AuditKind::Bazaar,
        "dbox" => AuditKind::Dbox,
        _ => return Err(bad()),
    };
    let owner = id.role == Role::Owner;
    let channel = match p.channel.as_deref().unwrap_or("") {
        "" => String::new(),
        _ if kind != AuditKind::Chat => return Err(bad()),
        "TELL" if !owner => return Err(ApiError::new(axum::http::StatusCode::FORBIDDEN, "Only the owner can read tells.")),
        c => CHAT_CHANNELS.iter().find(|x| **x == c).ok_or_else(bad)?.to_string(),
    };

    let who: String = p.who.as_deref().unwrap_or("").trim().chars().filter(|c| !c.is_control()).take(NAME_MAX).collect();
    let mut names = Vec::new();
    let mut nobody = false;
    if let Some(accid) = p.account {
        names = state.game.account_char_names(accid).await?;
        if !who.is_empty() {
            names.retain(|n| n.eq_ignore_ascii_case(&who));
        }
        nobody = names.is_empty();
    } else if !who.is_empty() {
        names.push(who);
    }

    let raw = p.q.as_deref().unwrap_or("");
    let q = match kind {
        AuditKind::Chat | AuditKind::Gm => raw.trim().chars().filter(|c| !c.is_control()).take(Q_MAX).collect(),
        _ => fold_search(Some(raw)),
    };

    let now = state.now();
    let oldest = now - AGE_LIMIT_DAYS * 86_400;
    let from = p.from.unwrap_or(now - 7 * 86_400).max(oldest);
    let to = p.to.unwrap_or(now + 1).min(now + 86_400);
    let page = p.page.unwrap_or(1).clamp(1, PAGE_MAX);
    let per = p.per.unwrap_or(25).clamp(10, 100);

    let query = AuditQuery { kind, channel, tells: owner, names, q, from, to, page, per };
    // Each search is written to the admin log (what was asked, never what came back); a new
    // search starts on page 1, so turning pages isn't logged again.
    if page == 1 {
        record(&state, logged(&id, &ip, &query, p.account)).await;
    }
    let found = if nobody || from >= to {
        AuditPage::default()
    } else {
        let slot = tokio::time::timeout(std::time::Duration::from_secs(WAIT_SECS), state.audit_slots.clone().acquire_owned())
            .await
            .ok()
            .and_then(Result::ok)
            .ok_or_else(|| ApiError::too_many("Other log searches are still running. Try again in a moment."))?;
        // Run to the end even if the browser goes away, holding the place until then: a
        // dropped request must not free it while its query still runs in the database.
        let game = state.game.clone();
        tokio::spawn(async move {
            let _slot = slot;
            game.audit(&query).await
        })
        .await
        .map_err(|e| ApiError::new(axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("The search failed: {e}")))??
    };
    let channels: Vec<&str> = CHAT_CHANNELS.iter().copied().filter(|c| owner || *c != "TELL").collect();
    Ok((
        [(header::CACHE_CONTROL, "private, no-store")],
        Json(json!({
            "items": found.items,
            "more": found.more && page < PAGE_MAX,
            "page": page, "per": per, "page_max": PAGE_MAX,
            "oldest": oldest,
            "channels": channels,
        })),
    ))
}

/// The admin log row for a search: which log and which filters.
fn logged(id: &Identity, ip: &str, q: &AuditQuery, account: Option<i64>) -> NewAction {
    let log = match q.kind {
        AuditKind::Chat => "Chat",
        AuditKind::Gm => "GM commands",
        AuditKind::Trade => "Trades",
        AuditKind::Bazaar => "Bazaar sales",
        AuditKind::Dbox => "Delivery box",
    };
    let mut filters = Vec::new();
    if !q.channel.is_empty() {
        filters.push(format!("channel {}", q.channel));
    } else if q.kind == AuditKind::Chat && q.tells {
        filters.push("any channel, tells included".to_owned());
    }
    if let Some(a) = account {
        filters.push(format!("account {a}"));
    }
    if account.is_none() && !q.names.is_empty() {
        filters.push(format!("character {}", q.names.join(", ")));
    }
    if !q.q.is_empty() {
        filters.push(format!("text \"{}\"", q.q));
    }
    NewAction {
        actor_kind: if id.owner_link() { "owner" } else { "account" },
        actor_accid: id.accid(),
        actor_login: id.login().map(String::from),
        actor_role: id.role.as_str(),
        ip: Some(ip.to_owned()),
        action: "audit_read",
        target_name: Some(log.to_owned()),
        reason: (!filters.is_empty()).then(|| {
            let all = filters.join("; ");
            let mut c = all.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        }),
        detail: Some(json!({ "from": q.from, "to": q.to }).to_string()),
        result: "ok",
        ..Default::default()
    }
}
