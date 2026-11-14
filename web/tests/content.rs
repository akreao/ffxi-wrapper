//! The public home page, news and rules, and the owner's editors behind them.

mod common;

use axum::http::{Method, StatusCode};
use common::*;
use ffxi_web::config::Config;
use ffxi_web::routes::public::{CONTENT_KEYS, HOME_KEYS, NEWS_KEYS, POST_KEYS, POST_LIST_KEYS};
use serde_json::{json, Value};

struct Who {
    cookie: String,
    csrf: String,
}

/// kaelyn (player), halvard (moderator), steven (owner).
async fn people() -> Site {
    let s = site().await;
    s.fake.add_account(1, "kaelyn", "hunter22", 0);
    s.fake.add_account(3, "halvard", "gm-password", 3);
    s.fake.add_account(4, "steven", "owner-pass", 5);
    s
}

async fn who(s: &Site, login: &str, pw: &str) -> Who {
    let (cookie, csrf) = s.sign_in(login, pw).await;
    Who { cookie, csrf }
}

fn with(w: &Who, r: Req) -> Req {
    r.session(&w.cookie, &w.csrf)
}

fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

fn sorted(a: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = a.iter().map(|s| s.to_string()).collect();
    v.sort();
    v
}

async fn new_post(s: &Site, w: &Who, title: &str, body: &str, publish: bool, pinned: bool) -> i64 {
    let r = s.send(with(w, Req::post("/api/admin/news", json!({ "title": title, "body": body, "publish": publish, "pinned": pinned })))).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    r.json()["id"].as_i64().unwrap()
}

// ---- who may write ----

#[tokio::test]
async fn only_the_owner_writes_news_and_content() {
    let s = people().await;
    let owner = who(&s, "steven", "owner-pass").await;
    let moderator = who(&s, "halvard", "gm-password").await;
    let player = who(&s, "kaelyn", "hunter22").await;
    let id = new_post(&s, &owner, "First", "Hello", true, false).await;

    let body = json!({ "title": "T", "body": "B", "publish": true });
    let calls: Vec<(Method, String, Value)> = vec![
        (Method::GET, "/api/admin/news".into(), Value::Null),
        (Method::GET, format!("/api/admin/news/{id}"), Value::Null),
        (Method::POST, "/api/admin/news".into(), body.clone()),
        (Method::PUT, format!("/api/admin/news/{id}"), body.clone()),
        (Method::DELETE, format!("/api/admin/news/{id}"), json!({})),
        (Method::GET, "/api/admin/content".into(), Value::Null),
        (Method::PUT, "/api/admin/content/footer".into(), json!({ "body": "x" })),
        (Method::DELETE, "/api/admin/content/footer".into(), json!({})),
    ];
    let before = s.state.store.action_count().await.unwrap();
    for (m, path, b) in &calls {
        let mk = |w: Option<&Who>| {
            let mut r = if *m == Method::GET { Req::get(path) } else { Req::post(path, b.clone()) };
            r.method = m.clone();
            match w { Some(w) => with(w, r), None => r }
        };
        assert_eq!(s.send(mk(None)).await.status, StatusCode::UNAUTHORIZED, "visitor {m} {path}");
        assert_eq!(s.send(mk(Some(&player))).await.status, StatusCode::FORBIDDEN, "player {m} {path}");
        assert_eq!(s.send(mk(Some(&moderator))).await.status, StatusCode::FORBIDDEN, "moderator {m} {path}");
    }
    // Nothing the refused calls tried changed anything.
    assert_eq!(s.state.store.action_count().await.unwrap(), before);
    let list = s.send(with(&owner, Req::get("/api/admin/news"))).await.json();
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    assert_eq!(list["items"][0]["title"], "First");
    // The owner can use every one of them.
    assert_eq!(s.send(with(&owner, Req::get("/api/admin/content"))).await.status, StatusCode::OK);
}

