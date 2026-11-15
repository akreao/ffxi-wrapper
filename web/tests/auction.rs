//! The auction house: read-only, prices for any signed-in account, names for staff only.

mod common;

use axum::http::StatusCode;
use common::*;
use ffxi_web::model::*;

fn item(id: i64, name: &str, stack: i64, flags: i64) -> ItemDetailRaw {
    ItemDetailRaw { item: ItemRaw { id, name: name.into(), kind: 1, category: 35, stack, flags, ..Default::default() }, ..Default::default() }
}

fn listing(price: i64, seller: i64, name: &str) -> AuctionRow {
    AuctionRow { price, at: 1_700_000_000, seller, seller_name: Some(name.into()), ..Default::default() }
}

fn sale(price: i64, at: i64, seller: (i64, &str), buyer: (i64, &str)) -> AuctionRow {
    AuctionRow { price, at, seller: seller.0, seller_name: Some(seller.1.into()), buyer: buyer.0, buyer_name: Some(buyer.1.into()) }
}

/// kaelyn (player), halvard (moderator); fire crystals with player and bot rows, a sword
/// that doesn't stack, and listings of items players can't have.
async fn site_with_auction() -> Site {
    let s = site().await;
    s.fake.add_account(1, "kaelyn", "hunter22", 0);
    s.fake.add_account(3, "halvard", "gm-password", 3);
    s.fake.add_item(item(4096, "fire_crystal", 12, 0));
    s.fake.add_item(item(16535, "bronze_sword", 1, 0));
    s.fake.add_item(item(4097, "ice_crystal", 12, 0));
    s.fake.add_item(item(10, "judges_sword", 1, 2));
    s.fake.add_item(ItemDetailRaw { item: ItemRaw { kind: 8, ..item(65535, "gil", 0, 0).item }, ..Default::default() });

    s.fake.add_auction(4096, false, listing(300, 21828, "Pipiru"));
    s.fake.add_auction(4096, false, listing(250, 0, "M.H.M.U."));
    s.fake.add_auction(4096, true, listing(3000, 0, "M.H.M.U."));
    s.fake.add_auction(4096, false, sale(280, 1_700_000_100, (21828, "Pipiru"), (0, "M.H.M.U.")));
    s.fake.add_auction(4096, false, sale(260, 1_700_000_200, (0, "M.H.M.U."), (21900, "Kaelyn")));
    s.fake.add_auction(16535, false, listing(1200, 21900, "Kaelyn"));
    // Sold out: only history.
    s.fake.add_auction(4097, false, sale(200, 1_700_000_000, (0, "M.H.M.U."), (21828, "Pipiru")));
    s.fake.add_auction(10, false, listing(1, 0, "M.H.M.U."));
    s.fake.add_auction(65535, false, listing(1, 21828, "Pipiru"));
    s
}

