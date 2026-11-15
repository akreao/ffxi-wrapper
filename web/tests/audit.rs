//! The audit logs: staff only, read-only, tells for the owner alone, bounded searches.

mod common;

use axum::http::StatusCode;
use common::*;
use ffxi_web::model::*;

fn chat(at: i64, who: &str, channel: &str, text: &str, to: Option<&str>) -> AuditRow {
    AuditRow { at, who: who.into(), channel: Some(channel.into()), text: Some(text.into()), to: to.map(Into::into), ..Default::default() }
}

/// kaelyn (player, character Kaelyn), halvard (moderator), steven (owner), and some chat.
async fn site_with_logs() -> Site {
    let s = site().await;
    s.fake.add_account(1, "kaelyn", "hunter22", 0);
    s.fake.add_account(3, "halvard", "gm-password", 3);
    s.fake.add_account(5, "steven", "owner-pass", 5);
    s.fake.add_char(11, 1, "Kaelyn", 0);
    s.fake.add_audit(AuditKind::Chat, chat(NOW - 60, "Kaelyn", "SAY", "<b>hi</b>", None));
    s.fake.add_audit(AuditKind::Chat, chat(NOW - 50, "Kaelyn", "TELL", "secret", Some("Halvard")));
    s.fake.add_audit(AuditKind::Chat, chat(NOW - 40 * 86_400, "Kaelyn", "SHOUT", "old", None));
    s.fake.add_audit(AuditKind::Gm, AuditRow { at: NOW - 30, who: "Halvard".into(), channel: Some("zone".into()), text: Some("!zone 230".into()), ..Default::default() });
    s
}

fn channels(j: &serde_json::Value) -> Vec<String> {
    j["items"].as_array().unwrap().iter().map(|r| r["channel"].as_str().unwrap().to_owned()).collect()
}

#[tokio::test]
async fn staff_only() {
    let s = site_with_logs().await;
    assert_eq!(s.send(Req::get("/api/admin/audit")).await.status, StatusCode::UNAUTHORIZED);
    let (player, _) = s.sign_in("kaelyn", "hunter22").await;
    assert_eq!(s.send(Req::get("/api/admin/audit").cookie(&player)).await.status, StatusCode::FORBIDDEN);
    assert_eq!(s.send(Req::get("/admin/audit").cookie(&player)).await.status, StatusCode::NOT_FOUND);
    let (m, _) = s.sign_in("halvard", "gm-password").await;
    assert_eq!(s.send(Req::get("/admin/audit").cookie(&m)).await.status, StatusCode::OK);
    let r = s.send(Req::get("/api/admin/audit").cookie(&m)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.header("cache-control").as_deref(), Some("private, no-store"));
}

#[tokio::test]
async fn tells_are_the_owners_alone() {
    let s = site_with_logs().await;
    let (m, _) = s.sign_in("halvard", "gm-password").await;
    let j = s.send(Req::get("/api/admin/audit?kind=chat").cookie(&m)).await.json();
    assert_eq!(channels(&j), ["SAY"]);
    assert!(!j["channels"].as_array().unwrap().iter().any(|c| c == "TELL"));
    assert_eq!(s.send(Req::get("/api/admin/audit?kind=chat&channel=TELL").cookie(&m)).await.status, StatusCode::FORBIDDEN);
    assert!(s.fake.audit_queries().iter().all(|q| !q.tells));

    let (o, _) = s.sign_in("steven", "owner-pass").await;
    let j = s.send(Req::get("/api/admin/audit?kind=chat").cookie(&o)).await.json();
    assert_eq!(channels(&j), ["SAY", "TELL"]);
    let j = s.send(Req::get("/api/admin/audit?kind=chat&channel=TELL").cookie(&o)).await.json();
    assert_eq!(channels(&j), ["TELL"]);
    assert_eq!(j["items"][0]["to"], "Halvard");
}

#[tokio::test]
async fn player_text_comes_back_as_plain_data() {
    let s = site_with_logs().await;
    let (m, _) = s.sign_in("halvard", "gm-password").await;
    let j = s.send(Req::get("/api/admin/audit").cookie(&m)).await.json();
    // JSON for app.js to put in text nodes; the server never makes HTML of it.
    assert_eq!(j["items"][0]["text"], "<b>hi</b>");
    assert_eq!(ffxi_web::game::plain_text(b"a\x07b\xfd\x01c"), "a b\u{fffd} c");
}

#[tokio::test]
async fn filters_are_checked_and_bounded() {
    let s = site_with_logs().await;
    let (m, _) = s.sign_in("halvard", "gm-password").await;
    for bad in ["kind=chats", "kind=chat&channel=SAY%27--", "kind=gm&channel=SAY"] {
        assert_eq!(s.send(Req::get(&format!("/api/admin/audit?{bad}")).cookie(&m)).await.status, StatusCode::BAD_REQUEST, "{bad}");
    }
    let long = "x".repeat(500);
    let j = s
        .send(Req::get(&format!("/api/admin/audit?kind=gm&page=999&per=5000&from=0&q={long}&who={long}")).cookie(&m))
        .await
        .json();
    assert_eq!((j["page"].as_i64(), j["per"].as_i64()), (Some(40), Some(100)));
    let q = s.fake.audit_queries().pop().unwrap();
    assert_eq!(q.kind, AuditKind::Gm);
    assert_eq!((q.page, q.per, q.q.len(), q.names[0].len()), (40, 100, 64, 15));
    // Nothing older than the age limit, whatever `from` says.
    assert_eq!(q.from, NOW - 90 * 86_400);

    // The default window is the last week: the 40-day-old shout needs a wider one.
    let j = s.send(Req::get("/api/admin/audit?channel=SHOUT").cookie(&m)).await.json();
    assert!(j["items"].as_array().unwrap().is_empty());
    let j = s.send(Req::get(&format!("/api/admin/audit?channel=SHOUT&from={}", NOW - 60 * 86_400)).cookie(&m)).await.json();
    assert_eq!(channels(&j), ["SHOUT"]);
}

#[tokio::test]
async fn an_account_filter_means_its_characters() {
    let s = site_with_logs().await;
    let (m, _) = s.sign_in("halvard", "gm-password").await;
    let j = s.send(Req::get("/api/admin/audit?account=1").cookie(&m)).await.json();
    assert_eq!(channels(&j), ["SAY"]);
    assert_eq!(s.fake.audit_queries().pop().unwrap().names, ["kaelynchar", "Kaelyn"]);
    // An account with no characters (or a name not on it) matches nothing, without a query.
    let before = s.fake.audit_queries().len();
    let j = s.send(Req::get("/api/admin/audit?account=99").cookie(&m)).await.json();
    assert!(j["items"].as_array().unwrap().is_empty());
    let j = s.send(Req::get("/api/admin/audit?account=1&who=Halvard").cookie(&m)).await.json();
    assert!(j["items"].as_array().unwrap().is_empty());
    assert_eq!(s.fake.audit_queries().len(), before);
}

#[tokio::test]
async fn searches_are_rate_limited() {
    let s = site_with_logs().await;
    let (m, _) = s.sign_in("halvard", "gm-password").await;
    for _ in 0..20 {
        assert_eq!(s.send(Req::get("/api/admin/audit").cookie(&m)).await.status, StatusCode::OK);
    }
    assert_eq!(s.send(Req::get("/api/admin/audit").cookie(&m)).await.status, StatusCode::TOO_MANY_REQUESTS);
    s.advance(61);
    assert_eq!(s.send(Req::get("/api/admin/audit").cookie(&m)).await.status, StatusCode::OK);
}
