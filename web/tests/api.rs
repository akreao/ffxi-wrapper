//! Router tests: the whole site over an in-memory fake of the game database.

mod common;

use axum::http::StatusCode;
use common::*;
use ffxi_web::auth;
use serde_json::json;
use totp_rs::{Algorithm, Builder, Secret};

const TOTP_SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

async fn with_players() -> Site {
    let s = site().await;
    s.fake.add_account(1, "kaelyn", "hunter22", 0);
    s.fake.add_account(2, "ravenna", "secret-pw-2", 0);
    s.fake.add_account(3, "halvard", "gm-password", 3);
    s.fake.add_account(4, "steven", "owner-pass", 5);
    s
}

fn code_at(t: i64) -> String {
    let totp = Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(6)
        .with_skew(1)
        .with_step_duration(30)
        .with_secret(Secret::try_from_base32(TOTP_SECRET).unwrap())
        .build_noncompliant();
    totp.generate(t as u64).to_string()
}

// ---- sign-in ----

#[tokio::test]
async fn signing_in_gives_a_session_the_page_can_ask_about() {
    let s = with_players().await;
    let (cookie, csrf) = s.sign_in("Kaelyn", "hunter22").await; // the login is case-insensitive
    let me = s.send(Req::get("/api/me").cookie(&cookie)).await;
    assert_eq!(me.status, StatusCode::OK);
    let j = me.json();
    assert_eq!(j["role"], "player");
    assert_eq!(j["login"], "kaelyn");
    assert_eq!(j["csrf"], csrf);
    assert_eq!(j["server_name"], "Test Server");
    let anon = s.send(Req::get("/api/me")).await.json();
    assert_eq!(anon["role"], "visitor");
    assert!(anon.get("csrf").is_none());
    // The cookie is HttpOnly, SameSite=Strict, and not marked Secure over plain HTTP.
    let r = s.send(Req::post("/api/auth/login", json!({ "login": "kaelyn", "password": "hunter22" }))).await;
    let c = r.header("set-cookie").unwrap();
    assert!(c.contains("HttpOnly") && c.contains("SameSite=Strict") && c.contains("Path=/") && !c.contains("Secure"), "{c}");
    assert!(!c.contains("Domain"));
}

#[tokio::test]
async fn a_wrong_password_and_an_unknown_login_look_the_same() {
    let s = with_players().await;
    let a = s.send(Req::post("/api/auth/login", json!({ "login": "kaelyn", "password": "nope-nope" }))).await;
    let b = s.send(Req::post("/api/auth/login", json!({ "login": "nobody", "password": "nope-nope" }).clone())).await;
    assert_eq!(a.status, StatusCode::UNAUTHORIZED);
    assert_eq!(b.status, StatusCode::UNAUTHORIZED);
    assert_eq!(a.json()["error"], b.json()["error"]);
    assert!(a.header("set-cookie").is_none());
}

#[tokio::test]
async fn malformed_logins_and_passwords_are_refused_without_a_lookup() {
    let s = with_players().await;
    for (i, (login, pw)) in [("", "x"), ("a".repeat(17).as_str(), "x"), ("kaelyn", ""), ("kaelyn", "p".repeat(33).as_str()), ("bad\u{1f}", "x")].into_iter().enumerate() {
        let r = s.send(Req::post("/api/auth/login", json!({ "login": login, "password": pw })).peer(20 + i as u8)).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{login:?}/{pw:?}");
    }
}

#[tokio::test]
async fn five_failures_lock_the_pair_for_fifteen_minutes() {
    let s = with_players().await;
    for i in 0..5 {
        let r = s.send(Req::post("/api/auth/login", json!({ "login": "kaelyn", "password": "wrong-pw" }))).await;
        if i < 4 {
            assert_eq!(r.status, StatusCode::UNAUTHORIZED);
        }
    }
    // Even the right password is refused now, from that address.
    let locked = s.send(Req::post("/api/auth/login", json!({ "login": "kaelyn", "password": "hunter22" }))).await;
    assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
    // Another address can still sign in as her.
    let other = s.send(Req::post("/api/auth/login", json!({ "login": "kaelyn", "password": "hunter22" })).peer(9)).await;
    assert_eq!(other.status, StatusCode::OK);
    s.advance(15 * 60 + 5);
    let again = s.send(Req::post("/api/auth/login", json!({ "login": "kaelyn", "password": "hunter22" }))).await;
    assert_eq!(again.status, StatusCode::OK);
    // The action log shows the failures and the refusal.
    let log = s.state.store.actions(None, 50, false, true).await.unwrap();
    assert!(log.iter().any(|a| a.action == "sign_in_failed" && a.result == "refused"));
    assert!(log.iter().filter(|a| a.action == "sign_in_failed").all(|a| a.reason.as_deref() != Some("hunter22")));
}

