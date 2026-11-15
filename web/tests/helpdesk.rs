//! The admin panel's Help desk: listing tickets, replying, closing, the rank rule and the log.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};

/// kaelyn (player), halvard (moderator, GM 3), moira (GM 4), steven (owner, GM 5).
async fn site_with_tickets() -> Site {
    let s = site().await;
    s.fake.add_account(1, "kaelyn", "hunter22", 0);
    s.fake.add_account(3, "halvard", "gm-password", 3);
    s.fake.add_account(4, "steven", "owner-pass", 5);
    s.fake.add_account(5, "moira", "gm4-password", 4);
    s.fake.add_ticket(10, "I'm stuck in a wall <script>alert(1)</script>");
    s.fake.add_ticket(50, "A GM 4 asks for help");
    s
}

async fn signed(s: &Site, login: &str, pw: &str) -> (String, String) {
    s.sign_in(login, pw).await
}

fn post(w: &(String, String), path: &str, body: Value) -> Req {
    Req::post(path, body).session(&w.0, &w.1)
}

async fn log(s: &Site, w: &(String, String)) -> Vec<(String, String)> {
    let j = s.send(Req::get("/api/admin/log").cookie(&w.0)).await.json();
    j["items"].as_array().unwrap().iter().map(|a| (a["action"].as_str().unwrap().into(), a["result"].as_str().unwrap().into())).collect()
}

#[tokio::test]
async fn only_staff_see_tickets() {
    let s = site_with_tickets().await;
    let player = signed(&s, "kaelyn", "hunter22").await;
    let m = signed(&s, "halvard", "gm-password").await;
    assert_eq!(s.send(Req::get("/api/admin/tickets")).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(s.send(Req::get("/api/admin/tickets").cookie(&player.0)).await.status, StatusCode::FORBIDDEN);
    let r = s.send(post(&player, "/api/admin/tickets/1/reply", json!({ "text": "hi" }))).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let j = s.send(Req::get("/api/admin/tickets").cookie(&m.0)).await.json();
    assert_eq!(j["total"], 2);
    // Newest first; text comes back as stored (the page shows it as text).
    assert_eq!(j["items"][0]["id"], 2);
    assert_eq!(j["items"][1]["message"], "I'm stuck in a wall <script>alert(1)</script>");
    assert_eq!(j["items"][1]["character"], "kaelynchar");
    assert_eq!(j["items"][1]["login"], "kaelyn");
    assert_eq!(j["items"][1]["can_act"], true);
    assert_eq!(j["items"][0]["can_act"], false);
    assert_eq!(j["counts"]["open"], 2);
}

#[tokio::test]
async fn reply_moves_the_ticket_to_answered() {
    let s = site_with_tickets().await;
    let m = signed(&s, "halvard", "gm-password").await;
    let r = s.send(post(&m, "/api/admin/tickets/1/reply", json!({ "text": "  Use /warp.\r\nGood luck!  " }))).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(s.fake.ticket(1).response.as_deref(), Some("Use /warp.\nGood luck!"));
    let j = s.send(Req::get("/api/admin/tickets?status=answered").cookie(&m.0)).await.json();
    assert_eq!(j["total"], 1);
    assert_eq!(j["counts"]["open"], 1);
    assert_eq!(log(&s, &m).await[0], ("ticket_reply".into(), "ok".into()));
    // A second reply (another GM at the same time, or a change) is refused and the first kept.
    assert_eq!(s.send(post(&m, "/api/admin/tickets/1/reply", json!({ "text": "Or ask a friend." }))).await.status, StatusCode::CONFLICT);
    assert_eq!(s.fake.ticket(1).response.as_deref(), Some("Use /warp.\nGood luck!"));
    assert_eq!(log(&s, &m).await[0], ("ticket_reply".into(), "ok".into()));
    assert_eq!(log(&s, &m).await.iter().filter(|a| a.0 == "ticket_reply").count(), 1);
    // An answered ticket can't be closed without a reply.
    assert_eq!(s.send(post(&m, "/api/admin/tickets/1/close", json!({}))).await.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn replies_are_plain_ascii_up_to_1024_bytes() {
    let s = site_with_tickets().await;
    let m = signed(&s, "halvard", "gm-password").await;
    for bad in [json!({ "text": "" }), json!({ "text": "   " }), json!({}), json!({ "text": "café" }), json!({ "text": "tab\there" }), json!({ "text": "x".repeat(1025) })] {
        let r = s.send(post(&m, "/api/admin/tickets/1/reply", bad.clone())).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{bad}");
    }
    assert!(s.fake.ticket(1).response.is_none());
    let r = s.send(post(&m, "/api/admin/tickets/1/reply", json!({ "text": "x".repeat(1024) }))).await;
    assert_eq!(r.status, StatusCode::OK);
}

#[tokio::test]
async fn close_without_reply() {
    let s = site_with_tickets().await;
    let m = signed(&s, "halvard", "gm-password").await;
    let r = s.send(post(&m, "/api/admin/tickets/1/close", json!({ "reason": "Sorted out in game." }))).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert!(s.fake.ticket(1).closed.is_some());
    assert_eq!(s.send(post(&m, "/api/admin/tickets/1/close", json!({}))).await.status, StatusCode::CONFLICT);
    assert_eq!(s.send(post(&m, "/api/admin/tickets/1/reply", json!({ "text": "late" }))).await.status, StatusCode::CONFLICT);
    let j = s.send(Req::get("/api/admin/tickets?status=closed").cookie(&m.0)).await.json();
    assert_eq!(j["total"], 1);
    assert_eq!(s.send(post(&m, "/api/admin/tickets/99/close", json!({}))).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn moderators_cant_act_on_equal_or_higher_staff() {
    let s = site_with_tickets().await;
    let m = signed(&s, "halvard", "gm-password").await;
    // Ticket 2 was filed by moira's character (GM 4).
    assert_eq!(s.send(post(&m, "/api/admin/tickets/2/reply", json!({ "text": "no" }))).await.status, StatusCode::FORBIDDEN);
    assert_eq!(s.send(post(&m, "/api/admin/tickets/2/close", json!({}))).await.status, StatusCode::FORBIDDEN);
    assert!(s.fake.ticket(2).response.is_none() && s.fake.ticket(2).closed.is_none());
    assert_eq!(&log(&s, &m).await[..2], &[("ticket_close".into(), "refused".into()), ("ticket_reply".into(), "refused".into())]);
    // Equal rank: halvard's own ticket.
    let own = s.fake.add_ticket(30, "my own");
    assert_eq!(s.send(post(&m, &format!("/api/admin/tickets/{own}/reply"), json!({ "text": "no" }))).await.status, StatusCode::FORBIDDEN);
    // The owner may.
    let o = signed(&s, "steven", "owner-pass").await;
    assert_eq!(s.send(post(&o, "/api/admin/tickets/2/reply", json!({ "text": "yes" }))).await.status, StatusCode::OK);
}

#[tokio::test]
async fn changes_need_the_csrf_token() {
    let s = site_with_tickets().await;
    let m = signed(&s, "halvard", "gm-password").await;
    let r = s.send(Req::post("/api/admin/tickets/1/reply", json!({ "text": "hi" })).cookie(&m.0)).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert!(s.fake.ticket(1).response.is_none());
}
