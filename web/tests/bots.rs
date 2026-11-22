//! A player's AI adventurers switch (LSB's `BotsOptOut` char var) on My account.

mod common;

use axum::http::StatusCode;
use common::*;
use ffxi_web::config::Config;
use serde_json::json;
use std::path::{Path, PathBuf};

/// A settings folder whose zz_app.lua sets `PLAYER_BOTS_ENABLED`, as the desktop app writes it.
fn settings_dir(name: &str, enabled: Option<bool>) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("bots-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    if let Some(on) = enabled {
        std::fs::write(dir.join("zz_app.lua"), format!("xi.settings.map = xi.settings.map or {{}}\nxi.settings.map.PLAYER_BOTS_ENABLED = {on}\n")).unwrap();
    }
    dir
}

async fn players(dir: PathBuf) -> Site {
    let mut cfg = Config::for_tests();
    cfg.admin_secret = SECRET.into();
    cfg.lsb_settings = Some(dir);
    let s = site_with(cfg).await;
    s.fake.add_account(1, "kaelyn", "hunter22", 0);
    s.fake.add_account(2, "ravenna", "secret-pw-2", 0);
    s
}

#[tokio::test]
async fn a_player_hides_and_shows_bots_for_their_own_character() {
    let s = players(settings_dir("own", Some(true))).await;
    let (cookie, csrf) = s.sign_in("kaelyn", "hunter22").await;

    let r = s.send(Req::get("/api/me/characters/10/bots").cookie(&cookie)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json(), json!({ "enabled": true, "hidden": false }));

    let r = s.send(Req::post("/api/me/characters/10/bots", json!({ "hidden": true })).session(&cookie, &csrf)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(s.fake.0.lock().unwrap().bots_hidden, vec![10]);
    assert_eq!(s.send(Req::get("/api/me/characters/10/bots").cookie(&cookie)).await.json()["hidden"], true);

    let r = s.send(Req::post("/api/me/characters/10/bots", json!({ "hidden": false })).session(&cookie, &csrf)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(s.fake.0.lock().unwrap().bots_hidden.is_empty());
}

#[tokio::test]
async fn another_accounts_character_answers_like_a_missing_one() {
    let s = players(settings_dir("other", Some(true))).await;
    let (cookie, csrf) = s.sign_in("kaelyn", "hunter22").await;

    let theirs = s.send(Req::post("/api/me/characters/20/bots", json!({ "hidden": true })).session(&cookie, &csrf)).await;
    let missing = s.send(Req::post("/api/me/characters/99999/bots", json!({ "hidden": true })).session(&cookie, &csrf)).await;
    assert_eq!(theirs.status, StatusCode::NOT_FOUND);
    assert_eq!(theirs.text(), missing.text());
    assert_eq!(s.send(Req::get("/api/me/characters/20/bots").cookie(&cookie)).await.status, StatusCode::NOT_FOUND);
    let d = s.fake.0.lock().unwrap();
    assert!(d.bots_hidden.is_empty());
    assert!(d.char_queries.iter().all(|(acc, _)| *acc == 1), "{:?}", d.char_queries);
}

#[tokio::test]
async fn changing_it_needs_a_signed_in_request_with_csrf() {
    let s = players(settings_dir("csrf", Some(true))).await;
    let (cookie, _) = s.sign_in("kaelyn", "hunter22").await;
    let anon = s.send(Req::post("/api/me/characters/10/bots", json!({ "hidden": true }))).await;
    assert_eq!(anon.status, StatusCode::UNAUTHORIZED);
    let no_csrf = s.send(Req::post("/api/me/characters/10/bots", json!({ "hidden": true })).cookie(&cookie)).await;
    assert!(no_csrf.status.is_client_error(), "{}", no_csrf.status);
    assert!(s.fake.0.lock().unwrap().bots_hidden.is_empty());
    let bad = s.send(Req::post("/api/me/characters/10/bots", json!({ "hidden": "yes" })).session(&cookie, &s.sign_in("kaelyn", "hunter22").await.1)).await;
    assert!(bad.status.is_client_error(), "{}", bad.status);
}

#[tokio::test]
async fn the_switch_is_only_offered_while_the_server_has_bots_on() {
    for (name, setting, enabled) in [("unset", None, false), ("off", Some(false), false), ("on", Some(true), true)] {
        let s = players(settings_dir(name, setting)).await;
        let (cookie, _) = s.sign_in("kaelyn", "hunter22").await;
        let r = s.send(Req::get("/api/me/characters/10/bots").cookie(&cookie)).await;
        assert_eq!(r.json()["enabled"], enabled, "{name}");
    }
}

#[test]
fn the_bots_setting_is_read_like_lsb_loads_it() {
    use ffxi_web::lsb_settings::last_flag;
    let key = "PLAYER_BOTS_ENABLED";
    assert_eq!(last_flag("xi.settings.map.PLAYER_BOTS_ENABLED = true\n", key), Some(true));
    assert_eq!(last_flag("x = {\n    PLAYER_BOTS_ENABLED = false,\n}", key), Some(false));
    assert_eq!(last_flag("PLAYER_BOTS_ENABLED = true\nPLAYER_BOTS_ENABLED = false", key), Some(false));
    // Not comments, strings, comparisons, other names or expressions.
    assert_eq!(last_flag("-- PLAYER_BOTS_ENABLED = true", key), None);
    assert_eq!(last_flag("NOTE = 'PLAYER_BOTS_ENABLED = true'", key), None);
    assert_eq!(last_flag("if PLAYER_BOTS_ENABLED == true then end", key), None);
    assert_eq!(last_flag("MY_PLAYER_BOTS_ENABLED = true", key), None);
    assert_eq!(last_flag("PLAYER_BOTS_ENABLED = trueish", key), None);
    assert_eq!(last_flag("PLAYER_BOTS_ENABLED = 1", key), None);
}

#[tokio::test]
async fn the_rules_and_online_list_tell_visitors_about_bots_only_while_they_are_on() {
    for (name, setting) in [("public-off", Some(false)), ("public-on", Some(true))] {
        let on = setting == Some(true);
        let s = players(settings_dir(name, setting)).await;
        let rules = s.send(Req::get("/api/public/content/rules")).await.json();
        assert_eq!(rules["bots_html"].is_string(), on, "{name}: {rules}");
        if on {
            let html = rules["bots_html"].as_str().unwrap();
            assert!(html.contains("<strong>AI adventurers.</strong>"), "{html}");
            assert!(html.contains("<code>!bots off</code>") && html.contains("<a href=\"/me\""), "{html}");
        }
        assert_eq!(s.send(Req::get("/api/public/online")).await.json()["bots"], on, "{name}");
    }
}

#[tokio::test]
async fn the_owner_can_reword_the_bots_notice() {
    let s = players(settings_dir("edited", Some(true))).await;
    s.state.store.content_set("bots", "Our *helpers* are bots.", "owner", NOW).await.unwrap();
    let rules = s.send(Req::get("/api/public/content/rules")).await.json();
    assert_eq!(rules["bots_html"].as_str().unwrap().trim(), "<p>Our <em>helpers</em> are bots.</p>");
}