#[tokio::test]
async fn the_preview_is_for_staff_only() {
    let s = people().await;
    let moderator = who(&s, "halvard", "gm-password").await;
    let player = who(&s, "kaelyn", "hunter22").await;
    let owner = who(&s, "steven", "owner-pass").await;
    let req = || Req::post("/api/admin/preview", json!({ "markdown": "**hi** <script>x</script>" }));
    assert_eq!(s.send(req()).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(s.send(with(&player, req())).await.status, StatusCode::FORBIDDEN);
    for w in [&moderator, &owner] {
        let r = s.send(with(w, req())).await;
        assert_eq!(r.status, StatusCode::OK);
        let html = r.json()["html"].as_str().unwrap().to_string();
        assert!(html.contains("<strong>hi</strong>") && !html.contains("<script"), "{html}");
    }
}

#[tokio::test]
async fn editor_pages_are_the_owners_alone() {
    let s = people().await;
    let owner = who(&s, "steven", "owner-pass").await;
    let moderator = who(&s, "halvard", "gm-password").await;
    let player = who(&s, "kaelyn", "hunter22").await;
    for path in ["/admin/news", "/admin/news/3", "/admin/content"] {
        assert_eq!(s.send(Req::get(path)).await.status, StatusCode::SEE_OTHER, "visitor {path}");
        assert_eq!(s.send(Req::get(path).cookie(&moderator.cookie)).await.status, StatusCode::NOT_FOUND, "moderator {path}");
        assert_eq!(s.send(Req::get(path).cookie(&player.cookie)).await.status, StatusCode::NOT_FOUND, "player {path}");
        assert_eq!(s.send(Req::get(path).cookie(&owner.cookie)).await.status, StatusCode::OK, "owner {path}");
    }
    for path in ["/news", "/news/1", "/rules", "/"] {
        assert_eq!(s.send(Req::get(path)).await.status, StatusCode::OK, "visitor {path}");
    }
}

// ---- what visitors get ----

#[tokio::test]
async fn public_routes_send_exactly_these_keys() {
    let s = people().await;
    let owner = who(&s, "steven", "owner-pass").await;
    let id = new_post(&s, &owner, "Pinned news", "Some **text**", true, true).await;
    new_post(&s, &owner, "A draft", "secret draft", false, false).await;

    let home = s.send(Req::get("/api/public/home")).await.json();
    assert_eq!(keys(&home), sorted(&HOME_KEYS));
    assert_eq!(home["signup_open"], false);
    let posts = home["news"].as_array().unwrap();
    assert_eq!(posts.len(), 1, "drafts stay private");
    assert_eq!(keys(&posts[0]), sorted(&POST_LIST_KEYS));

    let list = s.send(Req::get("/api/public/news")).await.json();
    assert_eq!(keys(&list), sorted(&NEWS_KEYS));
    assert_eq!(keys(&list["items"][0]), sorted(&POST_LIST_KEYS));

    let post = s.send(Req::get(&format!("/api/public/news/{id}"))).await.json();
    assert_eq!(keys(&post), sorted(&POST_KEYS));

    let rules = s.send(Req::get("/api/public/content/rules")).await.json();
    assert_eq!(keys(&rules), sorted(&CONTENT_KEYS));

    // Nothing about who edited, the logins, or the drafts is anywhere in what a visitor reads.
    let all = format!("{home}{list}{post}{rules}");
    for secret in ["steven", "updated_by", "secret draft", "A draft", "kaelyn"] {
        assert!(!all.contains(secret), "{secret} leaked: {all}");
    }
}

#[tokio::test]
async fn only_published_posts_and_the_rules_are_served_by_name() {
    let s = people().await;
    let owner = who(&s, "steven", "owner-pass").await;
    let draft = new_post(&s, &owner, "Draft", "x", false, false).await;
    assert_eq!(s.send(Req::get(&format!("/api/public/news/{draft}"))).await.status, StatusCode::NOT_FOUND);
    assert_eq!(s.send(Req::get("/api/public/news/9999")).await.status, StatusCode::NOT_FOUND);
    assert_eq!(s.send(Req::get("/api/public/news/abc")).await.status, StatusCode::BAD_REQUEST);
    // The other blocks aren't separate public routes.
    for key in ["footer", "connect", "home_title", "nope"] {
        assert_eq!(s.send(Req::get(&format!("/api/public/content/{key}"))).await.status, StatusCode::NOT_FOUND, "{key}");
    }
}

#[tokio::test]
async fn posts_are_rendered_without_raw_html_or_unsafe_links() {
    let s = people().await;
    let owner = who(&s, "steven", "owner-pass").await;
    let body = "Hi <script>alert(1)</script>\n\n[bad](javascript:alert(1)) [ok](https://example.org) ![x](https://evil.example/a.png)\n\n<img src=x onerror=alert(1)>";
    let id = new_post(&s, &owner, "Render", body, true, false).await;
    let html = s.send(Req::get(&format!("/api/public/news/{id}"))).await.json()["body_html"].as_str().unwrap().to_string();
    for bad in ["<script", "javascript:", "<img", "evil.example", "onerror=alert(1)>"] {
        assert!(!html.contains(bad), "{bad} in {html}");
    }
    assert!(html.contains("&lt;script&gt;"));
    assert!(html.contains("<a href=\"https://example.org\" rel=\"nofollow noopener\">ok</a>"));
    // The admin preview is the same text.
    let prev = s.send(with(&owner, Req::post("/api/admin/preview", json!({ "markdown": body })))).await.json();
    assert_eq!(prev["html"], json!(html));
}

#[tokio::test]
async fn pinned_posts_come_first_and_the_list_pages() {
    let s = people().await;
    let owner = who(&s, "steven", "owner-pass").await;
    for i in 1..=12 {
        s.advance(10);
        new_post(&s, &owner, &format!("Post {i}"), "x", true, i == 3).await;
    }
    let first = s.send(Req::get("/api/public/news")).await.json();
    let titles: Vec<&str> = first["items"].as_array().unwrap().iter().map(|p| p["title"].as_str().unwrap()).collect();
    assert_eq!(titles.len(), 10);
    assert_eq!(titles[0], "Post 3", "pinned first");
    assert_eq!(titles[1], "Post 12", "then newest");
    assert_eq!(first["total"], 12);
    let second = s.send(Req::get("/api/public/news?page=2")).await.json();
    assert_eq!(second["items"].as_array().unwrap().len(), 2);
    let home = s.send(Req::get("/api/public/home")).await.json();
    let home_titles: Vec<&str> = home["news"].as_array().unwrap().iter().map(|p| p["title"].as_str().unwrap()).collect();
    assert_eq!(home_titles, vec!["Post 3", "Post 12", "Post 11", "Post 10", "Post 9"]);
    // A page far past the end is empty, and silly numbers are held to a limit.
    assert_eq!(s.send(Req::get("/api/public/news?page=99")).await.json()["items"].as_array().unwrap().len(), 0);
    assert_eq!(s.send(Req::get("/api/public/news?page=-4")).await.json()["page"], 1);
    assert_eq!(s.send(Req::get("/api/public/news?page=99999999999")).await.json()["page"], 10_000);
}

// ---- the editors ----

#[tokio::test]
async fn publishing_pinning_unpublishing_and_deleting_show_up_at_once() {
    let s = people().await;
    let owner = who(&s, "steven", "owner-pass").await;
    // Warm the cache with an empty site.
    assert_eq!(s.send(Req::get("/api/public/home")).await.json()["news"].as_array().unwrap().len(), 0);
    assert_eq!(s.send(Req::get("/api/public/news")).await.json()["total"], 0);

    let id = new_post(&s, &owner, "Hello", "World", false, false).await;
    assert_eq!(s.send(Req::get("/api/public/news")).await.json()["total"], 0, "a draft isn't news");

    let put = |b: Value| with(&owner, Req::put(&format!("/api/admin/news/{id}"), b));
    assert_eq!(s.send(put(json!({ "title": "Hello", "body": "World", "publish": true }))).await.status, StatusCode::OK);
    assert_eq!(s.send(Req::get("/api/public/news")).await.json()["total"], 1, "no waiting for the cache");
    assert_eq!(s.send(Req::get("/api/public/home")).await.json()["news"][0]["title"], "Hello");
    assert_eq!(s.send(Req::get(&format!("/api/public/news/{id}"))).await.status, StatusCode::OK);

    // An edit that leaves the publish flag out keeps the post published.
    assert_eq!(s.send(put(json!({ "title": "Hello again", "body": "World", "pinned": true }))).await.status, StatusCode::OK);
    let j = s.send(Req::get(&format!("/api/public/news/{id}"))).await.json();
    assert_eq!(j["title"], "Hello again");
    assert_eq!(j["pinned"], true);

    assert_eq!(s.send(put(json!({ "title": "Hello again", "body": "World", "publish": false }))).await.status, StatusCode::OK);
    assert_eq!(s.send(Req::get(&format!("/api/public/news/{id}"))).await.status, StatusCode::NOT_FOUND);
    assert_eq!(s.send(Req::get("/api/public/home")).await.json()["news"].as_array().unwrap().len(), 0);

    assert_eq!(s.send(put(json!({ "title": "Hello again", "body": "World", "publish": true }))).await.status, StatusCode::OK);
    assert_eq!(s.send(with(&owner, Req::delete(&format!("/api/admin/news/{id}")))).await.status, StatusCode::OK);
    assert_eq!(s.send(Req::get("/api/public/news")).await.json()["total"], 0);
    assert_eq!(s.send(with(&owner, Req::delete(&format!("/api/admin/news/{id}")))).await.status, StatusCode::NOT_FOUND);
    assert_eq!(s.send(put(json!({ "title": "x", "body": "y" }))).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_public_cache_holds_for_30_seconds_and_any_edit_drops_it() {
    let s = people().await;
    let owner = who(&s, "steven", "owner-pass").await;
    s.fake.0.lock().unwrap().online = vec![online_row("Torvald", "Ronfaure", 1)];
    assert_eq!(s.send(Req::get("/api/public/home")).await.json()["online"], 1);
    // The game changes behind the cache: the old answer stays for 30 seconds.
    s.fake.0.lock().unwrap().online.clear();
    s.advance(29);
    assert_eq!(s.send(Req::get("/api/public/home")).await.json()["online"], 1);
    s.advance(2);
    assert_eq!(s.send(Req::get("/api/public/home")).await.json()["online"], 0);

    // An edit drops every cached answer, including the ones for other keys.
    let before = s.send(Req::get("/api/public/content/rules")).await.json();
    assert!(before["html"].as_str().unwrap().contains("hasn't written"));
    assert!(!s.state.public_cache.is_empty());
    s.fake.0.lock().unwrap().online = vec![online_row("Torvald", "Ronfaure", 1)];
    let r = s.send(with(&owner, Req::put("/api/admin/content/rules", json!({ "body": "# Be kind" })))).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(s.state.public_cache.is_empty());
    assert!(s.send(Req::get("/api/public/content/rules")).await.json()["html"].as_str().unwrap().contains("<h1>Be kind</h1>"));
    assert_eq!(s.send(Req::get("/api/public/home")).await.json()["online"], 1, "the edit dropped the stale home answer too");
}

#[tokio::test]
async fn unknown_pages_and_ids_cant_fill_the_cache() {
    let s = site().await;
    for i in 0..50 {
        let _ = s.send(Req::get(&format!("/api/public/news/{}", 1000 + i)).peer(2 + (i % 200) as u8)).await;
        let _ = s.send(Req::get(&format!("/api/public/news?page={}", 5 + i)).peer(2 + (i % 200) as u8)).await;
    }
    assert!(s.state.public_cache.len() <= 1, "only the empty first page may be kept, found {}", s.state.public_cache.len());
}

#[tokio::test]
async fn a_post_is_checked_for_length_and_shape() {
    let s = people().await;
    let owner = who(&s, "steven", "owner-pass").await;
    let post = |b: Value| async { s.send(with(&owner, Req::post("/api/admin/news", b))).await };
    assert_eq!(post(json!({ "title": "  ", "body": "x" })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(post(json!({ "title": "t".repeat(121), "body": "x" })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(post(json!({ "title": "two\nlines", "body": "x" })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(post(json!({ "title": "t".repeat(120), "body": "x" })).await.status, StatusCode::CREATED);
    assert_eq!(post(json!({ "title": "t", "body": "x".repeat(20_001) })).await.status, StatusCode::BAD_REQUEST);
    // 20,000 characters is a lot more than the 8 KB the other routes take, and goes through.
    assert_eq!(post(json!({ "title": "t", "body": "x".repeat(20_000) })).await.status, StatusCode::CREATED);
    assert_eq!(post(json!({ "title": "t", "body": "a\u{0}b" })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(post(json!({ "title": "t", "body": "x", "author": "a".repeat(41) })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(post(json!({ "title": "t", "body": "  ", "publish": true })).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(post(json!({ "title": "t", "body": "  " })).await.status, StatusCode::CREATED, "an empty draft is fine");
    // Not JSON of the right shape.
    assert!(post(json!({ "body": "x" })).await.status.is_client_error());
    // Far more than any post: the body limit stops it.
    assert_eq!(post(json!({ "title": "t", "body": "x".repeat(200_000) })).await.status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn the_author_is_a_typed_name_never_the_login() {
    let s = people().await;
    let owner = who(&s, "steven", "owner-pass").await;
    let a = new_post(&s, &owner, "One", "x", true, false).await;
    let r = s.send(with(&owner, Req::post("/api/admin/news", json!({ "title": "Two", "body": "x", "author": "  Halvard ", "publish": true })))).await;
    let b = r.json()["id"].as_i64().unwrap();
    assert_eq!(s.send(Req::get(&format!("/api/public/news/{a}"))).await.json()["author"], "Staff");
    assert_eq!(s.send(Req::get(&format!("/api/public/news/{b}"))).await.json()["author"], "Halvard");
    // The owner sees who edited, in the admin API only.
    assert_eq!(s.send(with(&owner, Req::get(&format!("/api/admin/news/{a}")))).await.json()["updated_by"], "steven");
}

#[tokio::test]
async fn content_blocks_default_save_and_reset() {
    let s = people().await;
    let owner = who(&s, "steven", "owner-pass").await;
    let home = s.send(Req::get("/api/public/home")).await.json();
    assert_eq!(home["title"], "Test Server", "the title is the server's name until changed");
    assert!(home["connect_html"].as_str().unwrap().contains("ask the server owner"));
    assert!(home["connect_address"].is_null(), "no address is made up");
    assert!(home["footer_html"].as_str().unwrap().contains("LandSandBoat"));

    let blocks = s.send(with(&owner, Req::get("/api/admin/content"))).await.json();
    let list = blocks["blocks"].as_array().unwrap();
    assert_eq!(list.iter().map(|b| b["key"].as_str().unwrap()).collect::<Vec<_>>(), ["home_title", "home_description", "connect", "rules", "footer"]);
    assert!(list.iter().all(|b| b["is_default"] == true));

    let put = |key: &str, body: &str| with(&owner, Req::put(&format!("/api/admin/content/{key}"), json!({ "body": body })));
    assert_eq!(s.send(put("home_title", "  Steven's Server ")).await.status, StatusCode::OK);
    assert_eq!(s.send(put("home_description", "A *small* server.")).await.status, StatusCode::OK);
    assert_eq!(s.send(put("footer", "Be nice.")).await.status, StatusCode::OK);
    let home = s.send(Req::get("/api/public/home")).await.json();
    assert_eq!(home["title"], "Steven's Server");
    assert!(home["description_html"].as_str().unwrap().contains("<em>small</em>"));
    assert!(home["footer_html"].as_str().unwrap().contains("Be nice."));
    let list = s.send(with(&owner, Req::get("/api/admin/content"))).await.json();
    let title = &list["blocks"][0];
    assert_eq!(title["is_default"], false);
    assert_eq!(title["updated_by"], "steven");
    assert_eq!(title["default"], "Test Server");

    // Reset puts the default back.
    assert_eq!(s.send(with(&owner, Req::delete("/api/admin/content/home_title"))).await.status, StatusCode::OK);
    assert_eq!(s.send(Req::get("/api/public/home")).await.json()["title"], "Test Server");
    // Resetting what was never edited changes nothing and isn't logged.
    let n = s.state.store.action_count().await.unwrap();
    assert_eq!(s.send(with(&owner, Req::delete("/api/admin/content/connect"))).await.status, StatusCode::OK);
    assert_eq!(s.state.store.action_count().await.unwrap(), n);
    // Unknown blocks.
    assert_eq!(s.send(put("nope", "x")).await.status, StatusCode::NOT_FOUND);
    assert_eq!(s.send(with(&owner, Req::delete("/api/admin/content/nope"))).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn content_blocks_have_size_limits() {
    let s = people().await;
    let owner = who(&s, "steven", "owner-pass").await;
    let put = |key: &str, body: &str| with(&owner, Req::put(&format!("/api/admin/content/{key}"), json!({ "body": body })));
    assert_eq!(s.send(put("home_title", &"t".repeat(81))).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(s.send(put("home_title", &"t".repeat(80))).await.status, StatusCode::OK);
    assert_eq!(s.send(put("home_title", "  ")).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(s.send(put("home_title", "two\nlines")).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(s.send(put("footer", &"f".repeat(401))).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(s.send(put("connect", &"c".repeat(2_001))).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(s.send(put("home_description", &"d".repeat(1_001))).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(s.send(put("rules", &"r".repeat(20_001))).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(s.send(put("rules", &"r".repeat(20_000))).await.status, StatusCode::OK);
    assert_eq!(s.send(put("rules", "a\u{7}b")).await.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_connect_address_comes_from_the_public_url_when_there_is_one() {
    let mut cfg = Config::for_tests();
    cfg.admin_secret = SECRET.into();
    cfg.public_url = "https://play.example.net:8443/".into();
    let s = site_with(cfg).await;
    let home = s.send(Req::get("/api/public/home")).await.json();
    assert_eq!(home["connect_address"], "play.example.net");
    assert!(home["connect_html"].as_str().unwrap().contains("address below"));
}

#[tokio::test]
async fn every_write_is_logged_without_whole_bodies() {
    let s = people().await;
    let owner = who(&s, "steven", "owner-pass").await;
    let secret_body = "a long secret body text that must not be copied into the log";
    let id = new_post(&s, &owner, "Log me", secret_body, false, false).await;
    let put = |b: Value| with(&owner, Req::put(&format!("/api/admin/news/{id}"), b));
    s.send(put(json!({ "title": "Log me", "body": secret_body, "publish": true }))).await;
    s.send(put(json!({ "title": "Log me", "body": secret_body, "pinned": true }))).await;
    s.send(put(json!({ "title": "Log me too", "body": secret_body, "pinned": true }))).await;
    s.send(put(json!({ "title": "Log me too", "body": secret_body, "pinned": true, "publish": false }))).await;
    s.send(with(&owner, Req::put("/api/admin/content/footer", json!({ "body": "new footer text" })))).await;
    s.send(with(&owner, Req::put("/api/admin/content/home_title", json!({ "body": "New Title" })))).await;
    s.send(with(&owner, Req::delete("/api/admin/content/footer"))).await;
    s.send(with(&owner, Req::delete(&format!("/api/admin/news/{id}")))).await;

    let log = s.send(with(&owner, Req::get("/api/admin/log?days=0&per=100"))).await.json();
    let rows = log["items"].as_array().unwrap();
    let mut got: Vec<(String, String)> = rows
        .iter()
        .filter(|r| r["action"].as_str().unwrap().starts_with("news") || r["action"].as_str().unwrap().starts_with("content"))
        .map(|r| (r["action"].as_str().unwrap().to_string(), r["target_kind"].as_str().unwrap().to_string()))
        .collect();
    got.reverse();
    let want: Vec<(&str, &str)> = vec![
        ("news_create", "news"), ("news_publish", "news"), ("news_pin", "news"), ("news_edit", "news"), ("news_unpublish", "news"),
        ("content_edit", "content"), ("content_edit", "content"), ("content_reset", "content"), ("news_delete", "news"),
    ];
    assert_eq!(got, want.iter().map(|(a, k)| (a.to_string(), k.to_string())).collect::<Vec<_>>());
    let text = log.to_string();
    assert!(!text.contains("secret body"), "a whole body reached the log: {text}");
    assert!(!text.contains("new footer text"), "{text}");
    // Titles and sizes are there.
    assert!(text.contains("Log me too") && text.contains("body_size"));
    assert!(rows.iter().all(|r| r["result"] == "ok"));
}
