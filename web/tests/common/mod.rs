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
    pub ban: Option<BanRecord>,
    pub trust_tokens: bool,
}

#[derive(Clone)]
pub struct FakeChar {
    pub charid: i64,
    pub accid: i64,
    pub name: String,
    pub gm: i64,
    pub online: bool,
}

/// A character as the public pages see the game database: every field a rule looks at.
#[derive(Clone)]
pub struct Person {
    pub name: String,
    /// 0 is a character without an account.
    pub accid: i64,
    pub banned: bool,
    pub gm: i64,
    pub gm_hidden: bool,
    pub anon: bool,
    pub in_game: bool,
    pub connecting: bool,
    pub nation: i64,
    pub zone: Option<String>,
    pub mjob: i64,
    pub mlvl: i64,
    pub sjob: i64,
    pub slvl: i64,
    /// Levels and experience per job, in `JOBS` order.
    pub jobs: [i64; 22],
    pub exp: [i64; 22],
    /// (skill id, value in tenths)
    pub skills: Vec<(i64, i64)>,
    pub playtime: i64,
}

impl Person {
    pub fn new(name: &str) -> Person {
        Person { name: name.into(), accid: 1, banned: false, gm: 0, gm_hidden: false, anon: false, in_game: false, connecting: false, nation: 1, zone: None, mjob: 1, mlvl: 1, sjob: 0, slvl: 1, jobs: [1; 22], exp: [0; 22], skills: vec![], playtime: 0 }
    }
    /// In game as `job` (1-22) at `lvl`, in `zone`.
    pub fn online(mut self, job: i64, lvl: i64, zone: &str) -> Person {
        self.in_game = true;
        self.zone = Some(zone.into());
        self.mjob = job;
        self.mlvl = lvl;
        self.sjob = 1;
        self.slvl = lvl / 2;
        self.jobs[job as usize - 1] = lvl;
        self
    }
    pub fn level(mut self, job: i64, lvl: i64) -> Person {
        self.jobs[job as usize - 1] = lvl;
        self
    }
    pub fn skill(mut self, id: i64, tenths: i64) -> Person {
        self.skills.push((id, tenths));
        self
    }
    pub fn played(mut self, secs: i64) -> Person {
        self.playtime = secs;
        self
    }
    pub fn gm(mut self, level: i64) -> Person {
        self.gm = level;
        self
    }
    pub fn hidden(mut self) -> Person {
        self.gm_hidden = true;
        self
    }
    pub fn anon(mut self) -> Person {
        self.anon = true;
        self
    }
    pub fn banned(mut self) -> Person {
        self.banned = true;
        self
    }
    pub fn no_account(mut self) -> Person {
        self.accid = 0;
        self
    }
}

#[derive(Default)]
pub struct Data {
    pub people: Vec<Person>,
    /// How many times the public queries ran, to show the 30 s cache.
    pub public_reads: usize,
    pub accounts: Vec<FakeAccount>,
    pub chars: Vec<FakeChar>,
    pub online: Vec<OnlineRow>,
    pub down: bool,
    /// Making an account fails, as when the database refuses the insert.
    pub fail_create: bool,
    pub not_ready: bool,
    /// (accid, charid) of every account-scoped character query, in order.
    pub char_queries: Vec<(i64, i64)>,
    pub account_queries: Vec<i64>,
    pub tickets: Vec<FakeTicket>,
    pub items: Vec<ItemDetailRaw>,
    /// Every item search the database was asked for.
    pub item_queries: Vec<ItemQuery>,
}

#[derive(Clone, Debug)]
pub struct FakeTicket {
    pub id: i64,
    pub charid: i64,
    pub message: String,
    pub response: Option<String>,
    pub responded: Option<i64>,
    pub closed: Option<i64>,
}

#[derive(Clone, Default)]
pub struct Fake(pub Arc<Mutex<Data>>);

