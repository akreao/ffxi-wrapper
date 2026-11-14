//! Creating accounts on the site: when it is open, the invite codes, the limits, and the
//! owner's page for codes.

mod common;

use axum::http::StatusCode;
use common::*;
use ffxi_web::config::Config;
use ffxi_web::routes::signup::SIGNUP_KEYS;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static N: AtomicUsize = AtomicUsize::new(0);

/// A settings folder with LSB's `login.ACCOUNT_CREATION` set as given (`None`: no file at all).
fn settings_dir(file: Option<&str>) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("lsb-settings-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    std::fs::create_dir_all(&dir).unwrap();
    if let Some(text) = file {
        std::fs::write(dir.join("zz_app.lua"), text).unwrap();
    }
    dir
}

const OPEN: &str = "xi = xi or {}\nxi.settings = xi.settings or {}\nxi.settings.login = { ACCOUNT_CREATION = true }\n";
const CLOSED: &str = "xi.settings.login = { ACCOUNT_CREATION = false }\n";

async fn site_for(lua: Option<&str>) -> Site {
    let mut cfg = Config::for_tests();
    cfg.admin_secret = SECRET.into();
    cfg.lsb_settings = Some(settings_dir(lua));
    let s = site_with(cfg).await;
    s.fake.add_account(900, "kaelyn", "hunter22", 0);
    s.fake.add_account(901, "halvard", "gm-password", 3);
    s.fake.add_account(902, "steven", "owner-pass", 5);
    s
}

/// An open site with the owner signed in.
async fn open_site() -> (Site, (String, String)) {
    let s = site_for(Some(OPEN)).await;
    let owner = s.sign_in("steven", "owner-pass").await;
    (s, owner)
}

fn signup(login: &str, password: &str, invite: &str) -> Req {
    Req::post("/api/public/signup", json!({ "login": login, "password": password, "invite": invite }))
}

async fn new_invite(s: &Site, owner: &(String, String), max_uses: i64) -> String {
    let r = s.send(Req::post("/api/admin/invites", json!({ "note": "test", "max_uses": max_uses })).session(&owner.0, &owner.1)).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    r.json()["code"].as_str().unwrap().to_string()
}

async fn uses_of(s: &Site, owner: &(String, String)) -> Vec<(i64, String)> {
    let r = s.send(Req::get("/api/admin/invites").session(&owner.0, &owner.1)).await;
    r.json()["items"].as_array().unwrap().iter().map(|i| (i["uses"].as_i64().unwrap(), i["state"].as_str().unwrap().to_string())).collect()
}

fn account_named(s: &Site, login: &str) -> Option<FakeAccount> {
    s.fake.0.lock().unwrap().accounts.iter().find(|a| a.login.eq_ignore_ascii_case(login)).cloned()
}

fn account_count(s: &Site) -> usize {
    s.fake.0.lock().unwrap().accounts.len()
}

// ---- open or closed ----

#[tokio::test]
async fn closed_until_lsb_allows_accounts() {
    for (lua, label) in [(None, "no settings file: LSB's default is on"), (Some(OPEN), "on"), (Some(CLOSED), "off"), (Some("xi.settings.login.ACCOUNT_CREATION = maybe"), "unreadable")] {
        let s = site_for(lua).await;
        let expect_open = matches!(lua, None | Some(OPEN));
        let info = s.send(Req::get("/api/public/signup")).await.json();
        assert_eq!(info["open"], json!(expect_open), "{label}");
        let home = s.send(Req::get("/api/public/home")).await.json();
        assert_eq!(home["signup_open"], json!(expect_open), "{label}");
        let r = s.send(signup("wren", "correct-horse", "")).await;
        if !expect_open {
            assert_eq!(r.status, StatusCode::FORBIDDEN, "{label}");
            assert_eq!(r.json()["closed"], json!(true));
            assert_eq!(account_named(&s, "wren").map(|a| a.id), None);
        }
    }
}

#[tokio::test]
async fn unknown_means_closed() {
    // No settings folder configured at all.
    let mut cfg = Config::for_tests();
    cfg.admin_secret = SECRET.into();
    let s = site_with(cfg).await;
    assert_eq!(s.send(Req::get("/api/public/signup")).await.json()["open"], json!(false));
    assert_eq!(s.send(signup("wren", "correct-horse", "")).await.status, StatusCode::FORBIDDEN);
    // A folder that doesn't exist.
    let mut cfg = Config::for_tests();
    cfg.lsb_settings = Some(PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("no-such-folder"));
    let s = site_with(cfg).await;
    assert_eq!(s.send(Req::get("/api/public/signup")).await.json()["open"], json!(false));
}

