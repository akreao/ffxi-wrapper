//! The item database: any signed-in account may search it; it shows static data only.

mod common;

use axum::http::StatusCode;
use common::*;
use ffxi_web::model::*;

fn item(id: i64, name: &str, category: i64, flags: i64) -> ItemRaw {
    ItemRaw { id, name: name.into(), kind: 1, category, stack: 1, flags, ..Default::default() }
}

/// kaelyn (player), halvard (moderator) and a handful of items.
async fn site_with_items() -> Site {
    let s = site().await;
    s.fake.add_account(1, "kaelyn", "hunter22", 0);
    s.fake.add_account(3, "halvard", "gm-password", 3);
    s.fake.add_item(ItemDetailRaw { item: ItemRaw { stack: 12, ..item(4096, "fire_crystal", 35, 0) }, ..Default::default() });
    s.fake.add_item(ItemDetailRaw {
        item: ItemRaw { kind: 7, level: Some(7), jobs: 0b101, slots: 1, ..item(16535, "bronze_sword", 3, 0x4000 | 0x8000 | 0x40) },
        weapon: Some((3, 6, 231)),
        mods: vec![(1, 2), (11, 1), (60_000, 3)],
    });
    s.fake.add_item(ItemDetailRaw { item: ItemRaw { level: Some(30), jobs: 1 << 21, slots: 0b11 << 11, ..item(13000, "fighters_earring", 24, 0) }, ..Default::default() });
    add_hidden(&s);
    s
}

/// Items players can't have: never listed, and their pages answer like missing ones.
const HIDDEN: [i64; 3] = [10, 65535, 2190];

fn add_hidden(s: &Site) {
    // LSB's GM-only flag (the Judge's set).
    s.fake.add_item(ItemDetailRaw { item: item(10, "judges_sword", 3, 2), ..Default::default() });
    // Gil: currency (type 8) that LSB keeps as an item.
    s.fake.add_item(ItemDetailRaw { item: ItemRaw { kind: 8, stack: 0, ..item(65535, "gil", 0, 0) }, ..Default::default() });
    // An unknown retail item LSB has only a placeholder row for.
    s.fake.add_item(ItemDetailRaw { item: item(2190, "???_sword", 0, 0x40 | 0x1000 | 0x2000), ..Default::default() });
}

