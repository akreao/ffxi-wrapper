//! A fake game database and request helpers for the router tests.

#![allow(dead_code)]

use async_trait::async_trait;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use ffxi_web::auth;
use ffxi_web::config::Config;
use ffxi_web::game::{DbError, DbResult, Game};
use ffxi_web::model::*;
use ffxi_web::status::{ServiceState, StaticProbe};
use ffxi_web::store::Store;
use ffxi_web::{app_state, Parts, Shared};
use http_body_util::BodyExt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

pub const SECRET: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
pub const NOW: i64 = 1_800_000_000;

#[derive(Clone)]
pub struct FakeAccount {
    pub id: i64,
    pub login: String,
    pub hash: String,
    pub status: i64,
    pub totp: Option<TotpRow>,
}

#[derive(Clone)]
pub struct FakeChar {
    pub charid: i64,
    pub accid: i64,
    pub name: String,
    pub gm: i64,
}

#[derive(Default)]
pub struct Data {
    pub accounts: Vec<FakeAccount>,
    pub chars: Vec<FakeChar>,
    pub online: Vec<OnlineRow>,
    pub down: bool,
    pub not_ready: bool,
    /// (accid, charid) of every account-scoped character query, in order.
    pub char_queries: Vec<(i64, i64)>,
    pub account_queries: Vec<i64>,
}

#[derive(Clone, Default)]
pub struct Fake(pub Arc<Mutex<Data>>);

impl Fake {
    pub fn add_account(&self, id: i64, login: &str, password: &str, gm: i64) {
        let hash = auth::hash_password(password, 4).unwrap();
        let mut d = self.0.lock().unwrap();
        d.accounts.push(FakeAccount { id, login: login.into(), hash, status: 1, totp: None });
        d.chars.push(FakeChar { charid: id * 10, accid: id, name: format!("{login}char"), gm });
    }
    pub fn set_status(&self, id: i64, status: i64) {
        self.0.lock().unwrap().accounts.iter_mut().find(|a| a.id == id).unwrap().status = status;
    }
    pub fn set_gm(&self, id: i64, gm: i64) {
        for c in self.0.lock().unwrap().chars.iter_mut().filter(|c| c.accid == id) {
            c.gm = gm;
        }
    }
    pub fn set_hash(&self, id: i64, hash: &str) {
        self.0.lock().unwrap().accounts.iter_mut().find(|a| a.id == id).unwrap().hash = hash.into();
    }
    pub fn set_totp(&self, id: i64, secret: &str) {
        self.0.lock().unwrap().accounts.iter_mut().find(|a| a.id == id).unwrap().totp = Some(TotpRow { validated: true, secret: secret.into() });
    }
    pub fn hash_of(&self, id: i64) -> String {
        self.0.lock().unwrap().accounts.iter().find(|a| a.id == id).unwrap().hash.clone()
    }
    fn check(&self) -> DbResult<()> {
        let d = self.0.lock().unwrap();
        if d.not_ready {
            return Err(DbError::NotReady);
        }
        if d.down {
            return Err(DbError::Failed("down".into()));
        }
        Ok(())
    }
}

fn summary(c: &FakeChar) -> CharSummary {
    CharSummary { charid: c.charid, name: c.name.clone(), zone_id: 230, zone: Some("Southern San dOria".into()), nation: 0, mjob: Some(1), mlvl: Some(10), sjob: Some(2), slvl: Some(5), playtime: 7200, last_logout: Some(NOW - 100), online: false }
}

