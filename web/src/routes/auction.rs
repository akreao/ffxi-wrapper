//! The auction house, read-only: what is up for sale, and each item's listings and recent
//! sales. Players see what the game shows them: how many are for sale and the sale history
//! (prices and dates), never a listing's asking price, since buying in game is a blind bid.
//! Moderators and the owner also see the listings with their prices, and who sold and bought
//! (names, and links to the character). Rows the auction house
//! bot wrote are marked as the bot's for everyone. Only items the item database shows
//! (`ITEM_SHOWN`) appear. Nothing here changes a listing: cancelling one would race the bot
//! and need the item sent back. The bot's settings stay in the desktop app.

use super::items::{item_out, item_query, private, throttle, ItemOut, ItemParams};
use crate::http::*;
use crate::model::*;
use crate::state::Shared;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use serde_json::json;

pub fn api() -> Router<Shared> {
    Router::new().route("/auction", get(list)).route("/auction/{id}", get(detail))
}

/// The cheapest listings shown per side (the count says how many more there are).
const LISTINGS_MAX: i64 = 50;
/// Recent sales per side; the game's own history window shows 10.
const SALES_MAX: i64 = 20;

#[derive(Serialize)]
struct ListedOut {
    #[serde(flatten)]
    item: ItemOut,
    singles: i64,
    /// Staff only: the cheapest asking price.
    #[serde(skip_serializing_if = "Option::is_none")]
    single_low: Option<i64>,
    stacks: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    stack_low: Option<i64>,
}

async fn list(State(state): State<Shared>, Signed(id): Signed, Query(p): Query<ItemParams>) -> ApiResult<impl IntoResponse> {
    throttle(&state, &id)?;
    let query = item_query(&state, p).await?;
    let found = state.game.auction_listed(&query).await?;
    let staff = id.role.is_admin();
    let items: Vec<ListedOut> = found
        .items
        .iter()
        .map(|l| ListedOut { item: item_out(&l.item), singles: l.singles, single_low: l.single_low.filter(|_| staff), stacks: l.stacks, stack_low: l.stack_low.filter(|_| staff) })
        .collect();
    Ok(private(Json(json!({ "items": items, "total": found.total, "page": query.page, "per": query.per }))))
}

/// A seller or buyer. Everyone learns whether it was the bot; staff also get the name, and the
/// character id for a link (never sent to players).
#[derive(Serialize, Debug, PartialEq)]
struct PartyOut {
    bot: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    charid: Option<i64>,
}

#[derive(Serialize, Debug, PartialEq)]
struct RowOut {
    price: i64,
    at: i64,
    seller: PartyOut,
    #[serde(skip_serializing_if = "Option::is_none")]
    buyer: Option<PartyOut>,
}

/// ffxiahbot lists and seeds prices as seller 0, and buys by setting only the buyer's name, so
/// its rows have id 0 where LSB always writes the character's id.
fn party(id: i64, name: Option<&str>, staff: bool) -> PartyOut {
    let bot = id == 0;
    PartyOut {
        bot,
        name: if staff { name.map(str::to_owned) } else { None },
        charid: if staff && !bot { Some(id) } else { None },
    }
}

fn row_out(r: &AuctionRow, sale: bool, staff: bool) -> RowOut {
    RowOut {
        price: r.price,
        at: r.at,
        seller: party(r.seller, r.seller_name.as_deref(), staff),
        buyer: sale.then(|| party(r.buyer, r.buyer_name.as_deref(), staff)),
    }
}

async fn detail(State(state): State<Shared>, Signed(id): Signed, Path(itemid): Path<i64>) -> ApiResult<impl IntoResponse> {
    throttle(&state, &id)?;
    if !(1..=65535).contains(&itemid) {
        return Err(ApiError::not_found());
    }
    let level_cap = crate::lsb_settings::max_level_async(state.cfg.lsb_settings.clone()).await;
    let item = state.game.item(itemid, level_cap).await?.ok_or_else(ApiError::not_found)?.item;
    let staff = id.role.is_admin();
    let mut sides = Vec::new();
    // Stacks only for items that stack; LSB refuses stack listings of the others.
    for stack in [false, true].into_iter().filter(|s| !s || item.stack > 1) {
        let side = state.game.auction_side(itemid, stack, if staff { LISTINGS_MAX } else { 0 }, SALES_MAX).await?;
        let mut out = json!({
            "stack": stack,
            "listed": side.listed,
            "sales": side.sales.iter().map(|r| row_out(r, true, staff)).collect::<Vec<_>>(),
        });
        // Asking prices are staff-only: players only learn how many are up, as in game.
        if staff {
            out["listings"] = json!(side.listings.iter().map(|r| row_out(r, false, true)).collect::<Vec<_>>());
        }
        sides.push(out);
    }
    Ok(private(Json(json!({ "item": item_out(&item), "names": staff, "sides": sides }))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sale() -> AuctionRow {
        AuctionRow { price: 900, at: 10, seller: 0, seller_name: Some("M.H.M.U.".into()), buyer: 21828, buyer_name: Some("Pipiru".into()) }
    }

    #[test]
    fn players_get_no_names_or_ids() {
        let out = row_out(&sale(), true, false);
        assert_eq!(out.seller, PartyOut { bot: true, name: None, charid: None });
        assert_eq!(out.buyer, Some(PartyOut { bot: false, name: None, charid: None }));
        let text = serde_json::to_string(&out).unwrap();
        assert!(!text.contains("21828") && !text.contains("Pipiru") && !text.contains("M.H.M.U."), "{text}");
    }

    #[test]
    fn staff_get_names_and_player_ids() {
        let out = row_out(&sale(), true, true);
        assert_eq!(out.seller, PartyOut { bot: true, name: Some("M.H.M.U.".into()), charid: None });
        assert_eq!(out.buyer, Some(PartyOut { bot: false, name: Some("Pipiru".into()), charid: Some(21828) }));
    }

    #[test]
    fn listings_have_no_buyer() {
        let out = row_out(&sale(), false, true);
        assert_eq!(out.buyer, None);
    }
}