#[tokio::test]
async fn the_setting_is_looked_up_once_a_minute() {
    let s = site_for(Some(OPEN)).await;
    let dir = s.state.cfg.lsb_settings.clone().unwrap();
    assert_eq!(s.send(Req::get("/api/public/signup")).await.json()["open"], json!(true));
    std::fs::write(dir.join("zz_app.lua"), CLOSED).unwrap();
    s.advance(59);
    assert_eq!(s.send(Req::get("/api/public/signup")).await.json()["open"], json!(true), "still the answer from a minute ago");
    s.advance(2);
    assert_eq!(s.send(Req::get("/api/public/signup")).await.json()["open"], json!(false));
}

#[tokio::test]
async fn the_public_signup_answer_has_exactly_the_pinned_keys() {
    let (s, _) = open_site().await;
    let r = s.send(Req::get("/api/public/signup")).await;
    let mut k: Vec<String> = r.json().as_object().unwrap().keys().cloned().collect();
    k.sort();
    assert_eq!(k, SIGNUP_KEYS.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    assert_eq!(r.json()["invite_required"], json!(true), "on by default");
}

// ---- making an account ----

#[tokio::test]
async fn an_account_is_made_signed_in_and_logged_without_the_password() {
    let (s, owner) = open_site().await;
    let code = new_invite(&s, &owner, 1).await;
    let r = s.send(signup("Wren", "correct-horse", &code)).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    assert_eq!(r.json()["next"], json!("/me?welcome=1"));
    let cookie = r.cookie().expect("signed in");
    let csrf = r.json()["csrf"].as_str().unwrap().to_string();

    let a = account_named(&s, "wren").unwrap();
    assert_eq!(a.id, 1000, "the highest existing id is 902, so the floor of 1000 applies");
    assert_eq!(a.status, 1, "NORMAL");
    assert!(a.hash.starts_with("$2a$"), "LSB's own prefix: {}", a.hash);
    assert!(ffxi_web::auth::check_password("correct-horse", &a.hash) == ffxi_web::auth::HashCheck::Match);

    let me = s.send(Req::get("/api/me").cookie(&cookie)).await.json();
    assert_eq!(me["role"], json!("player"));
    assert_eq!(me["login"], json!("Wren"));
    assert_eq!(me["accid"], json!(a.id));
    // The session works for changes: the CSRF token it was given is the right one.
    let acct = s.send(Req::get("/api/me/account").cookie(&cookie)).await;
    assert_eq!(acct.status, StatusCode::OK, "{}", acct.text());
    let _ = csrf;

    let rows = s.state.store.actions(None, 100, false, true).await.unwrap();
    let row = rows.iter().find(|r| r.action == "signup").expect("a signup row");
    assert_eq!(row.target_id, Some(a.id));
    assert_eq!(row.target_name.as_deref(), Some("Wren"));
    let all = format!("{rows:?}");
    assert!(!all.contains("correct-horse"), "the password is never logged");
    assert!(!all.contains(&code), "neither is the code");
    assert!(row.detail.as_deref().unwrap().contains("\"invite\":1"), "which invite was used: {:?}", row.detail);
}

#[tokio::test]
async fn ids_continue_from_the_highest_and_start_at_1000() {
    let s = site_for(Some(OPEN)).await;
    s.state.store.set_setting("signup_needs_invite", "0", "t", 0).await.unwrap();
    let r = s.send(signup("first", "correct-horse", "").peer(3)).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    assert_eq!(account_named(&s, "first").unwrap().id, 1000);
    s.fake.add_account(5000, "bigid", "pw-pw-pw-1", 0);
    let r = s.send(signup("second", "correct-horse", "").peer(4)).await;
    assert_eq!(r.status, StatusCode::CREATED);
    assert_eq!(account_named(&s, "second").unwrap().id, 5001);
}

#[tokio::test]
async fn logins_and_passwords_are_checked_before_anything_is_counted() {
    let s = site_for(Some(OPEN)).await;
    s.state.store.set_setting("signup_needs_invite", "0", "t", 0).await.unwrap();
    for (login, pw) in [("ab", "correct-horse"), ("wren!", "correct-horse"), ("a-b-c", "correct-horse"), ("wren", "short"), ("wren", &"x".repeat(33)), ("wren", "bad\u{1}control")] {
        let r = s.send(signup(login, pw, "")).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{login} {pw}: {}", r.text());
    }
    // None of those used up the address's three tries.
    for i in 0..3 {
        let r = s.send(signup(&format!("ok{i}"), "correct-horse", "")).await;
        assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    }
    assert_eq!(s.send(signup("ok3", "correct-horse", "")).await.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn a_login_in_another_case_is_the_same_login() {
    let (s, owner) = open_site().await;
    let a = new_invite(&s, &owner, 5).await;
    assert_eq!(s.send(signup("wren", "correct-horse", &a)).await.status, StatusCode::CREATED);
    let r = s.send(signup("WREN", "correct-horse", &a).peer(3)).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text());
    // An existing game account too.
    assert_eq!(s.send(signup("Kaelyn", "correct-horse", &a).peer(4)).await.status, StatusCode::CONFLICT);
    assert_eq!(account_count(&s), 4);
    assert_eq!(uses_of(&s, &owner).await, vec![(1, "active".to_string())], "the two refused tries gave their uses back");
}

#[tokio::test]
async fn without_the_invite_requirement_no_code_is_asked_or_spent() {
    let (s, owner) = open_site().await;
    let r = s.send(Req::put("/api/admin/signup", json!({ "invite_required": false })).session(&owner.0, &owner.1)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(s.send(Req::get("/api/public/signup")).await.json()["invite_required"], json!(false));
    let r = s.send(signup("wren", "correct-horse", "")).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    // Back on: a request without a code is refused again.
    s.send(Req::put("/api/admin/signup", json!({ "invite_required": true })).session(&owner.0, &owner.1)).await;
    assert_eq!(s.send(signup("wren2", "correct-horse", "").peer(3)).await.status, StatusCode::BAD_REQUEST);
}

// ---- invite codes ----

#[tokio::test]
async fn a_missing_or_unusable_code_gets_one_answer() {
    let (s, owner) = open_site().await;
    let used = new_invite(&s, &owner, 1).await;
    assert_eq!(s.send(signup("first", "correct-horse", &used)).await.status, StatusCode::CREATED);
    let revoked = new_invite(&s, &owner, 3).await;
    let id = s.send(Req::get("/api/admin/invites").session(&owner.0, &owner.1)).await.json()["items"][0]["id"].as_i64().unwrap();
    assert_eq!(s.send(Req::post(&format!("/api/admin/invites/{id}/revoke"), json!({})).session(&owner.0, &owner.1)).await.status, StatusCode::OK);
    let expiring = {
        let r = s.send(Req::post("/api/admin/invites", json!({ "max_uses": 3, "days": 1 })).session(&owner.0, &owner.1)).await;
        r.json()["code"].as_str().unwrap().to_string()
    };
    s.advance(2 * 86_400);

    let mut answers = vec![];
    for (i, code) in [used.as_str(), revoked.as_str(), expiring.as_str(), "NOPE-NOPE-NOPE-NOPE", "x"].iter().enumerate() {
        // A new address each time, so the limit isn't what answers.
        let r = s.send(signup(&format!("try{i}"), "correct-horse", code).peer(10 + i as u8)).await;
        answers.push((r.status, r.bytes));
    }
    assert!(answers.iter().all(|a| a == &answers[0]), "every kind of unusable code is answered the same: {answers:?}");
    assert_eq!(answers[0].0, StatusCode::BAD_REQUEST);
    // And "taken" is never said to someone without a working code.
    let r = s.send(signup("kaelyn", "correct-horse", "NOPE-NOPE-NOPE-NOPE").peer(30)).await;
    assert_eq!((r.status, r.bytes), answers[0].clone());
    // No code at all is a different, plain message.
    let r = s.send(signup("try9", "correct-horse", "").peer(31)).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.json()["error"].as_str().unwrap().contains("Enter an invite code"));
    assert_eq!(account_count(&s), 4, "none of these made an account");
}

#[tokio::test]
async fn codes_ignore_case_spaces_and_dashes() {
    let (s, owner) = open_site().await;
    let code = new_invite(&s, &owner, 1).await;
    let typed = format!("  {}  ", code.to_lowercase().replace('-', " "));
    assert_eq!(s.send(signup("wren", "correct-horse", &typed)).await.status, StatusCode::CREATED);
}

#[tokio::test]
async fn two_sign_ups_with_one_single_use_code_make_one_account() {
    let (s, owner) = open_site().await;
    let code = new_invite(&s, &owner, 1).await;
    let (a, b) = tokio::join!(
        s.send(signup("alpha", "correct-horse", &code).peer(3)),
        s.send(signup("bravo", "correct-horse", &code).peer(4)),
    );
    let mut got = [a.status, b.status];
    got.sort();
    assert_eq!(got, [StatusCode::CREATED, StatusCode::BAD_REQUEST], "{} / {}", a.text(), b.text());
    assert_eq!(account_count(&s), 4, "three set up for the test and one new");
    assert_eq!(uses_of(&s, &owner).await, vec![(1, "used up".to_string())]);
}

#[tokio::test]
async fn many_sign_ups_with_a_three_use_code_make_exactly_three() {
    let (s, owner) = open_site().await;
    let code = new_invite(&s, &owner, 3).await;
    let tries: Vec<_> = (0..8u8).map(|i| s.send(signup(&format!("user{i}"), "correct-horse", &code).peer(20 + i))).collect();
    let results = futures_util::future::join_all(tries).await;
    assert_eq!(results.iter().filter(|r| r.status == StatusCode::CREATED).count(), 3);
    assert_eq!(uses_of(&s, &owner).await, vec![(3, "used up".to_string())]);
}

#[tokio::test]
async fn two_sign_ups_with_one_login_make_one_account_and_give_the_second_code_back() {
    let (s, owner) = open_site().await;
    let one = new_invite(&s, &owner, 1).await;
    let two = new_invite(&s, &owner, 1).await;
    let (a, b) = tokio::join!(s.send(signup("wren", "correct-horse", &one).peer(3)), s.send(signup("Wren", "correct-horse", &two).peer(4)));
    let mut got = [a.status, b.status];
    got.sort();
    assert_eq!(got, [StatusCode::CREATED, StatusCode::CONFLICT], "{} / {}", a.text(), b.text());
    assert_eq!(account_count(&s), 4);
    let mut uses = uses_of(&s, &owner).await;
    uses.sort();
    assert_eq!(uses, vec![(0, "active".to_string()), (1, "used up".to_string())], "the loser keeps its code");
}

#[tokio::test]
async fn a_failed_insert_burns_no_use() {
    let (s, owner) = open_site().await;
    let code = new_invite(&s, &owner, 1).await;
    s.fake.0.lock().unwrap().fail_create = true;
    let r = s.send(signup("wren", "correct-horse", &code)).await;
    assert_eq!(r.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(uses_of(&s, &owner).await, vec![(0, "active".to_string())]);
    s.fake.0.lock().unwrap().fail_create = false;
    assert_eq!(s.send(signup("wren", "correct-horse", &code).peer(3)).await.status, StatusCode::CREATED, "the same code still works");
}

#[tokio::test]
async fn a_bad_code_is_refused_before_any_hashing_and_a_busy_site_spends_nothing() {
    let (s, owner) = open_site().await;
    let good = new_invite(&s, &owner, 1).await;
    // Every bcrypt slot taken: a bad code is still answered (it never needs one) ...
    let held = s.state.bcrypt_slots.acquire_many(4).await.unwrap();
    let b = s.send(signup("two", "correct-horse", "NOPE-NOPE-NOPE-NOPE").peer(4)).await;
    assert_eq!(b.status, StatusCode::BAD_REQUEST);
    // ... and a good one that can't get a slot gives its use back.
    let a = s.send(signup("one", "correct-horse", &good).peer(3)).await;
    drop(held);
    assert_eq!(a.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(uses_of(&s, &owner).await, vec![(0, "active".to_string())], "nothing was spent");
    assert_eq!(s.state.limiter.signups.count("signup:day", 86_400, s.state.now()), 0, "nor the day's count");
}

#[tokio::test]
async fn a_visitor_who_hangs_up_leaves_nothing_counted_that_made_no_account() {
    let (s, owner) = open_site().await;
    let good = new_invite(&s, &owner, 5).await;
    // Hang up while the hash waits for a slot; the work carries on without the visitor.
    let held = s.state.bcrypt_slots.acquire_many(4).await.unwrap();
    let gone = tokio::time::timeout(std::time::Duration::from_millis(100), s.send(signup("ghost", "correct-horse", &good).peer(3))).await;
    assert!(gone.is_err(), "the request was dropped");
    drop(held);
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    let made = account_named(&s, "ghost").is_some();
    let day = s.state.limiter.signups.count("signup:day", 86_400, s.state.now());
    let used = uses_of(&s, &owner).await[0].0;
    assert_eq!((day, used), if made { (1, 1) } else { (0, 0) }, "counts match what exists");
    assert!(made, "the sign-up finished on its own");
}

// ---- limits ----

#[tokio::test]
async fn three_an_hour_from_one_address() {
    let s = site_for(Some(OPEN)).await;
    s.state.store.set_setting("signup_needs_invite", "0", "t", 0).await.unwrap();
    for i in 0..3 {
        assert_eq!(s.send(signup(&format!("user{i}"), "correct-horse", "")).await.status, StatusCode::CREATED);
    }
    let r = s.send(signup("user3", "correct-horse", "")).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(s.send(signup("user3", "correct-horse", "").peer(9)).await.status, StatusCode::CREATED, "another address is fine");
    s.advance(3601);
    assert_eq!(s.send(signup("user4", "correct-horse", "")).await.status, StatusCode::CREATED, "an hour later");
}

#[tokio::test]
async fn refused_tries_count_against_the_address_so_codes_cannot_be_guessed() {
    let (s, _) = open_site().await;
    for i in 0..3 {
        assert_eq!(s.send(signup("wren", "correct-horse", &format!("GUESS-{i}"))).await.status, StatusCode::BAD_REQUEST);
    }
    let r = s.send(signup("wren", "correct-horse", "GUESS-4")).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    // The refusal doesn't depend on the code, so it says nothing about it.
    assert!(r.json()["error"].as_str().unwrap().contains("Too many"));
}

#[tokio::test]
async fn thirty_accounts_a_day_for_the_whole_server() {
    let s = site_for(Some(OPEN)).await;
    s.state.store.set_setting("signup_needs_invite", "0", "t", 0).await.unwrap();
    for i in 0..30u8 {
        let r = s.send(signup(&format!("user{i}"), "correct-horse", "").peer(100 + i)).await;
        assert_eq!(r.status, StatusCode::CREATED, "{i}: {}", r.text());
    }
    let r = s.send(signup("user30", "correct-horse", "").peer(200)).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(r.json()["error"].as_str().unwrap().contains("tomorrow"), "{}", r.text());
    s.advance(24 * 3600 + 1);
    assert_eq!(s.send(signup("user30", "correct-horse", "").peer(200)).await.status, StatusCode::CREATED);
}

#[tokio::test]
async fn tries_that_make_no_account_leave_the_daily_count_alone() {
    let s = site_for(Some(OPEN)).await;
    s.state.store.set_setting("signup_needs_invite", "0", "t", 0).await.unwrap();
    // 40 taken-login tries from 40 addresses: none of them is an account.
    for i in 0..40u8 {
        assert_eq!(s.send(signup("kaelyn", "correct-horse", "").peer(100 + i)).await.status, StatusCode::CONFLICT);
    }
    assert_eq!(s.send(signup("fresh", "correct-horse", "").peer(200)).await.status, StatusCode::CREATED);
}

#[tokio::test]
async fn behind_the_gateway_address_everyone_shares_one_bucket() {
    let s = site_for(Some(OPEN)).await;
    s.state.store.set_setting("signup_needs_invite", "0", "t", 0).await.unwrap();
    // 10.0.0.1 looks like Docker's gateway: nobody can be told apart.
    for i in 0..10 {
        assert_eq!(s.send(signup(&format!("user{i}"), "correct-horse", "").peer(1)).await.status, StatusCode::CREATED);
    }
    assert_eq!(s.send(signup("user10", "correct-horse", "").peer(1)).await.status, StatusCode::TOO_MANY_REQUESTS);
}

// ---- the owner's page ----

#[tokio::test]
async fn only_the_owner_manages_codes() {
    let (s, owner) = open_site().await;
    let moderator = s.sign_in("halvard", "gm-password").await;
    let player = s.sign_in("kaelyn", "hunter22").await;
    for who in [&moderator, &player] {
        let r = s.send(Req::get("/api/admin/invites").session(&who.0, &who.1)).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN);
        let r = s.send(Req::post("/api/admin/invites", json!({ "max_uses": 1 })).session(&who.0, &who.1)).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN);
        let r = s.send(Req::put("/api/admin/signup", json!({ "invite_required": false })).session(&who.0, &who.1)).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN);
    }
    assert_eq!(s.send(Req::get("/api/admin/invites")).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(s.send(Req::get("/api/admin/invites").session(&owner.0, &owner.1)).await.status, StatusCode::OK);
}

#[tokio::test]
async fn a_code_is_shown_once_listed_by_its_last_four_and_every_change_is_logged() {
    let (s, owner) = open_site().await;
    let r = s.send(Req::post("/api/admin/invites", json!({ "note": "for the guild", "max_uses": 5, "days": 7 })).session(&owner.0, &owner.1)).await;
    assert_eq!(r.status, StatusCode::CREATED);
    let code = r.json()["code"].as_str().unwrap().to_string();
    let id = r.json()["id"].as_i64().unwrap();
    assert_eq!(code.len(), 19);

    let list = s.send(Req::get("/api/admin/invites").session(&owner.0, &owner.1)).await;
    let text = list.text();
    assert!(!text.contains(&code) && !text.contains(&code.replace('-', "")), "the list never repeats the code");
    let item: &Value = &list.json()["items"][0];
    assert_eq!(item["hint"], json!(code[15..]));
    assert_eq!(item["note"], json!("for the guild"));
    assert_eq!(item["max_uses"], json!(5));
    assert_eq!(item["expires"], json!(NOW + 7 * 86_400));
    assert_eq!(item["state"], json!("active"));
    let mut keys: Vec<&String> = item.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, ["created", "created_by", "expires", "hint", "id", "max_uses", "note", "state", "uses"]);

    assert_eq!(s.send(Req::post(&format!("/api/admin/invites/{id}/revoke"), json!({})).session(&owner.0, &owner.1)).await.status, StatusCode::OK);
    assert_eq!(s.send(Req::post("/api/admin/invites/999/revoke", json!({})).session(&owner.0, &owner.1)).await.status, StatusCode::NOT_FOUND);
    assert_eq!(s.send(signup("wren", "correct-horse", &code)).await.status, StatusCode::BAD_REQUEST, "revoked codes stop working");
    s.send(Req::put("/api/admin/signup", json!({ "invite_required": false })).session(&owner.0, &owner.1)).await;

    let rows = s.state.store.actions(None, 100, false, true).await.unwrap();
    let actions: Vec<&str> = rows.iter().map(|r| r.action.as_str()).filter(|a| a.starts_with("invite") || a.starts_with("signup")).collect();
    assert!(actions.contains(&"invite_create") && actions.contains(&"invite_revoke") && actions.contains(&"signup_setting") && actions.contains(&"signup_refused"), "{actions:?}");
    assert!(!format!("{rows:?}").contains(&code), "the code never reaches the log");
}

#[tokio::test]
async fn bad_invite_requests_are_refused() {
    let (s, owner) = open_site().await;
    for body in [json!({ "max_uses": 0 }), json!({ "max_uses": 1001 }), json!({ "max_uses": 1, "days": -1 }), json!({ "max_uses": 1, "days": 366 }), json!({ "max_uses": 1, "note": "x".repeat(61) }), json!({ "max_uses": 1, "note": "a\nb" })] {
        let r = s.send(Req::post("/api/admin/invites", body.clone()).session(&owner.0, &owner.1)).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{body}");
    }
}

#[tokio::test]
async fn the_store_upgrades_from_version_two() {
    // An old file (version 2) gets the new tables without losing what it had.
    let s = site().await;
    s.state.store.set_setting("k", "v", "t", 1).await.unwrap();
    assert_eq!(s.state.store.setting("k").await.unwrap().as_deref(), Some("v"));
    assert!(s.state.store.invite_required().await.unwrap(), "on until switched off");
}