#[async_trait]
impl Game for Fake {
    async fn ping(&self) -> DbResult<()> { self.check() }
    async fn account_by_login(&self, login: &str) -> DbResult<Option<AccountAuth>> {
        self.check()?;
        Ok(self.0.lock().unwrap().accounts.iter().find(|a| a.login.eq_ignore_ascii_case(login.trim_end())).map(|a| AccountAuth { id: a.id, login: a.login.clone(), status: a.status, password: a.hash.clone() }))
    }
    async fn account_by_id(&self, id: i64) -> DbResult<Option<AccountAuth>> {
        self.check()?;
        Ok(self.0.lock().unwrap().accounts.iter().find(|a| a.id == id).map(|a| AccountAuth { id: a.id, login: a.login.clone(), status: a.status, password: a.hash.clone() }))
    }
    async fn totp(&self, accid: i64) -> DbResult<Option<TotpRow>> {
        self.check()?;
        Ok(self.0.lock().unwrap().accounts.iter().find(|a| a.id == accid).and_then(|a| a.totp.clone()))
    }
    async fn standing(&self, accid: i64) -> DbResult<Option<Standing>> {
        self.check()?;
        let d = self.0.lock().unwrap();
        Ok(d.accounts.iter().find(|a| a.id == accid).map(|a| Standing { status: a.status, gm: d.chars.iter().filter(|c| c.accid == accid).map(|c| c.gm).max().unwrap_or(0) }))
    }
    async fn my_account(&self, accid: i64) -> DbResult<Option<MyAccount>> {
        self.check()?;
        let mut d = self.0.lock().unwrap();
        d.account_queries.push(accid);
        Ok(d.accounts.iter().find(|a| a.id == accid).map(|a| MyAccount { login: a.login.clone(), created: Some(NOW - 1000), status: a.status, has_totp: a.totp.is_some() }))
    }
    async fn my_characters(&self, accid: i64) -> DbResult<Vec<CharSummary>> {
        self.check()?;
        let mut d = self.0.lock().unwrap();
        d.account_queries.push(accid);
        Ok(d.chars.iter().filter(|c| c.accid == accid).map(summary).collect())
    }
    async fn my_character(&self, accid: i64, charid: i64) -> DbResult<Option<CharDetail>> {
        self.check()?;
        let mut d = self.0.lock().unwrap();
        d.char_queries.push((accid, charid));
        Ok(d.chars.iter().find(|c| c.accid == accid && c.charid == charid).map(|c| CharDetail {
            summary: summary(c),
            gil: 1234,
            cap: 50,
            jobs: JOBS.iter().enumerate().map(|(i, j)| JobLevel { job: j, level: if i < 2 { 10 } else { 0 }, exp: 0 }).collect(),
        }))
    }
    async fn my_inventory(&self, accid: i64, charid: i64) -> DbResult<Option<Vec<InventoryItem>>> {
        self.check()?;
        let mut d = self.0.lock().unwrap();
        d.char_queries.push((accid, charid));
        Ok(d.chars.iter().find(|c| c.accid == accid && c.charid == charid).map(|_| vec![InventoryItem { location: 0, slot: 1, item_id: 4116, name: Some("Hi-Potion".into()), quantity: 3 }]))
    }
    async fn set_password(&self, accid: i64, hash: &str) -> DbResult<()> {
        self.check()?;
        self.0.lock().unwrap().accounts.iter_mut().find(|a| a.id == accid).unwrap().hash = hash.into();
        Ok(())
    }
    async fn counts(&self) -> DbResult<Counts> {
        self.check()?;
        let d = self.0.lock().unwrap();
        Ok(Counts { accounts: d.accounts.len() as i64, characters: d.chars.len() as i64, online: d.online.len() as i64, banned: d.accounts.iter().filter(|a| a.status & 1 == 0).count() as i64 })
    }
    async fn online(&self) -> DbResult<Vec<OnlineRow>> {
        self.check()?;
        Ok(self.0.lock().unwrap().online.clone())
    }
    async fn public_online(&self) -> DbResult<i64> {
        self.check()?;
        Ok(self.0.lock().unwrap().online.iter().filter(|r| r.gm == 0 && !r.gm_hidden && !r.connecting).count() as i64)
    }
}

pub fn online_row(name: &str, zone: &str, addr: i64) -> OnlineRow {
    OnlineRow { accid: 1, login: "someone".into(), charid: 1, name: name.into(), zone: Some(zone.into()), mjob: Some(7), mlvl: Some(52), sjob: Some(1), slvl: Some(26), gm: 0, connecting: false, gm_hidden: false, client_addr: addr }
}

pub struct Site {
    pub router: Router,
    pub state: Shared,
    pub fake: Fake,
    pub clock: Arc<AtomicI64>,
}

