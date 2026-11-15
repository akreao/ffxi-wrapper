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

/// What every request re-reads: the account's standing, login and creation time (a stored
/// website role only counts for the account it was given to, not a later one with its id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Standing {
    pub status: i64,
    pub login: String,
    pub created: Option<i64>,
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

// ---- the admin panel: accounts, characters, bans ----

/// One page of a list, and how many rows there are in all.
#[derive(Debug, Clone, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountFilter {
    All,
    Online,
    Banned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountSort {
    Login,
    Id,
    Created,
    LastLogin,
}

#[derive(Debug, Clone)]
pub struct AccountQuery {
    pub q: String,
    pub filter: AccountFilter,
    pub sort: AccountSort,
    pub desc: bool,
    pub page: i64,
    pub per: i64,
}

/// One row of the account list.
#[derive(Debug, Clone, Serialize)]
pub struct AccountRow {
    pub id: i64,
    pub login: String,
    pub status: i64,
    pub created: Option<i64>,
    pub last_login: Option<i64>,
    pub characters: Vec<String>,
    pub gm: i64,
    pub online: bool,
}

/// What `accounts_banned` records about a ban.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BanRecord {
    pub banned_at: Option<i64>,
    /// `None`: until someone lifts it.
    pub until: Option<i64>,
    pub reason: Option<String>,
}

/// A character as the admin panel lists it.
#[derive(Debug, Clone, Serialize)]
pub struct AdminChar {
    #[serde(flatten)]
    pub summary: CharSummary,
    pub accid: i64,
    pub login: String,
    pub gm: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminAccount {
    pub id: i64,
    pub login: String,
    pub status: i64,
    pub created: Option<i64>,
    pub last_login: Option<i64>,
    pub gm: i64,
    pub has_totp: bool,
    pub online: bool,
    pub ban: Option<BanRecord>,
    pub characters: Vec<AdminChar>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminCharDetail {
    #[serde(flatten)]
    pub detail: CharDetail,
    pub accid: i64,
    pub login: String,
    pub gm: i64,
    pub created: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharSort {
    Name,
    Account,
    Playtime,
    LastLogout,
}

#[derive(Debug, Clone)]
pub struct CharQuery {
    pub q: String,
    pub online: bool,
    pub sort: CharSort,
    pub desc: bool,
    pub page: i64,
    pub per: i64,
}

/// An account under a ban, for the Bans page.
#[derive(Debug, Clone, Serialize)]
pub struct BanRow {
    pub accid: i64,
    pub login: String,
    pub characters: Vec<String>,
    pub ban: Option<BanRecord>,
}

/// How a change to an account or character ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acted {
    Done,
    NotFound,
    /// A Moderator's ban or unban of an account with an in-game GM character: owners only.
    HasGm,
    /// The change doesn't apply (for instance unbanning someone who isn't banned).
    NoChange,
}

/// The job columns of `char_jobs` and `char_exp`, in job id order (1-22).
pub const JOBS: [&str; 22] = [
    "war", "mnk", "whm", "blm", "rdm", "thf", "pld", "drk", "bst", "brd", "rng", "sam", "nin",
    "drg", "smn", "blu", "cor", "pup", "dnc", "sch", "geo", "run",
];

/// The crafting skills a ranking may name: `char_skills.skillid` 48..=57
/// (`data/enums/skill_type.yaml` in LSB: fishing, woodworking, smithing,
/// goldsmithing, clothcraft, leathercraft, bonecraft, alchemy, cooking, synergy).
pub const CRAFTS: [(i64, &str); 10] = [
    (48, "Fishing"),
    (49, "Woodworking"),
    (50, "Smithing"),
    (51, "Goldsmithing"),
    (52, "Clothcraft"),
    (53, "Leathercraft"),
    (54, "Bonecraft"),
    (55, "Alchemy"),
    (56, "Cooking"),
    (57, "Synergy"),
];

/// A ranking a visitor may ask for. `Job` holds an index into [`JOBS`], never request text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Board {
    Level,
    Job(usize),
    Craft(i64),
    Playtime,
}

impl Board {
    /// The cache key and a stable name for the board.
    pub fn key(&self) -> String {
        match self {
            Board::Level => "level".into(),
            Board::Job(j) => format!("job:{}", JOBS[*j]),
            Board::Craft(s) => format!("craft:{s}"),
            Board::Playtime => "playtime".into(),
        }
    }
}

/// Every ranking lists at most this many characters.
pub const RANKING_TOP: usize = 100;

/// One character on the public who-is-online list. Anonymous characters (`/anon`)
/// carry only their name and nation: no job, level or zone.
#[derive(Debug, Clone, Serialize)]
pub struct PublicPlayer {
    pub name: String,
    pub nation: i64,
    pub zone: Option<String>,
    pub mjob: Option<i64>,
    pub mlvl: Option<i64>,
    pub sjob: Option<i64>,
    pub slvl: Option<i64>,
    pub anonymous: bool,
}

/// One line of a ranking. `job` is a job id (1-22) on the level and job boards;
/// `value` is the level (job board), highest job level (level board), skill
/// level (craft board) or whole hours played (playtime board).
#[derive(Debug, Clone, Serialize)]
pub struct RankRow {
    pub rank: i64,
    pub name: String,
    pub nation: i64,
    pub job: Option<i64>,
    pub value: i64,
}

/// The address LSB stores in `accounts_sessions.client_addr`: network byte
/// order read as a little-endian `u32`.
pub fn decode_ip(v: i64) -> std::net::Ipv4Addr {
    std::net::Ipv4Addr::from((v as u32).to_le_bytes())
}

/// `bronze_sword` -> `Bronze Sword`, `hi-potion` -> `Hi-Potion`.
pub fn title_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut start = true;
    for c in name.chars() {
        match c {
            '_' => {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
                start = true;
            }
            '-' => {
                out.push('-');
                start = true;
            }
            c if start => {
                out.extend(c.to_uppercase());
                start = false;
            }
            c => out.push(c),
        }
    }
    out.trim().to_string()
}

/// `Northern_San_dOria` -> `Northern San dOria`.
pub fn zone_name(name: &str) -> String {
    name.replace('_', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_and_zone_names_are_tidied() {
        assert_eq!(title_case("bronze_sword"), "Bronze Sword");
        assert_eq!(title_case("hi-potion"), "Hi-Potion");
        assert_eq!(title_case("flask_of_echo_drops"), "Flask Of Echo Drops");
        assert_eq!(title_case("_odd__name_"), "Odd Name");
        assert_eq!(zone_name("Northern_San_dOria"), "Northern San dOria");
    }

    #[test]
    fn session_addresses_decode_like_lsb_stores_them() {
        // Network byte order read as a little-endian u32: bytes c0 a8 00 01.
        assert_eq!(decode_ip(0x0100A8C0).to_string(), "192.168.0.1");
        assert_eq!(decode_ip(0x0000_007F).to_string(), "127.0.0.0");
        assert_eq!(decode_ip(0xFFFF_FFFF).to_string(), "255.255.255.255");
    }
}

/// What making a game account came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Created {
    /// The new account's id.
    Account(i64),
    /// Someone has the login already.
    Taken,
}