#[tokio::test]
async fn needs_a_sign_in_and_any_role_will_do() {
    let s = site_with_items().await;
    assert_eq!(s.send(Req::get("/api/items")).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(s.send(Req::get("/api/items/4096")).await.status, StatusCode::UNAUTHORIZED);
    let r = s.send(Req::get("/items")).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(r.header("location").as_deref(), Some("/signin?next=/items"));

    let (player, _) = s.sign_in("kaelyn", "hunter22").await;
    let j = s.send(Req::get("/api/items").cookie(&player)).await.json();
    assert_eq!(j["total"], 3);
    assert_eq!(s.send(Req::get("/items").cookie(&player)).await.status, StatusCode::OK);
    // The admin copy (where "Give item" will go) stays staff-only.
    assert_eq!(s.send(Req::get("/admin/items").cookie(&player)).await.status, StatusCode::NOT_FOUND);

    let (m, _) = s.sign_in("halvard", "gm-password").await;
    assert_eq!(s.send(Req::get("/api/items").cookie(&m)).await.json()["total"], 3);
    assert_eq!(s.send(Req::get("/admin/items").cookie(&m)).await.status, StatusCode::OK);
}

#[tokio::test]
async fn shows_static_data_and_nothing_internal() {
    let s = site_with_items().await;
    let (c, _) = s.sign_in("kaelyn", "hunter22").await;
    let j = s.send(Req::get("/api/items?q=bronze").cookie(&c)).await.json();
    let it = &j["items"][0];
    assert_eq!(it["name"], "Bronze Sword");
    assert_eq!(it["rare"], true);
    assert_eq!(it["ex"], true);
    assert_eq!(it["level"], 7);
    assert_eq!(it["jobs"], 0b101);
    assert_eq!(it["category"], 3);
    // No raw flags, type or anything else.
    let mut keys: Vec<&str> = it.as_object().unwrap().keys().map(|k| k.as_str()).collect();
    keys.sort();
    assert_eq!(keys, ["category", "ex", "id", "jobs", "level", "name", "rare", "slots", "stack"]);

    let d = s.send(Req::get("/api/items/16535").cookie(&c)).await.json();
    assert_eq!(d["weapon"]["dmg"], 6);
    assert_eq!(d["weapon"]["delay"], 231);
    assert_eq!(d["stats"][0]["name"], "DEF");
    assert_eq!(d["stats"][0]["value"], 2);
    assert_eq!(d["stats"][1]["name"], "AGI");
    assert_eq!(d["stats"][2]["name"], "Mod 60000");

    // Hidden and missing items answer alike, and searches never count them.
    for id in HIDDEN {
        let r = s.send(Req::get(&format!("/api/items/{id}")).cookie(&c)).await;
        assert_eq!((id, r.status, r.json()), (id, StatusCode::NOT_FOUND, serde_json::json!({ "error": "Not found." })));
    }
    for q in ["judges", "gil", "???", "%3F%3F%3F", "sword"] {
        let j = s.send(Req::get(&format!("/api/items?q={q}")).cookie(&c)).await.json();
        let ids: Vec<i64> = j["items"].as_array().unwrap().iter().map(|i| i["id"].as_i64().unwrap()).collect();
        assert!(ids.iter().all(|id| !HIDDEN.contains(id)), "{q}: {ids:?}");
        assert_eq!(j["total"].as_i64().unwrap(), ids.len() as i64, "{q}");
    }
    assert_eq!(s.send(Req::get("/api/items/9999").cookie(&c)).await.status, StatusCode::NOT_FOUND);
    assert_eq!(s.send(Req::get("/api/items/0").cookie(&c)).await.status, StatusCode::NOT_FOUND);
    assert_eq!(s.send(Req::get("/api/items/-1").cookie(&c)).await.status, StatusCode::NOT_FOUND);
    // Signed-in data: no shared cache keeps it.
    for path in ["/api/items", "/api/items/4096"] {
        assert_eq!(s.send(Req::get(path).cookie(&c)).await.header("cache-control").as_deref(), Some("private, no-store"), "{path}");
    }
}

#[tokio::test]
async fn gear_past_the_servers_max_level_is_hidden() {
    let mut cfg = ffxi_web::config::Config::for_tests();
    cfg.lsb_settings = Some(std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/settings-era")));
    let s = site_with(cfg).await;
    s.fake.add_account(1, "kaelyn", "hunter22", 0);
    for (id, name, level) in [(1, "level_75_body", 75), (2, "level_99_body", 99)] {
        s.fake.add_item(ItemDetailRaw { item: ItemRaw { level: Some(level), jobs: 1, slots: 32, ..item(id, name, 18, 0) }, ..Default::default() });
    }
    s.fake.add_item(ItemDetailRaw { item: item(3, "fire_crystal", 35, 0), ..Default::default() });
    let (c, _) = s.sign_in("kaelyn", "hunter22").await;
    let j = s.send(Req::get("/api/items").cookie(&c)).await.json();
    assert_eq!(j["total"], 2);
    assert_eq!(s.fake.0.lock().unwrap().item_queries.last().unwrap().level_cap, 75);
    assert_eq!(s.send(Req::get("/api/items/1").cookie(&c)).await.status, StatusCode::OK);
    assert_eq!(s.send(Req::get("/api/items/2").cookie(&c)).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn filters_and_paging_are_checked_and_capped() {
    let s = site_with_items().await;
    let (c, _) = s.sign_in("kaelyn", "hunter22").await;
    let names = |j: serde_json::Value| j["items"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(names(s.send(Req::get("/api/items?job=whm").cookie(&c)).await.json()), ["Bronze Sword"]);
    assert_eq!(names(s.send(Req::get("/api/items?job=run&slot=ear").cookie(&c)).await.json()), ["Fighters Earring"]);
    assert_eq!(names(s.send(Req::get("/api/items?min=10&max=40").cookie(&c)).await.json()), ["Fighters Earring"]);
    assert_eq!(names(s.send(Req::get("/api/items?category=35").cookie(&c)).await.json()), ["Fire Crystal"]);
    assert_eq!(names(s.send(Req::get("/api/items?q=FIRE%20cry").cookie(&c)).await.json()), ["Fire Crystal"]);
    for bad in ["job=xyz", "slot=tail", "category=66", "category=-1"] {
        assert_eq!(s.send(Req::get(&format!("/api/items?{bad}")).cookie(&c)).await.status, StatusCode::BAD_REQUEST, "{bad}");
    }
    // Page size and page number are capped, and the search text is cut short.
    let long = "a".repeat(500);
    let j = s.send(Req::get(&format!("/api/items?per=100000&page=99999999&q={long}")).cookie(&c)).await.json();
    assert_eq!(j["per"], 100);
    assert_eq!(j["page"], 1000);
    let asked = s.fake.0.lock().unwrap().item_queries.last().unwrap().clone();
    assert_eq!(asked.q.chars().count(), 40);
    assert_eq!(asked.per, 100);
}

#[tokio::test]
async fn reads_are_limited_per_account() {
    let s = site_with_items().await;
    let (a, _) = s.sign_in("kaelyn", "hunter22").await;
    // A second session of the same account shares the limit.
    let (b, _) = s.sign_in("kaelyn", "hunter22").await;
    for _ in 0..30 {
        assert_eq!(s.send(Req::get("/api/items").cookie(&a)).await.status, StatusCode::OK);
        assert_eq!(s.send(Req::get("/api/items/4096").cookie(&b)).await.status, StatusCode::OK);
    }
    assert_eq!(s.send(Req::get("/api/items").cookie(&a)).await.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(s.send(Req::get("/api/items").cookie(&b)).await.status, StatusCode::TOO_MANY_REQUESTS);
    // Another account isn't held up.
    let (m, _) = s.sign_in("halvard", "gm-password").await;
    assert_eq!(s.send(Req::get("/api/items").cookie(&m)).await.status, StatusCode::OK);
    s.advance(61);
    assert_eq!(s.send(Req::get("/api/items").cookie(&a)).await.status, StatusCode::OK);
}

#[test]
fn search_text_is_never_a_like_pattern() {
    use ffxi_web::game::like_pattern;
    assert_eq!(like_pattern("a%b_c|d"), "%a|%b|_c||d%");
    assert_eq!(like_pattern(""), "%%");
}

#[test]
fn the_level_cap_is_read_like_lsb_loads_it() {
    use ffxi_web::routes::items::server_max_level;
    use std::path::Path;
    let fixture = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/settings-era"));
    // main.lua says 60 (its commented 10 and the mob range don't count); zz_app.lua, read last, says 75.
    assert_eq!(server_max_level(Some(fixture)), 75);
    // No settings folder, or one that isn't there: LSB's default.
    assert_eq!(server_max_level(None), 99);
    assert_eq!(server_max_level(Some(Path::new("/no/such/folder"))), 99);
}

#[test]
fn names_read_like_the_game() {
    use ffxi_web::routes::items::{display_name, fold_search};
    assert_eq!(display_name("pile_of_chocobo_bedding"), "Pile of Chocobo Bedding");
    assert_eq!(display_name("bronze_sword"), "Bronze Sword");
    assert_eq!(fold_search(Some("  Fire Crystal\u{0} ")), "fire_crystal");
}
