//! The shapes the API returns. Integers are `i64` throughout: every query
//! casts its columns `AS SIGNED`, so sqlx never has to match MariaDB's
//! unsigned and small integer types.

use serde::Serialize;

/// What sign-in needs from `accounts`.
#[derive(Debug, Clone)]
pub struct AccountAuth {
    pub id: i64,
    pub login: String,
    pub status: i64,
    pub password: String,
}

/// What every request re-reads: the account's standing and its highest GM level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Standing {
    pub status: i64,
    pub gm: i64,
}

#[derive(Debug, Clone)]
pub struct TotpRow {
    pub validated: bool,
    pub secret: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MyAccount {
    pub login: String,
    pub created: Option<i64>,
    pub status: i64,
    pub has_totp: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CharSummary {
    pub charid: i64,
    pub name: String,
    pub zone_id: i64,
    pub zone: Option<String>,
    pub nation: i64,
    pub mjob: Option<i64>,
    pub mlvl: Option<i64>,
    pub sjob: Option<i64>,
    pub slvl: Option<i64>,
    pub playtime: i64,
    pub last_logout: Option<i64>,
    pub online: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobLevel {
    pub job: &'static str,
    pub level: i64,
    pub exp: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CharDetail {
    #[serde(flatten)]
    pub summary: CharSummary,
    pub gil: i64,
    /// Level cap (`char_jobs.genkai`).
    pub cap: i64,
    pub jobs: Vec<JobLevel>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InventoryItem {
    pub location: i64,
    pub slot: i64,
    pub item_id: i64,
    pub name: Option<String>,
    pub quantity: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Counts {
    pub accounts: i64,
    pub characters: i64,
    pub online: i64,
    pub banned: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct OnlineRow {
    pub accid: i64,
    pub login: String,
    pub charid: i64,
    pub name: String,
    pub zone: Option<String>,
    pub mjob: Option<i64>,
    pub mlvl: Option<i64>,
    pub sjob: Option<i64>,
    pub slvl: Option<i64>,
    pub gm: i64,
    pub connecting: bool,
    pub gm_hidden: bool,
    /// Raw `accounts_sessions.client_addr`; the route decodes it for Owners only.
    #[serde(skip)]
    pub client_addr: i64,
}

/// The job columns of `char_jobs` and `char_exp`, in job id order (1-22).
pub const JOBS: [&str; 22] = [
    "war", "mnk", "whm", "blm", "rdm", "thf", "pld", "drk", "bst", "brd", "rng", "sam", "nin",
    "drg", "smn", "blu", "cor", "pup", "dnc", "sch", "geo", "run",
];

/// The address LSB stores in `accounts_sessions.client_addr`: network byte
/// order read as a little-endian `u32`.
pub fn decode_ip(v: i64) -> std::net::Ipv4Addr {
    std::net::Ipv4Addr::from((v as u32).to_le_bytes())
}

/// `bronze_sword` -> `Bronze Sword`.
pub fn title_case(name: &str) -> String {
    name.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `Northern_San_dOria` -> `Northern San dOria`.
pub fn zone_name(name: &str) -> String {
    name.replace('_', " ")
}
