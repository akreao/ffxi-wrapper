//! Behind the Caddy proxy: whose address counts, HTTPS-only behaviour, and the plain-HTTP guard.

mod common;
use axum::http::StatusCode;
use common::*;
use ffxi_web::config::Config;
use serde_json::json;

/// The proxy container is 10.0.0.50 in these tests.
const PROXY: u8 = 50;
const PUBLIC: &str = "https://ffxi.example.com";

async fn public_site() -> Site {
    let mut cfg = Config::for_tests();
    cfg.admin_secret = SECRET.into();
    cfg.public_url = PUBLIC.into();
    cfg.trusted_proxy = "10.0.0.50".into();
    let s = site_with(cfg).await;
    s.fake.add_account(1, "kaelyn", "hunter22", 0);
    s.fake.add_account(2, "steven", "owner-pass", 5);
    s
}

fn login(pw: &str) -> serde_json::Value {
    json!({ "login": "kaelyn", "password": pw })
}

/// A request as the proxy sends it: its own peer address, the site's Host and Origin.
fn via_proxy(mut r: Req, forwarded: &str) -> Req {
    r.origin = r.origin.map(|_| PUBLIC.into());
    r.peer(PROXY).host("ffxi.example.com").forwarded(forwarded)
}

#[tokio::test]
async fn the_forwarded_address_is_believed_only_from_the_proxy() {
    let s = public_site().await;
    // A stranger on the direct port (a private peer, so it may sign in) writes the header himself.
    for _ in 0..5 {
        s.send(Req::post("/api/auth/login", login("wrong-pw")).peer(9).forwarded("198.51.100.77")).await;
    }
    let log = s.state.store.actions(None, 50, false, true).await.unwrap();
    assert!(log.iter().all(|a| a.ip.as_deref() == Some("10.0.0.9")), "{:?}", log.iter().map(|a| a.ip.clone()).collect::<Vec<_>>());
}

#[tokio::test]
async fn through_the_proxy_the_address_it_appended_is_the_client() {
    let s = public_site().await;
    // The client's own entries (left) are ignored; the proxy's (right) is used.
    let r = s.send(via_proxy(Req::post("/api/auth/login", login("wrong-pw")), "1.1.1.1, 2.2.2.2, 198.51.100.20")).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let log = s.state.store.actions(None, 50, false, true).await.unwrap();
    assert_eq!(log[0].ip.as_deref(), Some("198.51.100.20"));
}

#[tokio::test]
async fn spoofing_the_left_most_entry_does_not_dodge_the_address_limit() {
    let s = public_site().await;
    // 30 attempts on made-up logins from one real address, each claiming a different origin.
    for i in 0..30 {
        let r = s
            .send(via_proxy(Req::post("/api/auth/login", json!({ "login": format!("nobody{i}"), "password": "x-wrong" })), &format!("9.9.9.{i}, 198.51.100.21")))
            .await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{i}");
    }
    let r = s.send(via_proxy(Req::post("/api/auth/login", json!({ "login": "nobody-new", "password": "x-wrong" })), "9.9.9.200, 198.51.100.21")).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    // Another real address is untouched.
    let r = s.send(via_proxy(Req::post("/api/auth/login", login("hunter22")), "198.51.100.22")).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
}

#[tokio::test]
async fn a_proxy_request_without_a_usable_address_gets_no_address_bucket() {
    let s = public_site().await;
    let r = s.send(via_proxy(Req::post("/api/auth/login", login("hunter22")), "garbage")).await;
    assert_eq!(r.status, StatusCode::OK);
    let log = s.state.store.actions(None, 50, false, true).await.unwrap();
    assert_eq!(log[0].ip.as_deref(), Some("unknown (proxy)"));
}

#[tokio::test]
async fn over_https_the_site_sends_hsts_and_secure_cookies() {
    let s = public_site().await;
    let r = s.send(via_proxy(Req::post("/api/auth/login", login("hunter22")), "198.51.100.30")).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.header("strict-transport-security").unwrap().starts_with("max-age="));
    assert!(r.headers.get("set-cookie").unwrap().to_str().unwrap().contains("; Secure"));
    // The direct port is plain HTTP: no HSTS there, and a cookie browsers will keep.
    let r = s.send(Req::post("/api/auth/login", login("hunter22")).peer(9)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(!r.headers.get("set-cookie").unwrap().to_str().unwrap().contains("Secure"));
    let direct = s.send(Req::get("/healthz")).await;
    assert!(direct.header("strict-transport-security").is_none());
}

#[tokio::test]
async fn another_host_through_the_proxy_is_refused() {
    let s = public_site().await;
    let r = s.send(Req::get("/healthz").peer(PROXY).host("evil.example.net").forwarded("198.51.100.40")).await;
    assert_eq!(r.status, StatusCode::MISDIRECTED_REQUEST);
    let r = s.send(Req::get("/healthz").peer(PROXY).host("FFXI.example.com:443").forwarded("198.51.100.40")).await;
    assert_eq!(r.status, StatusCode::OK);
    // The direct port answers to any name (an IP address, a LAN name).
    let r = s.send(Req::get("/healthz").peer(9).host("192.168.1.5:54080")).await;
    assert_eq!(r.status, StatusCode::OK);
}

#[tokio::test]
async fn plain_http_sign_in_from_a_public_address_is_refused() {
    let s = public_site().await;
    // On the direct port from the internet: refused, and no attempt is counted or logged as a password try.
    let r = s.send(Req::post("/api/auth/login", login("hunter22")).from("198.51.100.9")).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert!(r.text().contains(PUBLIC), "{}", r.text());
    assert!(r.cookie().is_none());
    // A forged header doesn't turn a public peer into the proxy or a private client.
    let r = s.send(Req::post("/api/auth/login", login("hunter22")).from("198.51.100.9").forwarded("10.0.0.5")).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    // This computer, the LAN and Tailscale may use the direct port.
    for ip in ["127.0.0.1", "192.168.1.20", "100.101.102.103", "172.19.0.1"] {
        let r = s.send(Req::post("/api/auth/login", login("hunter22")).from(ip)).await;
        assert_eq!(r.status, StatusCode::OK, "{ip}: {}", r.text());
    }
    // Reading pages over plain HTTP from anywhere is fine.
    assert_eq!(s.send(Req::get("/api/public/home").from("198.51.100.9")).await.status, StatusCode::OK);
    // Through the proxy (HTTPS) a public client signs in.
    let r = s.send(via_proxy(Req::post("/api/auth/login", login("hunter22")), "198.51.100.9")).await;
    assert_eq!(r.status, StatusCode::OK);
}

#[tokio::test]
async fn without_a_public_address_nothing_changes() {
    let mut cfg = Config::for_tests();
    cfg.admin_secret = SECRET.into();
    let s = site_with(cfg).await;
    s.fake.add_account(1, "kaelyn", "hunter22", 0);
    let r = s.send(Req::post("/api/auth/login", login("hunter22")).forwarded("1.2.3.4")).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.header("strict-transport-security").is_none());
    assert!(!r.headers.get("set-cookie").unwrap().to_str().unwrap().contains("Secure"));
    let log = s.state.store.actions(None, 50, false, true).await.unwrap();
    assert_eq!(log[0].ip.as_deref(), Some("10.0.0.2"));
}
