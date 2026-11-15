//! Give item: owners put an item in a character's delivery box, logged with a reason.

mod common;

use axum::http::StatusCode;
use common::*;
use ffxi_web::model::*;
use serde_json::{json, Value};

/// kaelyn (player, character 10), halvard (moderator), steven (owner) and three items:
/// a crystal stacking to 12, a sword stacking to 1 and a GM-only sword.
async fn site_for_gives() -> Site {
    let s = site().await;
    s.fake.add_account(1, "kaelyn", "hunter22", 0);
    s.fake.add_account(3, "halvard", "gm-password", 3);
    s.fake.add_account(4, "steven", "owner-pass", 5);
    s.fake.add_item(ItemDetailRaw { item: ItemRaw { id: 4096, name: "fire_crystal".into(), kind: 1, category: 35, stack: 12, ..Default::default() }, ..Default::default() });
    s.fake.add_item(ItemDetailRaw { item: ItemRaw { id: 16535, name: "bronze_sword".into(), kind: 4, stack: 1, ..Default::default() }, ..Default::default() });
    s.fake.add_item(ItemDetailRaw { item: ItemRaw { id: 10, name: "judges_sword".into(), kind: 4, stack: 1, flags: 2, ..Default::default() }, ..Default::default() });
    // Rare/Ex: a character can only ever hold one.
    s.fake.add_item(ItemDetailRaw { item: ItemRaw { id: 13685, name: "destrier_beret".into(), kind: 5, stack: 1, flags: 0x8000 | 0x4000, ..Default::default() }, ..Default::default() });
    s
}

fn post(cookie: &str, csrf: &str, path: &str, body: Value) -> Req {
    Req::post(path, body).session(cookie, csrf)
}

fn gives(s: &Site) -> Vec<(i64, i64, i64, String)> {
    s.fake.0.lock().unwrap().gives.clone()
}

#[tokio::test]
async fn only_owners_give() {
    let s = site_for_gives().await;
    let body = json!({ "charid": 10, "quantity": 1, "reason": "Event prize" });
    assert!(s.send(Req::post("/api/admin/items/4096/give", body.clone())).await.status.is_client_error());
    let (p, pc) = s.sign_in("kaelyn", "hunter22").await;
    assert_eq!(s.send(post(&p, &pc, "/api/admin/items/4096/give", body.clone())).await.status, StatusCode::FORBIDDEN);
    let (m, mc) = s.sign_in("halvard", "gm-password").await;
    assert_eq!(s.send(post(&m, &mc, "/api/admin/items/4096/give", body.clone())).await.status, StatusCode::FORBIDDEN);
    // An owner's session without its CSRF token is turned away before anything happens.
    let (o, _) = s.sign_in("steven", "owner-pass").await;
    assert_eq!(s.send(post(&o, "wrong", "/api/admin/items/4096/give", body)).await.status, StatusCode::FORBIDDEN);
    assert!(gives(&s).is_empty());
}

#[tokio::test]
async fn a_give_lands_in_the_box_and_the_log() {
    let s = site_for_gives().await;
    let (o, oc) = s.sign_in("steven", "owner-pass").await;
    let r = s.send(post(&o, &oc, "/api/admin/items/4096/give", json!({ "charid": 10, "quantity": 12, "reason": "Event prize" }))).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["item"], "Fire Crystal");
    assert_eq!(gives(&s), vec![(10, 4096, 12, "GM".to_string())]);

    let log = s.send(Req::get("/api/admin/log?action=give_item").cookie(&o)).await.json();
    let row = &log["items"][0];
    assert_eq!((row["action"].as_str(), row["result"].as_str(), row["target_id"].as_i64()), (Some("give_item"), Some("ok"), Some(10)));
    assert_eq!(row["reason"], "Event prize");
    let d: Value = serde_json::from_str(row["detail"].as_str().unwrap()).unwrap();
    assert_eq!((d["item"].as_i64(), d["quantity"].as_i64(), d["account"].as_str()), (Some(4096), Some(12), Some("kaelyn")));
}