impl Fake {
    pub fn add_account(&self, id: i64, login: &str, password: &str, gm: i64) {
        let hash = auth::hash_password(password, 4).unwrap();
        let mut d = self.0.lock().unwrap();
        d.accounts.push(FakeAccount { id, login: login.into(), hash, status: 1, totp: None, ban: None, trust_tokens: true });
        d.chars.push(FakeChar { charid: id * 10, accid: id, name: format!("{login}char"), gm, online: false });
    }
    pub fn add_person(&self, p: Person) {
        self.0.lock().unwrap().people.push(p);
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
    pub fn set_online(&self, charid: i64, online: bool) {
        self.0.lock().unwrap().chars.iter_mut().find(|c| c.charid == charid).unwrap().online = online;
    }
    pub fn add_char(&self, charid: i64, accid: i64, name: &str, gm: i64) {
        self.0.lock().unwrap().chars.push(FakeChar { charid, accid, name: name.into(), gm, online: false });
    }
    pub fn account(&self, id: i64) -> FakeAccount {
        self.0.lock().unwrap().accounts.iter().find(|a| a.id == id).unwrap().clone()
    }
    pub fn gm_of_char(&self, charid: i64) -> i64 {
        self.0.lock().unwrap().chars.iter().find(|c| c.charid == charid).unwrap().gm
    }
    /// A ban made outside the site: status banned plus a record.
    pub fn ban_directly(&self, id: i64, record: Option<BanRecord>) {
        let mut d = self.0.lock().unwrap();
        let a = d.accounts.iter_mut().find(|a| a.id == id).unwrap();
        a.status = 2;
        a.ban = record;
    }
    /// An open ticket filed by `charid`; returns its id.
    pub fn add_ticket(&self, charid: i64, message: &str) -> i64 {
        let mut d = self.0.lock().unwrap();
        let id = d.tickets.len() as i64 + 1;
        d.tickets.push(FakeTicket { id, charid, message: message.into(), response: None, responded: None, closed: None });
        id
    }
    pub fn ticket(&self, id: i64) -> FakeTicket {
        self.0.lock().unwrap().tickets.iter().find(|t| t.id == id).unwrap().clone()
    }
    pub fn add_item(&self, item: ItemDetailRaw) {
        self.0.lock().unwrap().items.push(item);
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
    CharSummary { charid: c.charid, name: c.name.clone(), zone_id: 230, zone: Some("Southern San dOria".into()), nation: 0, mjob: Some(1), mlvl: Some(10), sjob: Some(2), slvl: Some(5), playtime: 7200, last_logout: Some(NOW - 100), online: c.online }
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
        Ok(d.accounts.iter().find(|a| a.id == accid).map(|a| Standing { status: a.status, login: a.login.clone(), created: fake_created(a.id) }))
    }
    async fn staff_by_gm_level(&self) -> DbResult<Vec<(i64, String, Option<i64>, i64)>> {
        self.check()?;
        let d = self.0.lock().unwrap();
        Ok(d.accounts
            .iter()
            .map(|a| (a.id, a.login.clone(), fake_created(a.id), d.chars.iter().filter(|c| c.accid == a.id).map(|c| c.gm).max().unwrap_or(0)))
            .filter(|(_, _, _, gm)| *gm >= 3)
            .collect())
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
    async fn create_account(&self, login: &str, hash: &str) -> DbResult<Created> {
        self.check()?;
        let mut d = self.0.lock().unwrap();
        if d.fail_create {
            return Err(DbError::Failed("insert failed".into()));
        }
        if d.accounts.iter().any(|a| a.login.eq_ignore_ascii_case(login)) {
            return Ok(Created::Taken);
        }
        let id = (d.accounts.iter().map(|a| a.id).max().unwrap_or(0) + 1).max(1000);
        d.accounts.push(FakeAccount { id, login: login.into(), hash: hash.into(), status: 1, totp: None, ban: None, trust_tokens: true });
        Ok(Created::Account(id))
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
    async fn public_online(&self) -> DbResult<Vec<PublicPlayer>> {
        tokio::task::yield_now().await; // a real query takes time, so requests overlap
        self.check()?;
        let mut d = self.0.lock().unwrap();
        d.public_reads += 1;
        let mut out: Vec<PublicPlayer> = d
            .people
            .iter()
            .filter(|p| p.in_game && !p.connecting && !p.banned && p.accid != 0 && p.gm == 0 && !p.gm_hidden)
            .map(|p| PublicPlayer {
                name: p.name.clone(),
                nation: p.nation,
                zone: if p.anon { None } else { p.zone.as_deref().map(zone_name) },
                mjob: (!p.anon).then_some(p.mjob),
                mlvl: (!p.anon).then_some(p.mlvl),
                sjob: (!p.anon).then_some(p.sjob),
                slvl: (!p.anon).then_some(p.slvl),
                anonymous: p.anon,
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }
    async fn public_ranking(&self, board: Board) -> DbResult<Vec<RankRow>> {
        tokio::task::yield_now().await;
        self.check()?;
        let mut d = self.0.lock().unwrap();
        d.public_reads += 1;
        // (sort key descending, name, row without its rank)
        let mut rows: Vec<((i64, i64), RankRow)> = Vec::new();
        for p in d.people.iter().filter(|p| p.accid != 0 && !p.banned && p.gm == 0 && !p.gm_hidden) {
            let mk = |job: Option<i64>, value: i64| RankRow { rank: 0, name: p.name.clone(), nation: p.nation, job, value };
            match board {
                Board::Level if !p.anon => {
                    let (n, lvl) = p.jobs.iter().enumerate().fold((0, -1), |b, (n, l)| if *l > b.1 { (n, *l) } else { b });
                    rows.push(((lvl, 0), mk(Some(n as i64 + 1), lvl)));
                }
                Board::Job(n) if !p.anon && p.jobs[n] > 1 => rows.push(((p.jobs[n], p.exp[n]), mk(Some(n as i64 + 1), p.jobs[n]))),
                Board::Craft(id) => {
                    if let Some((_, v)) = p.skills.iter().find(|(s, v)| *s == id && *v > 0) {
                        rows.push(((*v, 0), mk(None, v / 10)));
                    }
                }
                Board::Playtime if p.playtime >= 3600 => rows.push(((p.playtime, 0), mk(None, p.playtime / 3600))),
                _ => {}
            }
        }
        rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.name.cmp(&b.1.name)));
        Ok(rows.into_iter().take(RANKING_TOP).enumerate().map(|(i, (_, mut r))| { r.rank = i as i64 + 1; r }).collect())
    }

    async fn admin_accounts(&self, q: &AccountQuery) -> DbResult<Page<AccountRow>> {
        self.check()?;
        let d = self.0.lock().unwrap();
        let needle = q.q.to_lowercase();
        let mut rows: Vec<AccountRow> = d
            .accounts
            .iter()
            .map(|a| {
                let chars: Vec<&FakeChar> = d.chars.iter().filter(|c| c.accid == a.id).collect();
                AccountRow {
                    id: a.id,
                    login: a.login.clone(),
                    status: a.status,
                    created: fake_created(a.id),
                    last_login: Some(NOW - 10 * a.id),
                    characters: chars.iter().map(|c| c.name.clone()).collect(),
                    gm: chars.iter().map(|c| c.gm).max().unwrap_or(0),
                    online: chars.iter().any(|c| c.online),
                }
            })
            .filter(|r| needle.is_empty() || r.login.to_lowercase().contains(&needle) || r.characters.iter().any(|c| c.to_lowercase().contains(&needle)))
            .filter(|r| match q.filter {
                AccountFilter::All => true,
                AccountFilter::Online => r.online,
                AccountFilter::Banned => r.status & 1 == 0,
            })
            .collect();
        rows.sort_by(|a, b| match q.sort {
            AccountSort::Login => a.login.cmp(&b.login),
            AccountSort::Id => a.id.cmp(&b.id),
            AccountSort::Created => a.created.cmp(&b.created),
            AccountSort::LastLogin => a.last_login.cmp(&b.last_login),
        });
        if q.desc {
            rows.reverse();
        }
        let total = rows.len() as i64;
        let items = rows.into_iter().skip(((q.page - 1) * q.per) as usize).take(q.per as usize).collect();
        Ok(Page { items, total })
    }
    async fn admin_account(&self, id: i64) -> DbResult<Option<AdminAccount>> {
        self.check()?;
        let d = self.0.lock().unwrap();
        Ok(d.accounts.iter().find(|a| a.id == id).map(|a| {
            let chars: Vec<AdminChar> = d.chars.iter().filter(|c| c.accid == id).map(|c| AdminChar { summary: summary(c), accid: id, login: a.login.clone(), gm: c.gm }).collect();
            AdminAccount {
                id,
                login: a.login.clone(),
                status: a.status,
                created: Some(NOW - 1000),
                last_login: Some(NOW - 10),
                gm: chars.iter().map(|c| c.gm).max().unwrap_or(0),
                has_totp: a.totp.is_some(),
                online: chars.iter().any(|c| c.summary.online),
                ban: if a.status & 1 == 0 { a.ban.clone() } else { None },
                characters: chars,
            }
        }))
    }
    async fn admin_characters(&self, q: &CharQuery) -> DbResult<Page<AdminChar>> {
        self.check()?;
        let d = self.0.lock().unwrap();
        let needle = q.q.to_lowercase();
        let mut rows: Vec<AdminChar> = d
            .chars
            .iter()
            .filter(|c| needle.is_empty() || c.name.to_lowercase().contains(&needle))
            .filter(|c| !q.online || c.online)
            .map(|c| AdminChar { summary: summary(c), accid: c.accid, login: d.accounts.iter().find(|a| a.id == c.accid).unwrap().login.clone(), gm: c.gm })
            .collect();
        rows.sort_by(|a, b| match q.sort {
            CharSort::Name => a.summary.name.cmp(&b.summary.name),
            CharSort::Account => a.login.cmp(&b.login),
            CharSort::Playtime => a.summary.playtime.cmp(&b.summary.playtime),
            CharSort::LastLogout => a.summary.last_logout.cmp(&b.summary.last_logout),
        });
        if q.desc {
            rows.reverse();
        }
        let total = rows.len() as i64;
        let items = rows.into_iter().skip(((q.page - 1) * q.per) as usize).take(q.per as usize).collect();
        Ok(Page { items, total })
    }
    async fn admin_character(&self, charid: i64) -> DbResult<Option<AdminCharDetail>> {
        self.check()?;
        let d = self.0.lock().unwrap();
        Ok(d.chars.iter().find(|c| c.charid == charid).map(|c| AdminCharDetail {
            detail: CharDetail {
                summary: summary(c),
                gil: 1234,
                cap: 50,
                jobs: JOBS.iter().enumerate().map(|(i, j)| JobLevel { job: j, level: if i < 2 { 10 } else { 0 }, exp: 0 }).collect(),
            },
            accid: c.accid,
            login: d.accounts.iter().find(|a| a.id == c.accid).unwrap().login.clone(),
            gm: c.gm,
            created: Some(NOW - 5000),
        }))
    }
    async fn admin_inventory(&self, charid: i64) -> DbResult<Option<Vec<InventoryItem>>> {
        self.check()?;
        let d = self.0.lock().unwrap();
        Ok(d.chars.iter().find(|c| c.charid == charid).map(|_| vec![InventoryItem { location: 0, slot: 1, item_id: 4116, name: Some("Hi-Potion".into()), quantity: 3 }]))
    }
    async fn admin_bans(&self, q: &str, page: i64, per: i64) -> DbResult<Page<BanRow>> {
        self.check()?;
        let d = self.0.lock().unwrap();
        let needle = q.to_lowercase();
        let rows: Vec<BanRow> = d
            .accounts
            .iter()
            .filter(|a| a.status & 1 == 0)
            .map(|a| BanRow { accid: a.id, login: a.login.clone(), characters: d.chars.iter().filter(|c| c.accid == a.id).map(|c| c.name.clone()).collect(), ban: a.ban.clone() })
            .filter(|r| needle.is_empty() || r.login.to_lowercase().contains(&needle) || r.characters.iter().any(|c| c.to_lowercase().contains(&needle)))
            .collect();
        let total = rows.len() as i64;
        Ok(Page { items: rows.into_iter().skip(((page - 1) * per) as usize).take(per as usize).collect(), total })
    }
    async fn ban_account(&self, id: i64, until: Option<i64>, reason: &str, no_gm: bool) -> DbResult<Acted> {
        self.check()?;
        let mut d = self.0.lock().unwrap();
        if no_gm && d.chars.iter().any(|c| c.accid == id && c.gm > 0) {
            return Ok(Acted::HasGm);
        }
        let Some(a) = d.accounts.iter_mut().find(|a| a.id == id) else { return Ok(Acted::NotFound) };
        a.status = 2;
        a.trust_tokens = false;
        a.ban = Some(BanRecord { banned_at: Some(NOW), until, reason: Some(reason.into()) });
        Ok(Acted::Done)
    }
    async fn unban_account(&self, id: i64, no_gm: bool) -> DbResult<Acted> {
        self.check()?;
        let mut d = self.0.lock().unwrap();
        if no_gm && d.chars.iter().any(|c| c.accid == id && c.gm > 0) {
            return Ok(Acted::HasGm);
        }
        let Some(a) = d.accounts.iter_mut().find(|a| a.id == id) else { return Ok(Acted::NotFound) };
        if a.status & 1 == 1 {
            return Ok(Acted::NoChange);
        }
        a.status = 1;
        a.ban = None;
        Ok(Acted::Done)
    }
    async fn reset_password(&self, id: i64, hash: &str) -> DbResult<Acted> {
        self.check()?;
        let mut d = self.0.lock().unwrap();
        let Some(a) = d.accounts.iter_mut().find(|a| a.id == id) else { return Ok(Acted::NotFound) };
        a.hash = hash.into();
        Ok(Acted::Done)
    }
    async fn reset_totp(&self, id: i64) -> DbResult<Acted> {
        self.check()?;
        let mut d = self.0.lock().unwrap();
        let Some(a) = d.accounts.iter_mut().find(|a| a.id == id) else { return Ok(Acted::NotFound) };
        a.totp = None;
        a.trust_tokens = false;
        Ok(Acted::Done)
    }
    async fn set_gm_level(&self, charid: i64, level: i64) -> DbResult<Acted> {
        self.check()?;
        let mut d = self.0.lock().unwrap();
        let Some(c) = d.chars.iter_mut().find(|c| c.charid == charid && c.accid != 0) else { return Ok(Acted::NotFound) };
        c.gm = level;
        Ok(Acted::Done)
    }
    async fn lift_expired_bans(&self, now: i64) -> DbResult<Vec<(i64, String)>> {
        self.check()?;
        let mut d = self.0.lock().unwrap();
        let mut out = vec![];
        for a in d.accounts.iter_mut() {
            if a.status & 1 == 0 && a.ban.as_ref().and_then(|b| b.until).is_some_and(|u| u <= now) {
                a.status = 1;
                a.ban = None;
                out.push((a.id, a.login.clone()));
            }
        }
        Ok(out)
    }
    async fn admin_tickets(&self, q: &TicketQuery) -> DbResult<Page<Ticket>> {
        self.check()?;
        let d = self.0.lock().unwrap();
        let needle = q.q.to_lowercase();
        let mut rows: Vec<Ticket> = d
            .tickets
            .iter()
            .filter(|t| match q.filter {
                TicketFilter::Open => t.closed.is_none() && t.response.is_none(),
                TicketFilter::Answered => t.closed.is_none() && t.response.is_some(),
                TicketFilter::Read => t.closed.is_some() && t.response.is_some(),
                TicketFilter::Closed => t.closed.is_some() && t.response.is_none(),
                TicketFilter::All => true,
            })
            .map(|t| fake_ticket(&d, t))
            .filter(|t| needle.is_empty() || [&t.character, &t.login].iter().any(|s| s.as_deref().is_some_and(|s| s.to_lowercase().contains(&needle))))
            .collect();
        rows.sort_by_key(|t| std::cmp::Reverse(t.id));
        let total = rows.len() as i64;
        Ok(Page { items: rows.into_iter().skip(((q.page - 1) * q.per) as usize).take(q.per as usize).collect(), total })
    }
    async fn ticket_counts(&self) -> DbResult<TicketCounts> {
        self.check()?;
        let d = self.0.lock().unwrap();
        let mut c = TicketCounts::default();
        for t in &d.tickets {
            match (t.response.is_some(), t.closed.is_some()) {
                (false, false) => c.open += 1,
                (true, false) => c.answered += 1,
                (true, true) => c.read += 1,
                (false, true) => c.closed += 1,
            }
        }
        Ok(c)
    }
    async fn admin_ticket(&self, id: i64) -> DbResult<Option<Ticket>> {
        self.check()?;
        let d = self.0.lock().unwrap();
        Ok(d.tickets.iter().find(|t| t.id == id).map(|t| fake_ticket(&d, t)))
    }
    async fn answer_ticket(&self, id: i64, reply: &str) -> DbResult<Acted> {
        self.check()?;
        let mut d = self.0.lock().unwrap();
        let Some(t) = d.tickets.iter_mut().find(|t| t.id == id) else { return Ok(Acted::NotFound) };
        if t.closed.is_some() || t.response.is_some() {
            return Ok(Acted::NoChange);
        }
        t.response = Some(reply.into());
        t.responded = Some(NOW);
        Ok(Acted::Done)
    }
    async fn close_ticket(&self, id: i64) -> DbResult<Acted> {
        self.check()?;
        let mut d = self.0.lock().unwrap();
        let Some(t) = d.tickets.iter_mut().find(|t| t.id == id) else { return Ok(Acted::NotFound) };
        if t.closed.is_some() || t.response.is_some() {
            return Ok(Acted::NoChange);
        }
        t.closed = Some(NOW);
        Ok(Acted::Done)
    }
    async fn items(&self, q: &ItemQuery) -> DbResult<Page<ItemRaw>> {
        self.check()?;
        let mut d = self.0.lock().unwrap();
        d.item_queries.push(q.clone());
        // What ITEM_WHERE does: `q.q` is already folded; LIKE is a substring match here.
        let mut hits: Vec<ItemRaw> = d
            .items
            .iter()
            .map(|i| i.item.clone())
            .filter(|i| shown(i, q.level_cap))
            .filter(|i| q.q.is_empty() || i.name.contains(&q.q))
            .filter(|i| q.category == 0 || i.category == q.category)
            .filter(|i| q.job_bit == 0 || i.jobs & q.job_bit != 0)
            .filter(|i| q.slot_bit == 0 || i.slots & q.slot_bit != 0)
            .filter(|i| q.min_level == 0 || i.level.is_some_and(|l| l >= q.min_level))
            .filter(|i| q.max_level == 0 || i.level.is_some_and(|l| l <= q.max_level))
            .collect();
        hits.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        let total = hits.len() as i64;
        let items = hits.into_iter().skip(((q.page - 1) * q.per) as usize).take(q.per as usize).collect();
        Ok(Page { items, total })
    }
    async fn item(&self, id: i64, level_cap: i64) -> DbResult<Option<ItemDetailRaw>> {
        self.check()?;
        Ok(self.0.lock().unwrap().items.iter().find(|i| i.item.id == id && shown(&i.item, level_cap)).cloned())
    }
}

/// What ITEM_SHOWN lets through.
fn shown(i: &ItemRaw, level_cap: i64) -> bool {
    i.id > 0 && i.flags & 2 == 0 && i.kind != 8 && i.level.is_none_or(|l| l <= level_cap)
}

fn fake_ticket(d: &Data, t: &FakeTicket) -> Ticket {
    let c = d.chars.iter().find(|c| c.charid == t.charid);
    let accid = c.map(|c| c.accid).filter(|a| *a != 0);
    Ticket {
        id: t.id,
        charid: t.charid,
        character: c.map(|c| c.name.clone()),
        accid,
        login: accid.and_then(|a| d.accounts.iter().find(|x| x.id == a)).map(|a| a.login.clone()),
        gm: accid.map_or(0, |a| d.chars.iter().filter(|c| c.accid == a).map(|c| c.gm).max().unwrap_or(0)),
        message: t.message.clone(),
        response: t.response.clone(),
        created: Some(NOW - 600),
        responded: t.responded,
        closed: t.closed,
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
    site_with_store(cfg, upgraded_store().await).await
}

pub async fn site_with_store(cfg: Config, store: Store) -> Site {
    let fake = Fake::default();
    let clock = Arc::new(AtomicI64::new(NOW));
    let c = clock.clone();
    let state = app_state(Parts {
        cfg,
        game: Arc::new(fake.clone()),
        store,
        probe: Arc::new(StaticProbe(vec![ServiceState { id: "world", name: "World", state: "up" }])),
        clock: Arc::new(move || c.load(Ordering::SeqCst)),
        dummy_hash: auth::hash_password("dummy", 4).unwrap(),
        bcrypt_cost: 4,
    });
    Site { router: ffxi_web::router(state.clone()), state, fake, clock }
}

/// The accounts' `timecreate`, as the fake reports it everywhere.
pub fn fake_created(id: i64) -> Option<i64> {
    Some(NOW - 1000 * (1000 - id))
}

/// A store that looks upgraded from before website roles, so the first role lookup copies
/// roles from GM levels (as the existing tests expect). `Store::memory()` alone is a new store.
pub async fn upgraded_store() -> Store {
    let s = Store::memory().await.unwrap();
    s.forget_roles_seeded().await.unwrap();
    s
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
        let mut b = Request::builder().method(req.method.clone()).uri(&req.path).header(header::HOST, req.host.as_str());
        for f in &req.forwarded {
            b = b.header("x-forwarded-for", f);
        }
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
        r.extensions_mut().insert(ConnectInfo(req.from.map_or(SocketAddr::from(([10, 0, 0, req.peer], 5555)), |ip| SocketAddr::new(ip, 5555))));
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
    pub host: String,
    pub forwarded: Vec<String>,
    /// A peer address other than 10.0.0.<peer>.
    pub from: Option<std::net::IpAddr>,
}

impl Req {
    pub fn get(path: &str) -> Req {
        Req { method: Method::GET, path: path.into(), body: None, cookie: None, csrf: None, origin: None, content_type: true, peer: 2, host: "site.test".into(), forwarded: vec![], from: None }
    }
    pub fn post(path: &str, body: serde_json::Value) -> Req {
        Req { method: Method::POST, body: Some(body), origin: Some("http://site.test".into()), ..Req::get(path) }
    }
    pub fn put(path: &str, body: serde_json::Value) -> Req {
        Req { method: Method::PUT, ..Req::post(path, body) }
    }
    pub fn delete(path: &str) -> Req {
        Req { method: Method::DELETE, ..Req::post(path, serde_json::json!({})) }
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
    pub fn host(mut self, host: &str) -> Req {
        self.host = host.into();
        self
    }
    /// Adds an `X-Forwarded-For` header line.
    pub fn forwarded(mut self, line: &str) -> Req {
        self.forwarded.push(line.into());
        self
    }
    pub fn from(mut self, ip: &str) -> Req {
        self.from = Some(ip.parse().unwrap());
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
