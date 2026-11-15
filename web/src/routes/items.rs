//! The item database: static game data for any signed-in account (Player and up, and the
//! owner link). Names, stack sizes, rare/ex, equipment level, jobs and slots, weapon damage
//! and delay, and stats. Read-only, and nothing else about an item: not who owns one, not
//! drops, the auction house or its internal flags. Descriptions live in the client's files.

use crate::http::*;
use crate::model::*;
use crate::state::Shared;
use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use axum::http::header;
use axum::response::IntoResponse;
use std::collections::HashMap;
use std::sync::LazyLock;

pub fn api() -> Router<Shared> {
    Router::new().route("/items", get(list)).route("/items/{id}", get(detail))
}

/// The longest name search: item names are at most 31 characters (the game's limit) plus room.
pub const Q_MAX: usize = 40;
const PER_DEFAULT: i64 = 25;
const PER_MAX: i64 = 100;
/// Past this page the list stops: 100 per page covers every item many times over.
const PAGE_MAX: i64 = 1000;

const FLAG_EX: i64 = 0x4000;
const FLAG_RARE: i64 = 0x8000;

const JOBS: [&str; 22] = [
    "war", "mnk", "whm", "blm", "rdm", "thf", "pld", "drk", "bst", "brd", "rng", "sam", "nin", "drg", "smn", "blu", "cor", "pup", "dnc", "sch", "geo", "run",
];
/// Equipment slots in bit order (LSB's SLOTTYPE): main is bit 0.
const SLOTS: [&str; 16] = ["main", "sub", "ranged", "ammo", "head", "body", "hands", "legs", "feet", "neck", "waist", "ear1", "ear2", "ring1", "ring2", "back"];

/// LSB's modifier names by id, from `data/enums/mod.yaml` at the pinned LSB commit.
static MOD_NAMES: LazyLock<HashMap<i64, &'static str>> = LazyLock::new(|| {
    include_str!("../item_mods.txt")
        .lines()
        .filter_map(|l| l.split_once(' ').and_then(|(id, name)| Some((id.parse().ok()?, name))))
        .collect()
});

#[derive(Deserialize)]
pub(crate) struct ItemParams {
    q: Option<String>,
    /// Auction house category, 1-65.
    category: Option<i64>,
    /// Job abbreviation, `war` to `run`.
    job: Option<String>,
    /// One of [`SLOTS`]; `ear` and `ring` match either.
    slot: Option<String>,
    min: Option<i64>,
    max: Option<i64>,
    page: Option<i64>,
    per: Option<i64>,
}

#[derive(Serialize)]
pub(crate) struct ItemOut {
    id: i64,
    name: String,
    category: Option<i64>,
    stack: i64,
    rare: bool,
    ex: bool,
    level: Option<i64>,
    /// Bit (job - 1) for each job that can wear it; 0 when it isn't equipment.
    jobs: i64,
    /// Bit per [`SLOTS`] entry.
    slots: i64,
}

pub(crate) fn item_out(i: &ItemRaw) -> ItemOut {
    ItemOut {
        id: i.id,
        name: display_name(&i.name),
        category: Some(i.category).filter(|c| (1..=254).contains(c)),
        stack: i.stack,
        rare: i.flags & FLAG_RARE != 0,
        ex: i.flags & FLAG_EX != 0,
        level: i.level,
        jobs: i.jobs & 0x3f_ffff,
        slots: i.slots & 0xffff,
    }
}