#[tokio::test]
async fn needs_a_sign_in_and_any_role_will_do() {
    let s = site_with_auction().await;
    assert_eq!(s.send(Req::get("/api/auction")).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(s.send(Req::get("/api/auction/4096")).await.status, StatusCode::UNAUTHORIZED);
    let r = s.send(Req::get("/auction")).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(r.header("location").as_deref(), Some("/signin?next=/auction"));

    let (player, _) = s.sign_in("kaelyn", "hunter22").await;
    assert_eq!(s.send(Req::get("/auction").cookie(&player)).await.status, StatusCode::OK);
    assert_eq!(s.send(Req::get("/admin/auction").cookie(&player)).await.status, StatusCode::NOT_FOUND);
    let (m, _) = s.sign_in("halvard", "gm-password").await;
    assert_eq!(s.send(Req::get("/admin/auction").cookie(&m)).await.status, StatusCode::OK);
}

#[tokio::test]
async fn lists_what_is_for_sale_without_hidden_items() {
    let s = site_with_auction().await;
    let (c, _) = s.sign_in("kaelyn", "hunter22").await;
    let r = s.send(Req::get("/api/auction").cookie(&c)).await;
    assert_eq!(r.header("cache-control").as_deref(), Some("private, no-store"));
    let j = r.json();
    // Ice crystals have only sales; the Judge's sword and gil are never shown.
    assert_eq!(j["total"], 2);
    let fire = &j["items"].as_array().unwrap().iter().find(|i| i["id"] == 4096).unwrap();
    assert_eq!(fire["name"], "Fire Crystal");
    // Players get counts, never asking prices (buying in game is a blind bid).
    assert_eq!((fire["singles"].as_i64(), fire["stacks"].as_i64()), (Some(2), Some(1)));
    assert!(fire.get("single_low").is_none() && fire.get("stack_low").is_none(), "{fire}");
    let (m, _) = s.sign_in("halvard", "gm-password").await;
    let j = s.send(Req::get("/api/auction").cookie(&m)).await.json();
    let fire = &j["items"].as_array().unwrap().iter().find(|i| i["id"] == 4096).unwrap();
    assert_eq!((fire["single_low"].as_i64(), fire["stack_low"].as_i64()), (Some(250), Some(3000)));
    // The item search's filters apply.
    assert_eq!(s.send(Req::get("/api/auction?q=sword").cookie(&c)).await.json()["total"], 1);
    assert_eq!(s.send(Req::get("/api/auction?job=nope").cookie(&c)).await.status, StatusCode::BAD_REQUEST);

    for id in [10, 65535, 0, 70000] {
        assert_eq!(s.send(Req::get(&format!("/api/auction/{id}")).cookie(&c)).await.status, StatusCode::NOT_FOUND, "{id}");
    }
}

#[tokio::test]
async fn players_see_counts_and_sales_but_no_asking_prices_or_names() {
    let s = site_with_auction().await;
    let (c, _) = s.sign_in("kaelyn", "hunter22").await;
    let r = s.send(Req::get("/api/auction/4096").cookie(&c)).await;
    let j = r.json();
    let body = j["sides"].to_string();
    assert_eq!(j["names"], false);
    let single = &j["sides"][0];
    assert_eq!(single["stack"], false);
    assert_eq!(single["listed"], 2);
    // No listings, so no asking prices: only how many are up.
    assert!(single.get("listings").is_none(), "{single}");
    // Newest sale first.
    assert_eq!(single["sales"][0]["price"], 260);
    assert_eq!(single["sales"][0]["buyer"], serde_json::json!({ "bot": false }));
    assert_eq!(single["sales"][1]["buyer"], serde_json::json!({ "bot": true }));
    assert_eq!(j["sides"][1]["stack"], true);
    assert_eq!(j["sides"][1]["listed"], 1);
    // No asking prices (300, 250, 3000), character names or ids anywhere.
    for secret in ["300", "250", "3000", "listings", "Pipiru", "Kaelyn", "M.H.M.U.", "21828", "21900", "charid", "\"name\""] {
        assert!(!body.contains(secret), "{secret} in {body}");
    }

    // Items that don't stack have no stack side.
    let sword = s.send(Req::get("/api/auction/16535").cookie(&c)).await.json();
    assert_eq!(sword["sides"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn staff_see_names_and_character_links() {
    let s = site_with_auction().await;
    let (m, _) = s.sign_in("halvard", "gm-password").await;
    let j = s.send(Req::get("/api/auction/4096").cookie(&m)).await.json();
    assert_eq!(j["names"], true);
    let single = &j["sides"][0];
    // Cheapest first, with asking prices; a listing has no buyer.
    assert_eq!(single["listings"][0]["price"], 250);
    assert!(single["listings"][0].get("buyer").is_none());
    assert_eq!(j["sides"][1]["listings"][0]["price"], 3000);
    assert_eq!(single["listings"][0]["seller"], serde_json::json!({ "bot": true, "name": "M.H.M.U." }));
    assert_eq!(single["listings"][1]["seller"], serde_json::json!({ "bot": false, "name": "Pipiru", "charid": 21828 }));
    assert_eq!(single["sales"][0]["buyer"], serde_json::json!({ "bot": false, "name": "Kaelyn", "charid": 21900 }));
}

#[tokio::test]
async fn shares_the_item_search_rate_limit() {
    let s = site_with_auction().await;
    let (c, _) = s.sign_in("kaelyn", "hunter22").await;
    let mut last = StatusCode::OK;
    for _ in 0..61 {
        last = s.send(Req::get("/api/auction/4096").cookie(&c)).await.status;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(s.send(Req::get("/api/items").cookie(&c)).await.status, StatusCode::TOO_MANY_REQUESTS);
}