#[tokio::test]
async fn gives_are_checked_on_the_server() {
    let s = site_for_gives().await;
    let (o, oc) = s.sign_in("steven", "owner-pass").await;
    let give = |item: i64, body: Value| s.send(post(&o, &oc, &format!("/api/admin/items/{item}/give"), body));
    // A reason is required.
    assert_eq!(give(4096, json!({ "charid": 10, "quantity": 1 })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(give(4096, json!({ "charid": 10, "quantity": 1, "reason": "  " })).await.status, StatusCode::BAD_REQUEST);
    // One stack at most, at least one.
    assert_eq!(give(4096, json!({ "charid": 10, "quantity": 13, "reason": "x" })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(give(4096, json!({ "charid": 10, "quantity": 0, "reason": "x" })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(give(16535, json!({ "charid": 10, "quantity": 2, "reason": "x" })).await.status, StatusCode::BAD_REQUEST);
    // GM-only and missing items answer like missing ones; so do missing characters.
    assert_eq!(give(10, json!({ "charid": 10, "quantity": 1, "reason": "x" })).await.status, StatusCode::NOT_FOUND);
    assert_eq!(give(99, json!({ "charid": 10, "quantity": 1, "reason": "x" })).await.status, StatusCode::NOT_FOUND);
    assert_eq!(give(4096, json!({ "charid": 999, "quantity": 1, "reason": "x" })).await.status, StatusCode::NOT_FOUND);
    assert!(gives(&s).is_empty());
}

#[tokio::test]
async fn a_full_box_and_the_rate_limit_stop_gives() {
    let s = site_for_gives().await;
    let (o, oc) = s.sign_in("steven", "owner-pass").await;
    let give = || s.send(post(&o, &oc, "/api/admin/items/16535/give", json!({ "charid": 10, "quantity": 1, "reason": "x" })));
    for _ in 0..20 {
        assert_eq!(give().await.status, StatusCode::OK);
    }
    assert_eq!(give().await.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(gives(&s).len(), 20);
    // Ten minutes on, gives are open again, until the box holds LSB's in-flight cap.
    s.advance(601);
    s.fake.0.lock().unwrap().gives.extend((20..ffxi_web::game::DBOX_WAITING_MAX).map(|_| (10, 4096, 1, "GM".to_string())));
    assert_eq!(give().await.status, StatusCode::CONFLICT);
    assert_eq!(gives(&s).len(), ffxi_web::game::DBOX_WAITING_MAX);
    // The refusal is in the log as one.
    let log = s.send(Req::get("/api/admin/log?action=give_item&per=1").cookie(&o)).await.json();
    assert_eq!(log["items"][0]["result"], "refused");
}

#[tokio::test]
async fn a_rare_item_is_given_once() {
    let s = site_for_gives().await;
    let (o, oc) = s.sign_in("steven", "owner-pass").await;
    let give = |charid: i64| s.send(post(&o, &oc, "/api/admin/items/13685/give", json!({ "charid": charid, "quantity": 1, "reason": "Collector" })));
    assert_eq!(give(10).await.status, StatusCode::OK);
    // A second one would be stuck in their box: LSB won't let them take it.
    assert_eq!(give(10).await.status, StatusCode::CONFLICT);
    assert_eq!(give(30).await.status, StatusCode::OK, "another character may have one");
    assert_eq!(gives(&s).len(), 2);
}

#[tokio::test]
async fn nothing_is_given_when_the_log_cant_be_written() {
    let s = site_for_gives().await;
    let (o, oc) = s.sign_in("steven", "owner-pass").await;
    // Only the give's log row fails; sessions keep working.
    sqlx::query("CREATE TRIGGER no_gives BEFORE INSERT ON action_log WHEN NEW.action = 'give_item' BEGIN SELECT RAISE(ABORT, 'disk full'); END")
        .execute(s.state.store.pool())
        .await
        .unwrap();
    let r = s.send(post(&o, &oc, "/api/admin/items/4096/give", json!({ "charid": 10, "quantity": 1, "reason": "x" }))).await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(gives(&s).is_empty());
}
