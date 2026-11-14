//! The admin panel's player management: lists, bans, password and GM-level changes,
//! the role rules, the action log and the timed-ban lifter.

mod common;

use axum::http::StatusCode;
use common::*;
use ffxi_web::auth;
use ffxi_web::model::BanRecord;
use serde_json::{json, Value};

/// kaelyn (player), ravenna (player), halvard (moderator, gm 3), steven (owner, gm 5), moira (gm 4 moderator).
async fn site_with_people() -> Site {
    let s = site().await;
    s.fake.add_account(1, "kaelyn", "hunter22", 0);
    s.fake.add_account(2, "ravenna", "secret-pw-2", 0);
    s.fake.add_account(3, "halvard", "gm-password", 3);
    s.fake.add_account(4, "steven", "owner-pass", 5);
    s.fake.add_account(5, "moira", "gm4-password", 4);
    s.fake.add_account(6, "ownerb", "owner-pass-b", 5);
    s
}

struct Who {
    cookie: String,
    csrf: String,
}

async fn who(s: &Site, login: &str, pw: &str) -> Who {
    let (cookie, csrf) = s.sign_in(login, pw).await;
    Who { cookie, csrf }
}

fn get(w: &Who, path: &str) -> Req {
    Req::get(path).cookie(&w.cookie)
}
fn post(w: &Who, path: &str, body: Value) -> Req {
    Req::post(path, body).session(&w.cookie, &w.csrf)
}

async fn moderator(s: &Site) -> Who { who(s, "halvard", "gm-password").await }
async fn owner(s: &Site) -> Who { who(s, "steven", "owner-pass").await }

fn log_actions(j: &Value) -> Vec<(String, String)> {
    j["items"].as_array().unwrap().iter().map(|a| (a["action"].as_str().unwrap().to_string(), a["result"].as_str().unwrap().to_string())).collect()
}

// ---- who can read ----