pub async fn site_with(cfg: Config) -> Site {
    let fake = Fake::default();
    let clock = Arc::new(AtomicI64::new(NOW));
    let c = clock.clone();
    let state = app_state(Parts {
        cfg,
        game: Arc::new(fake.clone()),
        store: Store::memory().await.unwrap(),
        probe: Arc::new(StaticProbe(vec![ServiceState { id: "world", name: "World", state: "up" }])),
        clock: Arc::new(move || c.load(Ordering::SeqCst)),
        dummy_hash: auth::hash_password("dummy", 4).unwrap(),
        bcrypt_cost: 4,
    });
    Site { router: ffxi_web::router(state.clone()), state, fake, clock }
}

pub async fn site() -> Site {
    let mut cfg = Config::for_tests();
    cfg.admin_secret = SECRET.into();
    site_with(cfg).await
}

impl Site {
    pub fn advance(&self, secs: i64) {
        self.clock.fetch_add(secs, Ordering::SeqCst);
    }

    pub async fn send(&self, req: Req) -> Resp {
        let mut b = Request::builder().method(req.method.clone()).uri(&req.path).header(header::HOST, "site.test");
        if let Some(c) = &req.cookie {
            b = b.header(header::COOKIE, format!("web_session={c}"));
        }
        if let Some(t) = &req.csrf {
            b = b.header("x-csrf-token", t);
        }
        if let Some(o) = &req.origin {
            b = b.header(header::ORIGIN, o);
        }
        let body = match &req.body {
            Some(j) => {
                if req.content_type {
                    b = b.header(header::CONTENT_TYPE, "application/json");
                }
                Body::from(j.to_string())
            }
            None => Body::empty(),
        };
        let mut r = b.body(body).unwrap();
        r.extensions_mut().insert(ConnectInfo(SocketAddr::from(([10, 0, 0, req.peer], 5555))));
        let res = self.router.clone().oneshot(r).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        Resp { status, headers, bytes: bytes.to_vec() }
    }

    /// Signs in and returns (cookie, csrf).
    pub async fn sign_in(&self, login: &str, password: &str) -> (String, String) {
        let r = self.send(Req::post("/api/auth/login", serde_json::json!({ "login": login, "password": password }))).await;
        assert_eq!(r.status, StatusCode::OK, "sign-in failed: {}", r.text());
        (r.cookie().expect("a session cookie"), r.json()["csrf"].as_str().unwrap().to_string())
    }
}

pub struct Req {
    pub method: Method,
    pub path: String,
    pub body: Option<serde_json::Value>,
    pub cookie: Option<String>,
    pub csrf: Option<String>,
    pub origin: Option<String>,
    pub content_type: bool,
    pub peer: u8,
}

impl Req {
    pub fn get(path: &str) -> Req {
        Req { method: Method::GET, path: path.into(), body: None, cookie: None, csrf: None, origin: None, content_type: true, peer: 2 }
    }
    pub fn post(path: &str, body: serde_json::Value) -> Req {
        Req { method: Method::POST, body: Some(body), origin: Some("http://site.test".into()), ..Req::get(path) }
    }
    pub fn session(mut self, cookie: &str, csrf: &str) -> Req {
        self.cookie = Some(cookie.into());
        self.csrf = Some(csrf.into());
        self
    }
    pub fn cookie(mut self, cookie: &str) -> Req {
        self.cookie = Some(cookie.into());
        self
    }
    pub fn peer(mut self, last: u8) -> Req {
        self.peer = last;
        self
    }
}

pub struct Resp {
    pub status: StatusCode,
    pub headers: axum::http::HeaderMap,
    pub bytes: Vec<u8>,
}

impl Resp {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.bytes).unwrap_or(serde_json::Value::Null)
    }
    /// The value of the session cookie this response sets.
    pub fn cookie(&self) -> Option<String> {
        let v = self.headers.get(header::SET_COOKIE)?.to_str().ok()?;
        v.strip_prefix("web_session=")?.split(';').next().map(String::from)
    }
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers.get(name).and_then(|v| v.to_str().ok()).map(String::from)
    }
}