/// `pile_of_chocobo_bedding` -> `Pile of Chocobo Bedding`.
pub fn display_name(raw: &str) -> String {
    raw.split('_')
        .filter(|w| !w.is_empty())
        .enumerate()
        .map(|(n, w)| {
            if n > 0 && matches!(w, "of" | "the" | "and" | "a" | "an" | "in" | "on" | "to" | "for" | "with" | "from") {
                return w.to_string();
            }
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What was typed, cut to [`Q_MAX`] characters and spelled like `item_basic.name`.
pub fn fold_search(q: Option<&str>) -> String {
    q.unwrap_or("").trim().chars().filter(|c| !c.is_control()).take(Q_MAX).collect::<String>().to_lowercase().replace(' ', "_")
}

/// Static data, but only for signed-in viewers: no shared cache may keep it.
pub(crate) fn private<T: IntoResponse>(body: T) -> impl IntoResponse {
    ([(header::CACHE_CONTROL, "private, no-store")], body)
}

/// One read counted against the viewer's account (or their owner-link session).
pub(crate) fn throttle(state: &Shared, id: &Identity) -> ApiResult<()> {
    if state.limiter.item_read_allowed(&id.rate_key(), state.now()) {
        Ok(())
    } else {
        Err(ApiError::too_many("Slow down: too many item searches in a minute."))
    }
}

async fn list(State(state): State<Shared>, Signed(id): Signed, Query(p): Query<ItemParams>) -> ApiResult<impl IntoResponse> {
    throttle(&state, &id)?;
    let query = item_query(&state, p).await?;
    let found = state.game.items(&query).await?;
    let items: Vec<ItemOut> = found.items.iter().map(item_out).collect();
    Ok(private(Json(json!({ "items": items, "total": found.total, "page": query.page, "per": query.per }))))
}

/// The search filters checked and bounded, with the server's level cap: shared with the
/// auction house list, so both leave out the same items.
pub(crate) async fn item_query(state: &Shared, p: ItemParams) -> ApiResult<ItemQuery> {
    let bad = || ApiError::bad_request("That filter isn't one of the choices.");
    let job_bit = match p.job.as_deref().filter(|j| !j.is_empty()) {
        None => 0,
        Some(j) => JOBS.iter().position(|x| *x == j).map(|n| 1i64 << n).ok_or_else(bad)?,
    };
    let slot_bit = match p.slot.as_deref().filter(|s| !s.is_empty()) {
        None => 0,
        Some("ear") => 0b11 << 11,
        Some("ring") => 0b11 << 13,
        Some(s) => SLOTS.iter().position(|x| *x == s).map(|n| 1i64 << n).ok_or_else(bad)?,
    };
    let category = p.category.unwrap_or(0);
    if !(0..=65).contains(&category) {
        return Err(bad());
    }
    let level = |v: Option<i64>| v.unwrap_or(0).clamp(0, 99);
    let page = p.page.unwrap_or(1).clamp(1, PAGE_MAX);
    let per = p.per.unwrap_or(PER_DEFAULT).clamp(10, PER_MAX);
    let level_cap = crate::lsb_settings::max_level_async(state.cfg.lsb_settings.clone()).await;
    Ok(ItemQuery { q: fold_search(p.q.as_deref()), category, job_bit, slot_bit, min_level: level(p.min), max_level: level(p.max), level_cap, page, per })
}

#[derive(Serialize)]
struct StatOut {
    name: String,
    value: i64,
}

async fn detail(State(state): State<Shared>, Signed(id): Signed, Path(itemid): Path<i64>) -> ApiResult<impl IntoResponse> {
    throttle(&state, &id)?;
    if !(1..=65535).contains(&itemid) {
        return Err(ApiError::not_found());
    }
    let level_cap = crate::lsb_settings::max_level_async(state.cfg.lsb_settings.clone()).await;
    let d = state.game.item(itemid, level_cap).await?.ok_or_else(ApiError::not_found)?;
    let stats: Vec<StatOut> = d
        .mods
        .iter()
        .map(|(m, v)| StatOut { name: MOD_NAMES.get(m).map_or_else(|| format!("Mod {m}"), |n| n.replace('_', " ")), value: *v })
        .collect();
    let weapon = d.weapon.map(|(skill, dmg, delay)| json!({ "skill": skill, "dmg": dmg, "delay": delay }));
    Ok(private(Json(json!({ "item": item_out(&d.item), "ilevel": d.item.ilevel.filter(|l| *l > 0), "weapon": weapon, "stats": stats }))))
}