#[tokio::test]
async fn only_staff_read_the_player_lists() {
    let s = site_with_people().await;
    let player = who(&s, "kaelyn", "hunter22").await;
    let m = moderator(&s).await;
    for path in ["/api/admin/accounts", "/api/admin/accounts/1", "/api/admin/characters", "/api/admin/characters/10", "/api/admin/characters/10/inventory", "/api/admin/bans"] {
        assert_eq!(s.send(Req::get(path)).await.status, StatusCode::UNAUTHORIZED, "visitor {path}");
        assert_eq!(s.send(get(&player, path)).await.status, StatusCode::FORBIDDEN, "player {path}");
        assert_eq!(s.send(get(&m, path)).await.status, StatusCode::OK, "moderator {path}");
    }
    assert_eq!(s.send(get(&m, "/api/admin/accounts/999")).await.status, StatusCode::NOT_FOUND);
    assert_eq!(s.send(get(&m, "/api/admin/characters/999")).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn account_search_sorts_filters_and_pages_on_the_server() {
    let s = site_with_people().await;
    let m = moderator(&s).await;
    let all = s.send(get(&m, "/api/admin/accounts?per=10")).await.json();
    assert_eq!(all["total"], 6);
    assert_eq!(all["items"][0]["login"], "halvard", "sorted by login");
    assert_eq!(all["counts"]["accounts"], 6);
    let desc = s.send(get(&m, "/api/admin/accounts?sort=id&dir=desc&per=10")).await.json();
    assert_eq!(desc["items"][0]["id"], 6);
    // A search matches the login or a character's name.
    let by_char = s.send(get(&m, "/api/admin/accounts?q=RAVENNACHAR")).await.json();
    assert_eq!(by_char["total"], 1);
    assert_eq!(by_char["items"][0]["login"], "ravenna");
    // Paging: ten per page is the smallest size, so move the page past the end.
    let page2 = s.send(get(&m, "/api/admin/accounts?per=10&page=2")).await.json();
    assert_eq!(page2["items"].as_array().unwrap().len(), 0);
    assert_eq!(page2["total"], 6);
    // Ban and online filters.
    s.fake.ban_directly(1, None);
    s.fake.set_online(20, true);
    assert_eq!(s.send(get(&m, "/api/admin/accounts?status=banned")).await.json()["items"][0]["login"], "kaelyn");
    assert_eq!(s.send(get(&m, "/api/admin/accounts?status=online")).await.json()["items"][0]["login"], "ravenna");
    // Unknown sort keys fall back to the default instead of reaching SQL.
    assert_eq!(s.send(get(&m, "/api/admin/accounts?sort=password;drop")).await.status, StatusCode::OK);
}

#[tokio::test]
async fn character_lists_and_details_work_for_any_account() {
    let s = site_with_people().await;
    let m = moderator(&s).await;
    s.fake.set_online(10, true);
    let online = s.send(get(&m, "/api/admin/characters?online=1")).await.json();
    assert_eq!(online["total"], 1);
    assert_eq!(online["items"][0]["name"], "kaelynchar");
    assert_eq!(online["items"][0]["login"], "kaelyn");
    let d = s.send(get(&m, "/api/admin/characters/20")).await.json();
    assert_eq!(d["character"]["name"], "ravennachar");
    assert_eq!(d["character"]["gil"], 1234);
    assert_eq!(d["character"]["accid"], 2);
    assert_eq!(d["can_act"], true);
    let high = s.send(get(&m, "/api/admin/characters/40")).await.json();
    assert_eq!(high["can_act"], false, "an owner's character is above a moderator");
    let inv = s.send(get(&m, "/api/admin/characters/20/inventory")).await.json();
    assert_eq!(inv["items"][0]["name"], "Hi-Potion");
}

// ---- ban and unban ----

#[tokio::test]
async fn banning_needs_a_reason_and_a_sensible_end() {
    let s = site_with_people().await;
    let m = moderator(&s).await;
    let url = "/api/admin/accounts/1/ban";
    assert_eq!(s.send(post(&m, url, json!({}))).await.status, StatusCode::BAD_REQUEST, "no reason");
    assert_eq!(s.send(post(&m, url, json!({ "reason": "   " }))).await.status, StatusCode::BAD_REQUEST, "blank reason");
    assert_eq!(s.send(post(&m, url, json!({ "reason": "x".repeat(501) }))).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(s.send(post(&m, url, json!({ "reason": "spam", "until": NOW - 5 }))).await.status, StatusCode::BAD_REQUEST, "already over");
    assert_eq!(s.send(post(&m, url, json!({ "reason": "spam", "until": NOW + 11 * 365 * 86400 }))).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(s.fake.account(1).status, 1, "nothing changed");
}

#[tokio::test]
async fn a_moderator_can_ban_and_unban_a_player_and_it_is_logged() {
    let s = site_with_people().await;
    let m = moderator(&s).await;
    let r = s.send(post(&m, "/api/admin/accounts/1/ban", json!({ "reason": "Spamming shouts", "until": NOW + 86400 }))).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let a = s.fake.account(1);
    assert_eq!(a.status & 1, 0, "status says banned");
    assert!(!a.trust_tokens, "trust tokens are dropped");
    assert_eq!(a.ban, Some(BanRecord { banned_at: Some(NOW), until: Some(NOW + 86400), reason: Some("Spamming shouts".into()) }));

    // The ban shows on the account, on the Bans page and in its history.
    let acct = s.send(get(&m, "/api/admin/accounts/1")).await.json();
    assert_eq!(acct["account"]["ban"]["reason"], "Spamming shouts");
    assert_eq!(acct["history"][0]["action"], "ban");
    assert_eq!(acct["history"][0]["actor_login"], "halvard");
    let bans = s.send(get(&m, "/api/admin/bans")).await.json();
    assert_eq!(bans["total"], 1);
    assert_eq!(bans["items"][0]["login"], "kaelyn");
    assert_eq!(bans["now"], NOW);
    // The banned player can't sign in.
    let again = s.send(Req::post("/api/auth/login", json!({ "login": "kaelyn", "password": "hunter22" }))).await;
    assert_eq!(again.status, StatusCode::UNAUTHORIZED);

    // Unbanning needs a reason too.
    assert_eq!(s.send(post(&m, "/api/admin/accounts/1/unban", json!({}))).await.status, StatusCode::BAD_REQUEST);
    let r = s.send(post(&m, "/api/admin/accounts/1/unban", json!({ "reason": "Apologised" }))).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(s.fake.account(1).status, 1);
    assert!(s.fake.account(1).ban.is_none());
    // Not banned any more: a second unban is a conflict, not a silent success.
    assert_eq!(s.send(post(&m, "/api/admin/accounts/1/unban", json!({ "reason": "again" }))).await.status, StatusCode::CONFLICT);

    let log = s.send(get(&m, "/api/admin/log")).await.json();
    let acts = log_actions(&log);
    assert!(acts.contains(&("ban".into(), "ok".into())) && acts.contains(&("unban".into(), "ok".into())), "{acts:?}");
    let row = log["items"].as_array().unwrap().iter().find(|a| a["action"] == "ban").unwrap();
    assert_eq!(row["reason"], "Spamming shouts");
    assert_eq!(row["target_name"], "kaelyn");
    assert!(row["ip"].is_null(), "a moderator is not shown addresses");
}

#[tokio::test]
async fn nobody_acts_on_an_equal_or_higher_level_and_refusals_are_logged() {
    let s = site_with_people().await;
    let m = moderator(&s).await; // gm 3
    let o = owner(&s).await; // gm 5
    // A moderator can't touch another moderator, a higher one or an owner.
    for target in [3, 5, 4] {
        let r = s.send(post(&m, &format!("/api/admin/accounts/{target}/ban"), json!({ "reason": "no" }))).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "target {target}");
        assert_eq!(s.fake.account(target).status, 1);
    }
    // An owner can't touch another owner but can touch a moderator.
    assert_eq!(s.send(post(&o, "/api/admin/accounts/6/ban", json!({ "reason": "no" }))).await.status, StatusCode::FORBIDDEN);
    assert_eq!(s.send(post(&o, "/api/admin/accounts/4/ban", json!({ "reason": "myself" }))).await.status, StatusCode::FORBIDDEN, "not even themselves");
    assert_eq!(s.send(post(&o, "/api/admin/accounts/3/ban", json!({ "reason": "ok" }))).await.status, StatusCode::OK);
    assert_eq!(s.send(post(&o, "/api/admin/accounts/3/unban", json!({ "reason": "ok" }))).await.status, StatusCode::OK);
    // The Account page says so beforehand (the ban above ended halvard's web sign-in).
    let m = moderator(&s).await;
    assert_eq!(s.send(get(&m, "/api/admin/accounts/5")).await.json()["can_act"], false);
    assert_eq!(s.send(get(&m, "/api/admin/accounts/2")).await.json()["can_act"], true);

    let log = s.send(get(&o, "/api/admin/log")).await.json();
    let refused = log_actions(&log).into_iter().filter(|(_, r)| r == "refused").count();
    assert_eq!(refused, 5, "every refusal is in the log");
}

#[tokio::test]
async fn the_database_guard_catches_a_level_that_changed_after_the_check() {
    use ffxi_web::game::Game;
    use ffxi_web::model::Acted;
    let s = site_with_people().await;
    // The check inside the write is what counts: at or above the guard, nothing changes.
    assert_eq!(s.fake.ban_account(3, None, "r", Some(3)).await.unwrap(), Acted::TooHigh);
    assert_eq!(s.fake.ban_account(2, None, "r", Some(3)).await.unwrap(), Acted::Done);
    assert_eq!(s.fake.ban_account(3, None, "r", None).await.unwrap(), Acted::Done, "the owner link has no limit");
}

#[tokio::test]
async fn the_owner_link_may_act_on_anyone() {
    let s = site_with_people().await;
    let r = s.send(Req::get(&format!("/auth/owner?t={}", wrapper_core::owner_link::sign(SECRET, NOW as u64).unwrap()))).await;
    let cookie = r.cookie().unwrap();
    let csrf = s.send(Req::get("/api/me").cookie(&cookie)).await.json()["csrf"].as_str().unwrap().to_string();
    let w = Who { cookie, csrf };
    assert_eq!(s.send(post(&w, "/api/admin/accounts/4/ban", json!({ "reason": "test" }))).await.status, StatusCode::OK);
    let log = s.send(get(&w, "/api/admin/log")).await.json();
    let row = log["items"].as_array().unwrap().iter().find(|a| a["action"] == "ban").unwrap();
    assert_eq!(row["actor_kind"], "owner");
    assert!(row["actor_login"].is_null());
    assert!(row["ip"].is_string(), "the owner sees addresses");
}

#[tokio::test]
async fn changes_to_players_still_need_the_csrf_token() {
    let s = site_with_people().await;
    let m = moderator(&s).await;
    let mut r = Req::post("/api/admin/accounts/1/ban", json!({ "reason": "x" }));
    r.cookie = Some(m.cookie.clone());
    assert_eq!(s.send(r).await.status, StatusCode::FORBIDDEN);
    assert_eq!(s.fake.account(1).status, 1);
}

// ---- timed bans ----

#[tokio::test]
async fn timed_bans_are_lifted_when_they_run_out_and_logged_as_the_sites_own_unban() {
    let s = site_with_people().await;
    let m = moderator(&s).await;
    s.send(post(&m, "/api/admin/accounts/1/ban", json!({ "reason": "cool off", "until": NOW + 3600 }))).await;
    s.send(post(&m, "/api/admin/accounts/2/ban", json!({ "reason": "forever" }))).await;
    assert_eq!(ffxi_web::routes::admin::lift_expired_bans(&s.state).await, 0, "not yet");
    s.advance(3600 + 1);
    let m2 = moderator(&s).await;
    assert_eq!(ffxi_web::routes::admin::lift_expired_bans(&s.state).await, 1);
    assert_eq!(s.fake.account(1).status, 1);
    assert!(s.fake.account(1).ban.is_none());
    assert_eq!(s.fake.account(2).status & 1, 0, "a ban with no end stays");
    assert_eq!(ffxi_web::routes::admin::lift_expired_bans(&s.state).await, 0, "once");
    let log = s.send(get(&m2, "/api/admin/log?action=unban")).await.json();
    let row = &log["items"][0];
    assert_eq!(row["actor_kind"], "system");
    assert_eq!(row["target_name"], "kaelyn");
    // The player can sign in again.
    assert_eq!(s.send(Req::post("/api/auth/login", json!({ "login": "kaelyn", "password": "hunter22" }))).await.status, StatusCode::OK);
}

// ---- password reset, two-step reset ----

#[tokio::test]
async fn only_an_owner_resets_passwords_and_the_new_one_works() {
    let s = site_with_people().await;
    let m = moderator(&s).await;
    let o = owner(&s).await;
    assert_eq!(s.send(post(&m, "/api/admin/accounts/1/password", json!({}))).await.status, StatusCode::FORBIDDEN);
    assert!(auth::check_password("hunter22", &s.fake.hash_of(1)) == auth::HashCheck::Match, "untouched");

    // A player signed in on the web is signed out by the reset.
    let player = who(&s, "kaelyn", "hunter22").await;
    assert_eq!(s.send(get(&player, "/api/me/account")).await.status, StatusCode::OK);

    // Made up by the site, shown once, never stored in the log.
    let r = s.send(post(&o, "/api/admin/accounts/1/password", json!({ "reason": "Forgot it" }))).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.header("cache-control").as_deref(), Some("no-store"));
    let pw = r.json()["password"].as_str().unwrap().to_string();
    assert!(pw.len() >= 8 && pw.len() <= 32, "{pw}");
    assert_eq!(auth::check_password(&pw, &s.fake.hash_of(1)), auth::HashCheck::Match);
    assert!(s.fake.hash_of(1).starts_with("$2a$"), "bcrypt, in LSB's form");
    assert_eq!(s.send(get(&player, "/api/me/account")).await.status, StatusCode::UNAUTHORIZED, "web sign-ins ended");
    s.sign_in("kaelyn", &pw).await;
    let log = s.send(get(&o, "/api/admin/log")).await;
    assert!(!log.text().contains(&pw), "the password is not in the log");
    assert!(log_actions(&log.json()).contains(&("password_reset".into(), "ok".into())));

    // The owner may type one instead; it is not echoed back.
    let r = s.send(post(&o, "/api/admin/accounts/2/password", json!({ "new_password": "typed-by-owner" }))).await;
    assert!(r.json()["password"].is_null());
    assert_eq!(auth::check_password("typed-by-owner", &s.fake.hash_of(2)), auth::HashCheck::Match);
    // Weak ones are refused, and so are targets at the owner's level.
    assert_eq!(s.send(post(&o, "/api/admin/accounts/2/password", json!({ "new_password": "short" }))).await.status, StatusCode::BAD_REQUEST);
    let before = s.fake.hash_of(6);
    assert_eq!(s.send(post(&o, "/api/admin/accounts/6/password", json!({}))).await.status, StatusCode::FORBIDDEN);
    assert_eq!(s.fake.hash_of(6), before);
}

#[tokio::test]
async fn only_an_owner_removes_two_step_and_needs_a_reason() {
    let s = site_with_people().await;
    s.fake.set_totp(1, "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
    let m = moderator(&s).await;
    let o = owner(&s).await;
    assert_eq!(s.send(post(&m, "/api/admin/accounts/1/totp-reset", json!({ "reason": "lost phone" }))).await.status, StatusCode::FORBIDDEN);
    assert_eq!(s.send(post(&o, "/api/admin/accounts/1/totp-reset", json!({}))).await.status, StatusCode::BAD_REQUEST);
    assert!(s.fake.account(1).totp.is_some());
    assert_eq!(s.send(post(&o, "/api/admin/accounts/1/totp-reset", json!({ "reason": "lost phone" }))).await.status, StatusCode::OK);
    assert!(s.fake.account(1).totp.is_none());
    assert!(!s.fake.account(1).trust_tokens);
    // Nothing to remove the second time.
    assert_eq!(s.send(post(&o, "/api/admin/accounts/1/totp-reset", json!({ "reason": "again" }))).await.status, StatusCode::CONFLICT);
}

// ---- GM level ----

#[tokio::test]
async fn only_an_owner_changes_gm_levels_within_the_rules() {
    let s = site_with_people().await;
    let m = moderator(&s).await;
    let o = owner(&s).await;
    let url = "/api/admin/characters/20/gm-level";
    assert_eq!(s.send(post(&m, url, json!({ "level": 3, "reason": "events" }))).await.status, StatusCode::FORBIDDEN);
    assert_eq!(s.send(post(&o, url, json!({ "level": 6, "reason": "events" }))).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(s.send(post(&o, url, json!({ "level": -1, "reason": "events" }))).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(s.send(post(&o, url, json!({ "level": 3 }))).await.status, StatusCode::BAD_REQUEST, "needs a reason");
    assert_eq!(s.send(post(&o, url, json!({ "level": 0, "reason": "same" }))).await.status, StatusCode::BAD_REQUEST, "no change");
    assert_eq!(s.fake.gm_of_char(20), 0);

    assert_eq!(s.send(post(&o, url, json!({ "level": 3, "reason": "Helps with events" }))).await.status, StatusCode::OK);
    assert_eq!(s.fake.gm_of_char(20), 3);
    // It shows in the history with before and after.
    let d = s.send(get(&o, "/api/admin/characters/20")).await.json();
    assert_eq!(d["history"][0]["action"], "gm_level");
    assert_eq!(d["history"][0]["detail"], "{\"after\":3,\"before\":0}");
    // Another owner's characters, and the owner's own, are out of reach.
    assert_eq!(s.send(post(&o, "/api/admin/characters/60/gm-level", json!({ "level": 0, "reason": "x" }))).await.status, StatusCode::FORBIDDEN);
    assert_eq!(s.send(post(&o, "/api/admin/characters/40/gm-level", json!({ "level": 0, "reason": "lock myself out" }))).await.status, StatusCode::FORBIDDEN);
    assert_eq!(s.fake.gm_of_char(40), 5);
    // Not while the character is in game.
    s.fake.set_online(20, true);
    assert_eq!(s.send(post(&o, url, json!({ "level": 0, "reason": "undo" }))).await.status, StatusCode::CONFLICT);
    assert_eq!(s.fake.gm_of_char(20), 3);
    assert_eq!(s.send(post(&o, "/api/admin/characters/999/gm-level", json!({ "level": 1, "reason": "x" }))).await.status, StatusCode::NOT_FOUND);
}

// ---- the admin log ----

#[tokio::test]
async fn the_log_filters_by_text_actor_action_and_age_and_pages() {
    let s = site_with_people().await;
    let m = moderator(&s).await;
    s.send(post(&m, "/api/admin/accounts/1/ban", json!({ "reason": "Spamming shouts" }))).await;
    s.advance(86400 * 3);
    let o = owner(&s).await;
    s.send(post(&o, "/api/admin/accounts/2/ban", json!({ "reason": "Gil selling" }))).await;
    s.send(post(&o, "/api/admin/accounts/2/unban", json!({ "reason": "Mistake" }))).await;
    let q = |p: &str| format!("/api/admin/log?{p}");
    let by_text = s.send(get(&o, &q("q=shouts"))).await.json();
    assert_eq!(by_text["total"], 1);
    assert_eq!(by_text["items"][0]["target_name"], "kaelyn");
    let by_target = s.send(get(&o, &q("q=ravenna&action=ban"))).await.json();
    assert_eq!(by_target["total"], 1);
    let by_actor = s.send(get(&o, &q("actor=halvard&action=ban"))).await.json();
    assert_eq!(by_actor["total"], 1);
    assert!(by_actor["actors"].as_array().unwrap().iter().any(|a| a == "steven"));
    assert!(by_actor["actions"].as_array().unwrap().iter().any(|a| a == "unban"));
    let recent = s.send(get(&o, &q("days=1&action=ban"))).await.json();
    assert_eq!(recent["total"], 1, "the three-day-old ban is out");
    assert_eq!(recent["items"][0]["target_name"], "ravenna");
    // % and _ in a search are literal.
    assert_eq!(s.send(get(&o, &q("q=%25"))).await.json()["total"], 0);
    // Paging: newest first.
    let p1 = s.send(get(&o, &q("per=10&days=0&action=ban"))).await.json();
    assert_eq!(p1["total"], 2);
    assert_eq!(p1["items"][0]["target_name"], "ravenna");
}

#[tokio::test]
async fn the_character_page_history_lists_what_was_done_to_it_and_its_account() {
    let s = site_with_people().await;
    let o = owner(&s).await;
    s.send(post(&o, "/api/admin/accounts/2/ban", json!({ "reason": "Gil selling" }))).await;
    s.send(post(&o, "/api/admin/characters/20/gm-level", json!({ "level": 1, "reason": "Tester" }))).await;
    let h = s.send(get(&o, "/api/admin/characters/20")).await.json()["history"].clone();
    let actions: Vec<&str> = h.as_array().unwrap().iter().map(|a| a["action"].as_str().unwrap()).collect();
    assert_eq!(actions, ["gm_level", "ban"]);
}