#[tokio::test]
async fn banned_accounts_and_old_hashes_cannot_sign_in() {
    let s = with_players().await;
    s.fake.set_status(2, 2);
    let r = s.send(Req::post("/api/auth/login", json!({ "login": "ravenna", "password": "secret-pw-2" }))).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.json()["error"], "This account can't sign in.");
    // An old PASSWORD() hash can't be checked: the answer is the same as for a wrong password,
    // whatever was typed, so it doesn't tell a stranger the login exists.
    s.fake.set_hash(2, "*6BB4837EB74329105EE4568DDA7DC67ED2CA2AD9");
    s.fake.set_status(2, 1);
    let old = s.send(Req::post("/api/auth/login", json!({ "login": "ravenna", "password": "secret-pw-2" }))).await;
    let unknown = s.send(Req::post("/api/auth/login", json!({ "login": "nobody", "password": "secret-pw-2" })).peer(9)).await;
    assert_eq!(old.status, StatusCode::UNAUTHORIZED);
    assert_eq!(old.json()["error"], unknown.json()["error"]);
    assert_eq!(old.json()["error"], "Wrong login or password.");
}

#[tokio::test]
async fn two_step_accounts_need_a_fresh_code() {
    let s = with_players().await;
    s.fake.set_totp(1, TOTP_SECRET);
    let creds = json!({ "login": "kaelyn", "password": "hunter22" });
    let r = s.send(Req::post("/api/auth/login", creds.clone())).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.json()["need_otp"], true);
    let with = |code: &str| json!({ "login": "kaelyn", "password": "hunter22", "otp": code });
    let wrong = s.send(Req::post("/api/auth/login", with("000000"))).await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    let good = code_at(NOW);
    assert_eq!(s.send(Req::post("/api/auth/login", with(&good))).await.status, StatusCode::OK);
    // The same code can't be used twice, but the next step's can.
    assert_eq!(s.send(Req::post("/api/auth/login", with(&good))).await.status, StatusCode::UNAUTHORIZED);
    s.advance(30);
    let next = code_at(NOW + 30);
    assert_eq!(s.send(Req::post("/api/auth/login", with(&next))).await.status, StatusCode::OK);
    // Two steps off is too far.
    s.advance(120);
    assert_eq!(s.send(Req::post("/api/auth/login", with(&code_at(NOW)))).await.status, StatusCode::UNAUTHORIZED);
}

// ---- sessions ----

#[tokio::test]
async fn a_ban_or_demotion_takes_effect_on_the_next_request() {
    let s = with_players().await;
    let (cookie, _) = s.sign_in("steven", "owner-pass").await;
    assert_eq!(s.send(Req::get("/api/admin/dashboard").cookie(&cookie)).await.status, StatusCode::OK);
    // An in-game GM level change is not a website demotion...
    s.fake.set_gm(4, 0);
    assert_eq!(s.send(Req::get("/api/admin/dashboard").cookie(&cookie)).await.status, StatusCode::OK);
    // ...a change of the stored website role is, at once.
    s.state.store.set_site_role(4, "steven", fake_created(4), "player", "test", NOW).await.unwrap();
    assert_eq!(s.send(Req::get("/api/admin/dashboard").cookie(&cookie)).await.status, StatusCode::FORBIDDEN);
    assert_eq!(s.send(Req::get("/api/me").cookie(&cookie)).await.json()["role"], "player");
    s.fake.set_status(4, 2);
    assert_eq!(s.send(Req::get("/api/me").cookie(&cookie)).await.json()["role"], "visitor", "a banned account's session ends");
    assert_eq!(s.state.store.session_count().await.unwrap(), 0);
}

#[tokio::test]
async fn staff_sessions_idle_out_after_thirty_minutes_players_after_a_week() {
    let s = with_players().await;
    let (staff, _) = s.sign_in("halvard", "gm-password").await;
    let (player, _) = s.sign_in("kaelyn", "hunter22").await;
    s.advance(29 * 60);
    assert_eq!(s.send(Req::get("/api/admin/dashboard").cookie(&staff)).await.status, StatusCode::OK);
    s.advance(29 * 60); // 58 minutes in, but active 29 minutes ago
    assert_eq!(s.send(Req::get("/api/admin/dashboard").cookie(&staff)).await.status, StatusCode::OK);
    s.advance(31 * 60);
    assert_eq!(s.send(Req::get("/api/admin/dashboard").cookie(&staff)).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(s.send(Req::get("/api/me/account").cookie(&player)).await.status, StatusCode::OK, "a player is still in an hour on");
    s.advance(8 * 24 * 3600);
    assert_eq!(s.send(Req::get("/api/me/account").cookie(&player)).await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn signing_out_ends_the_session() {
    let s = with_players().await;
    let (cookie, csrf) = s.sign_in("kaelyn", "hunter22").await;
    let out = s.send(Req::post("/api/auth/logout", json!({})).session(&cookie, &csrf)).await;
    assert_eq!(out.status, StatusCode::OK);
    assert_eq!(s.send(Req::get("/api/me/account").cookie(&cookie)).await.status, StatusCode::UNAUTHORIZED);
}

// ---- CSRF and friends ----

#[tokio::test]
async fn changes_need_the_csrf_token_a_json_body_and_a_matching_origin() {
    let s = with_players().await;
    let (cookie, csrf) = s.sign_in("kaelyn", "hunter22").await;
    let body = json!({ "current": "hunter22", "new": "short" });
    let post = |req: Req| s.send(req);

    // No token, wrong token, no session.
    let r = post(Req::post("/api/me/password", body.clone()).cookie(&cookie)).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = post(Req::post("/api/me/password", body.clone()).session(&cookie, "not-the-token")).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = post(Req::post("/api/me/password", body.clone())).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    // Another site's Origin, even with the right token.
    let mut cross = Req::post("/api/me/password", body.clone()).session(&cookie, &csrf);
    cross.origin = Some("http://evil.test".into());
    assert_eq!(post(cross).await.status, StatusCode::FORBIDDEN);
    let mut null = Req::post("/api/me/password", body.clone()).session(&cookie, &csrf);
    null.origin = Some("null".into());
    assert_eq!(post(null).await.status, StatusCode::FORBIDDEN);

    // Not JSON.
    let mut form = Req::post("/api/me/password", body.clone()).session(&cookie, &csrf);
    form.content_type = false;
    assert_eq!(post(form).await.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);

    // Right in every way: the request reaches the handler (which refuses the short password).
    let ok = post(Req::post("/api/me/password", body).session(&cookie, &csrf)).await;
    assert_eq!(ok.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn sign_in_checks_the_origin_but_needs_no_token() {
    let s = with_players().await;
    let mut cross = Req::post("/api/auth/login", json!({ "login": "kaelyn", "password": "hunter22" }));
    cross.origin = Some("http://evil.test".into());
    assert_eq!(s.send(cross).await.status, StatusCode::FORBIDDEN);
    let mut none = Req::post("/api/auth/login", json!({ "login": "kaelyn", "password": "hunter22" }));
    none.origin = None;
    assert_eq!(s.send(none).await.status, StatusCode::OK);
}

#[tokio::test]
async fn thirty_changes_a_minute_per_session() {
    let s = with_players().await;
    let (cookie, csrf) = s.sign_in("kaelyn", "hunter22").await;
    let short = json!({ "current": "x", "new": "short" });
    for i in 0..31 {
        let r = s.send(Req::post("/api/me/password", short.clone()).session(&cookie, &csrf)).await;
        if i < 30 {
            assert_eq!(r.status, StatusCode::BAD_REQUEST, "{i}");
        } else {
            assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
        }
    }
    s.advance(61);
    assert_eq!(s.send(Req::post("/api/me/password", short).session(&cookie, &csrf)).await.status, StatusCode::BAD_REQUEST);
}

// ---- roles ----

#[tokio::test]
async fn each_role_reaches_only_its_own_routes() {
    let s = with_players().await;
    let (player, _) = s.sign_in("kaelyn", "hunter22").await;
    let (moderator, _) = s.sign_in("halvard", "gm-password").await;
    let (owner, _) = s.sign_in("steven", "owner-pass").await;

    let admin = "/api/admin/dashboard";
    assert_eq!(s.send(Req::get(admin)).await.status, StatusCode::UNAUTHORIZED, "visitor");
    assert_eq!(s.send(Req::get(admin).cookie(&player)).await.status, StatusCode::FORBIDDEN, "player");
    assert_eq!(s.send(Req::get("/api/admin/log").cookie(&player)).await.status, StatusCode::FORBIDDEN);
    assert_eq!(s.send(Req::get(admin).cookie(&moderator)).await.status, StatusCode::OK);
    assert_eq!(s.send(Req::get(admin).cookie(&owner)).await.status, StatusCode::OK);
    assert_eq!(s.send(Req::get("/api/admin/log").cookie(&moderator)).await.status, StatusCode::OK);

    assert_eq!(s.send(Req::get("/api/me/account")).await.status, StatusCode::UNAUTHORIZED);
    // Staff have game accounts too.
    assert_eq!(s.send(Req::get("/api/me/account").cookie(&moderator)).await.status, StatusCode::OK);
    assert_eq!(s.send(Req::get("/api/me").cookie(&moderator)).await.json()["role"], "moderator");
    assert_eq!(s.send(Req::get("/api/me").cookie(&owner)).await.json()["role"], "owner");
}

#[tokio::test]
async fn only_the_owner_sees_addresses_on_the_dashboard() {
    let s = with_players().await;
    s.fake.0.lock().unwrap().online = vec![online_row("Torvald", "Batallia Downs", 0x0100A8C0), online_row("Pipiru", "Batallia Downs", 0x0200A8C0), online_row("Sefira", "Port Jeuno", 0x0300A8C0)];
    let (moderator, _) = s.sign_in("halvard", "gm-password").await;
    let (owner, _) = s.sign_in("steven", "owner-pass").await;

    let m = s.send(Req::get("/api/admin/dashboard").cookie(&moderator)).await.json();
    assert!(m["online"][0].get("address").is_none(), "{m}");
    assert!(!m.to_string().contains("192.168"));
    let o = s.send(Req::get("/api/admin/dashboard").cookie(&owner)).await.json();
    assert_eq!(o["online"][0]["address"], "192.168.0.1");
    assert_eq!(o["counts"]["online"], 3);
    assert_eq!(o["counts"]["accounts"], 4);
    assert_eq!(o["zones"][0], json!({ "zone": "Batallia Downs", "count": 2 }));
    assert_eq!(o["zones"][1], json!({ "zone": "Port Jeuno", "count": 1 }));
    assert_eq!(o["services"][0]["id"], "db");
    assert!(o["actions"].is_array());
}

// ---- My account is scoped to the session's account ----

#[tokio::test]
async fn my_account_queries_use_the_sessions_account_id_never_the_requests() {
    let s = with_players().await;
    let (cookie, _) = s.sign_in("kaelyn", "hunter22").await;
    // Asking for someone else's account by any parameter changes nothing.
    let a = s.send(Req::get("/api/me/account?accid=2&id=2&login=ravenna").cookie(&cookie)).await.json();
    assert_eq!(a["account"]["login"], "kaelyn");
    assert_eq!(a["characters"].as_array().unwrap().len(), 1);
    assert_eq!(a["characters"][0]["name"], "kaelynchar");

    // Her own character works; her neighbour's is a 404 exactly like a missing one.
    assert_eq!(s.send(Req::get("/api/me/characters/10").cookie(&cookie)).await.status, StatusCode::OK);
    let theirs = s.send(Req::get("/api/me/characters/20").cookie(&cookie)).await;
    let missing = s.send(Req::get("/api/me/characters/99999").cookie(&cookie)).await;
    assert_eq!(theirs.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(theirs.text(), missing.text());
    let inv_theirs = s.send(Req::get("/api/me/characters/20/inventory").cookie(&cookie)).await;
    assert_eq!(inv_theirs.status, StatusCode::NOT_FOUND);
    assert_eq!(s.send(Req::get("/api/me/characters/10/inventory").cookie(&cookie)).await.json()["items"][0]["name"], "Hi-Potion");

    let d = s.fake.0.lock().unwrap();
    assert!(d.account_queries.iter().all(|a| *a == 1), "{:?}", d.account_queries);
    assert!(d.char_queries.iter().all(|(acc, _)| *acc == 1), "{:?}", d.char_queries);
    assert!(d.char_queries.contains(&(1, 20)), "the request's id reached the query as a character id only");
}

#[tokio::test]
async fn the_character_view_is_read_only() {
    let s = with_players().await;
    let (cookie, csrf) = s.sign_in("kaelyn", "hunter22").await;
    for method in [axum::http::Method::POST, axum::http::Method::PUT, axum::http::Method::DELETE] {
        let mut r = Req::post("/api/me/characters/10", json!({ "gil": 999999 })).session(&cookie, &csrf);
        r.method = method;
        let res = s.send(r).await;
        assert_eq!(res.status, StatusCode::METHOD_NOT_ALLOWED);
    }
}

// ---- password change ----

#[tokio::test]
async fn changing_your_password_checks_the_old_one_and_ends_other_sign_ins() {
    let s = with_players().await;
    let (cookie, csrf) = s.sign_in("kaelyn", "hunter22").await;
    let (other, _) = s.sign_in("kaelyn", "hunter22").await;
    let post = |body: serde_json::Value| s.send(Req::post("/api/me/password", body).session(&cookie, &csrf));

    assert_eq!(post(json!({ "current": "wrong-password", "new": "a-new-password" })).await.status, StatusCode::FORBIDDEN);
    assert_eq!(post(json!({ "current": "hunter22", "new": "short" })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(post(json!({ "current": "hunter22", "new": "x".repeat(33) })).await.status, StatusCode::BAD_REQUEST);
    let before = s.fake.hash_of(1);

    assert_eq!(post(json!({ "current": "hunter22", "new": "a-new-password" })).await.status, StatusCode::OK);
    let after = s.fake.hash_of(1);
    assert_ne!(before, after);
    assert!(after.starts_with("$2a$"), "{after}");
    assert_eq!(auth::check_password("a-new-password", &after), auth::HashCheck::Match);
    // This sign-in stays; the other ends.
    assert_eq!(s.send(Req::get("/api/me/account").cookie(&cookie)).await.status, StatusCode::OK);
    assert_eq!(s.send(Req::get("/api/me/account").cookie(&other)).await.status, StatusCode::UNAUTHORIZED);
    // The old password no longer signs in; the new one does.
    assert_eq!(s.send(Req::post("/api/auth/login", json!({ "login": "kaelyn", "password": "hunter22" }))).await.status, StatusCode::UNAUTHORIZED);
    s.sign_in("kaelyn", "a-new-password").await;
    // The log records the change, with no password in it.
    let log = s.state.store.actions(None, 50, false, true).await.unwrap();
    let change = log.iter().find(|a| a.action == "password_change" && a.result == "ok").expect("logged");
    assert_eq!(change.actor_login.as_deref(), Some("kaelyn"));
    assert!(change.reason.is_none());
}

#[tokio::test]
async fn a_player_can_only_change_their_own_password() {
    let s = with_players().await;
    let (cookie, csrf) = s.sign_in("kaelyn", "hunter22").await;
    // A body naming another account is ignored: the check is against the session's account.
    let r = s.send(Req::post("/api/me/password", json!({ "accid": 2, "login": "ravenna", "current": "secret-pw-2", "new": "hijacked-pw" })).session(&cookie, &csrf)).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(auth::check_password("secret-pw-2", &s.fake.hash_of(2)), auth::HashCheck::Match);
}

// ---- owner link ----

fn link(now: i64) -> String {
    wrapper_core::owner_link::sign(SECRET, now as u64).unwrap()
}

#[tokio::test]
async fn the_owner_link_signs_the_owner_in_once_and_redirects_to_admin() {
    let s = site().await;
    let t = link(NOW);
    let r = s.send(Req::get(&format!("/auth/owner?t={t}"))).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(r.header("location").as_deref(), Some("/admin"));
    assert_eq!(r.header("referrer-policy").as_deref(), Some("no-referrer"));
    let cookie = r.cookie().expect("owner session cookie");

    let me = s.send(Req::get("/api/me").cookie(&cookie)).await.json();
    assert_eq!(me["role"], "owner");
    assert_eq!(me["owner_link"], true);
    assert_eq!(s.send(Req::get("/api/admin/dashboard").cookie(&cookie)).await.status, StatusCode::OK);
    // No game account behind it.
    assert_eq!(s.send(Req::get("/api/me/account").cookie(&cookie)).await.status, StatusCode::NOT_FOUND);

    // Used once.
    let again = s.send(Req::get(&format!("/auth/owner?t={t}"))).await;
    assert_eq!(again.status, StatusCode::FORBIDDEN);
    assert!(again.cookie().is_none());
}

#[tokio::test]
async fn expired_tampered_and_missing_links_are_refused() {
    let s = site().await;
    let old = link(NOW - 120);
    assert_eq!(s.send(Req::get(&format!("/auth/owner?t={old}"))).await.status, StatusCode::FORBIDDEN);
    let mut t = link(NOW);
    t.push('x');
    assert_eq!(s.send(Req::get(&format!("/auth/owner?t={t}"))).await.status, StatusCode::FORBIDDEN);
    assert_eq!(s.send(Req::get("/auth/owner")).await.status, StatusCode::FORBIDDEN);
    let other = wrapper_core::owner_link::sign(&"ab".repeat(32), NOW as u64).unwrap();
    assert_eq!(s.send(Req::get(&format!("/auth/owner?t={other}"))).await.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn without_a_secret_the_owner_link_route_does_not_exist() {
    for secret in ["", "tooshort"] {
        let mut cfg = ffxi_web::config::Config::for_tests();
        cfg.admin_secret = secret.into();
        let s = site_with(cfg).await;
        let r = s.send(Req::get(&format!("/auth/owner?t={}", link(NOW)))).await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "{secret:?}");
    }
}

// ---- pages ----

#[tokio::test]
async fn pages_are_gated_before_any_html_goes_out() {
    let s = with_players().await;
    let (player, _) = s.sign_in("kaelyn", "hunter22").await;
    let (staff, _) = s.sign_in("halvard", "gm-password").await;

    // Everyone gets the home and sign-in pages.
    assert_eq!(s.send(Req::get("/")).await.status, StatusCode::OK);
    assert_eq!(s.send(Req::get("/signin")).await.status, StatusCode::OK);
    // A visitor is sent to sign in.
    let r = s.send(Req::get("/me")).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert!(r.header("location").unwrap().starts_with("/signin"));
    assert_eq!(s.send(Req::get("/admin")).await.status, StatusCode::SEE_OTHER);
    // A player gets My account, and "not found" for the admin page: its HTML never reaches them.
    assert_eq!(s.send(Req::get("/me").cookie(&player)).await.status, StatusCode::OK);
    assert_eq!(s.send(Req::get("/me/characters/10").cookie(&player)).await.status, StatusCode::OK);
    let denied = s.send(Req::get("/admin").cookie(&player)).await;
    assert_eq!(denied.status, StatusCode::NOT_FOUND);
    assert!(!denied.text().contains("sidebar"));
    // Staff get the admin page.
    let page = s.send(Req::get("/admin").cookie(&staff)).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.header("content-type").unwrap().starts_with("text/html"));
    assert!(page.text().contains("data-page=\"dashboard\""));
    // The pages named by file are not served as files.
    assert_eq!(s.send(Req::get("/ui/admin.html").cookie(&staff)).await.status, StatusCode::NOT_FOUND);
    // No page carries an admin link for a player: the public pages are static, and the script
    // only draws "Admin panel" for staff.
    for p in ["/", "/me", "/signin"] {
        let t = s.send(Req::get(p).cookie(&player)).await.text();
        assert!(!t.contains("/admin"), "{p} mentions /admin");
    }
}

#[tokio::test]
async fn every_response_carries_the_security_headers() {
    let s = with_players().await;
    for p in ["/", "/signin", "/api/me", "/ui/web.css", "/healthz"] {
        let r = s.send(Req::get(p)).await;
        let csp = r.header("content-security-policy").unwrap_or_default();
        assert!(csp.contains("default-src 'self'") && csp.contains("script-src 'self'") && csp.contains("style-src 'self'") && csp.contains("frame-ancestors 'none'"), "{p}: {csp}");
        assert_eq!(r.header("referrer-policy").as_deref(), Some("no-referrer"), "{p}");
        assert_eq!(r.header("x-content-type-options").as_deref(), Some("nosniff"), "{p}");
    }
}

#[tokio::test]
async fn the_shared_stylesheet_fonts_and_scripts_are_served() {
    let s = site().await;
    for (path, ctype) in [
        ("/ui/tokens.css", "text/css"),
        ("/ui/web.css", "text/css"),
        ("/ui/site.css", "text/css"),
        ("/ui/app.js", "text/javascript"),
        ("/ui/theme.js", "text/javascript"),
        ("/ui/fonts/FiraSans-Regular.woff2", "font/woff2"),
        ("/ui/modern/frame.svg", "image/svg+xml"),
    ] {
        let r = s.send(Req::get(path)).await;
        assert_eq!(r.status, StatusCode::OK, "{path}");
        assert!(r.header("content-type").unwrap().starts_with(ctype), "{path}");
        assert!(!r.bytes.is_empty());
    }
    assert_eq!(s.send(Req::get("/ui/nothing.css")).await.status, StatusCode::NOT_FOUND);
    assert_eq!(s.send(Req::get("/ui/../Cargo.toml")).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn no_page_uses_inline_scripts_or_styles() {
    // The Content-Security-Policy forbids them, so a page that needed one would be broken.
    for name in ["home", "signin", "me", "me-character", "admin", "accounts", "account", "characters", "character", "bans", "log"] {
        let html = std::fs::read_to_string(format!("ui/{name}.html")).unwrap();
        assert!(!html.contains(" style="), "{name}.html has an inline style");
        assert!(!html.contains("onclick") && !html.contains("onload"), "{name}.html has an inline handler");
        for script in html.split("<script").skip(1) {
            assert!(script.trim_start().starts_with("src="), "{name}.html has an inline script");
        }
    }
}

// ---- public pages and health ----

#[tokio::test]
async fn the_public_home_shows_only_its_listed_keys() {
    let s = with_players().await;
    s.fake.add_person(Person::new("Torvald").online(7, 52, "Batallia_Downs"));
    s.fake.add_person(Person::new("Halvard").gm(3).online(12, 75, "Ru'Lude_Gardens"));
    s.fake.add_person(Person::new("Ghost").gm(3).hidden().online(12, 75, "Ru'Lude_Gardens"));
    let j = s.send(Req::get("/api/public/home")).await.json();
    let mut keys: Vec<&str> = j.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    let mut expected = ffxi_web::routes::public::HOME_KEYS.to_vec();
    expected.sort();
    assert_eq!(keys, expected, "a new key on a public route must be added to HOME_KEYS on purpose");
    assert_eq!(j["online"], 1, "GMs and hidden characters aren't counted");
    let text = j.to_string();
    assert!(!text.contains("Torvald") && !text.contains("kaelyn"));
}

#[tokio::test]
async fn the_home_page_still_answers_while_the_database_is_down() {
    let s = site().await;
    s.fake.0.lock().unwrap().down = true;
    let j = s.send(Req::get("/api/public/home")).await.json();
    assert_eq!(j["up"], false);
    assert!(j["online"].is_null());
    assert_eq!(s.send(Req::get("/healthz")).await.status, StatusCode::SERVICE_UNAVAILABLE);
    s.fake.0.lock().unwrap().down = false;
    assert_eq!(s.send(Req::get("/healthz")).await.status, StatusCode::OK);
}

#[tokio::test]
async fn missing_lsb_tables_answer_503_with_a_message() {
    let s = with_players().await;
    let (cookie, _) = s.sign_in("kaelyn", "hunter22").await;
    s.fake.0.lock().unwrap().not_ready = true;
    let r = s.send(Req::get("/api/me/account").cookie(&cookie)).await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(r.json()["error"].as_str().unwrap().contains("database"));
}

#[tokio::test]
async fn public_gets_are_limited_per_address() {
    let s = site().await;
    for _ in 0..120 {
        assert_eq!(s.send(Req::get("/api/public/home")).await.status, StatusCode::OK);
    }
    assert_eq!(s.send(Req::get("/api/public/home")).await.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(s.send(Req::get("/api/public/home").peer(77)).await.status, StatusCode::OK);
    s.advance(61);
    assert_eq!(s.send(Req::get("/api/public/home")).await.status, StatusCode::OK);
}

#[tokio::test]
async fn the_dashboard_lists_recent_admin_actions_but_not_sign_ins() {
    let s = with_players().await;
    let (owner, _) = s.sign_in("steven", "owner-pass").await;
    s.state
        .store
        .log(ffxi_web::store::NewAction { actor_kind: "account", actor_login: Some("steven".into()), actor_role: "owner", action: "ban", target_kind: Some("account"), target_name: Some("yashka".into()), reason: Some("spam".into()), result: "ok", ..Default::default() }, NOW)
        .await
        .unwrap();
    let d = s.send(Req::get("/api/admin/dashboard").cookie(&owner)).await.json();
    let actions = d["actions"].as_array().unwrap();
    assert_eq!(actions.len(), 1, "{actions:?}");
    assert_eq!(actions[0]["action"], "ban");
    assert_eq!(actions[0]["target_name"], "yashka");
    let log = s.send(Req::get("/api/admin/log").cookie(&owner)).await.json();
    assert!(log["items"].as_array().unwrap().iter().any(|a| a["action"] == "sign_in"));
}

// ---- the sign-in limits hold up under bursts and tricks ----

#[tokio::test]
async fn a_burst_of_simultaneous_sign_ins_is_counted_before_the_passwords_are_checked() {
    let s = with_players().await;
    let attempts: Vec<_> = (0..60)
        .map(|i| s.send(Req::post("/api/auth/login", json!({ "login": "kaelyn", "password": if i == 59 { "hunter22".to_string() } else { format!("guess-{i}-pw") } }))))
        .collect();
    let results = futures_join_all(attempts).await;
    let refused = results.iter().filter(|r| r.status == StatusCode::TOO_MANY_REQUESTS).count();
    assert!(refused >= 55, "only {refused} of 60 were refused");
    // At most five passwords were really tried: the log has at most five error rows.
    let log = s.state.store.actions(None, 200, false, true).await.unwrap();
    let tried = log.iter().filter(|a| a.action == "sign_in_failed" && a.result == "error").count();
    assert!(tried <= 5, "{tried} passwords were checked");
}

async fn futures_join_all<F: std::future::Future<Output = Resp>>(futs: Vec<F>) -> Vec<Resp> {
    let mut out = Vec::new();
    // The router is driven one request at a time here, in order; the limiter's reservation is
    // what stops a real burst (see limits.rs's thread test).
    for f in futs {
        out.push(f.await);
    }
    out
}

#[tokio::test]
async fn other_spellings_of_a_login_share_one_account_lockout() {
    let s = with_players().await;
    // The database matches "KAELYN" and "kaelyn   " as the same account.
    for spelling in ["kaelyn", "KAELYN", "kaelyn ", "Kaelyn  ", "kaelyn"] {
        s.send(Req::post("/api/auth/login", json!({ "login": spelling, "password": "wrong-pw" }))).await;
    }
    let r = s.send(Req::post("/api/auth/login", json!({ "login": "kaelyn   ", "password": "hunter22" }))).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn behind_the_docker_gateway_strangers_cannot_lock_everyone_out() {
    let s = with_players().await;
    // peer 1 is 10.0.0.1: the kind of address Docker's proxy shows for every visitor.
    for i in 0..40 {
        s.send(Req::post("/api/auth/login", json!({ "login": format!("made-up-{i}"), "password": "wrong-pw" })).peer(1)).await;
    }
    let r = s.send(Req::post("/api/auth/login", json!({ "login": "steven", "password": "owner-pass" })).peer(1)).await;
    assert_eq!(r.status, StatusCode::OK);
    let log = s.state.store.actions(None, 200, false, true).await.unwrap();
    assert!(log.iter().any(|a| a.ip.as_deref() == Some("10.0.0.1 (proxy)")));
}

#[tokio::test]
async fn the_log_never_holds_what_was_typed_into_the_login_box() {
    let s = with_players().await;
    s.send(Req::post("/api/auth/login", json!({ "login": "my-secret-pw", "password": "x" }))).await;
    s.send(Req::post("/api/auth/login", json!({ "login": "kaelyn", "password": "wrong-pw" }))).await;
    s.send(Req::post("/api/auth/login", json!({ "login": "bad\u{1f}login", "password": "x" })).peer(9)).await;
    let (moderator, _) = s.sign_in("halvard", "gm-password").await;
    let log = s.send(Req::get("/api/admin/log").cookie(&moderator)).await;
    let text = log.text();
    assert!(!text.contains("my-secret-pw") && !text.contains("bad"), "{text}");
    // The attempt on a real account is logged against it.
    assert!(text.contains("\"actor_login\":\"kaelyn\""));
    assert!(text.contains("unknown login"));
}

#[tokio::test]
async fn request_bodies_are_capped() {
    let s = with_players().await;
    let big = "x".repeat(100_000);
    let r = s.send(Req::post("/api/auth/login", json!({ "login": big, "password": "x" }))).await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
}
