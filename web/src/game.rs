//! The game database (MariaDB, `xidb`): every query the site runs against it.
//!
//! The site never builds SQL from request text: values are bound parameters,
//! and the only interpolated names are the fixed job columns in [`JOBS`].
//! Integers are cast `AS SIGNED` and zero dates go through `NULLIF`, so sqlx
//! decodes everything as `i64` / `Option<i64>`.

use crate::model::*;
use async_trait::async_trait;
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions};
use sqlx::{MySqlPool, Row};
use std::time::Duration;

#[derive(Debug)]
pub enum DbError {
    /// LSB's tables are missing: db-init hasn't finished, or failed.
    NotReady,
    /// Another sign-up holds the lock and didn't let go in time.
    Busy,
    Failed(String),
}

impl From<sqlx::Error> for DbError {
    fn from(e: sqlx::Error) -> DbError {
        if let sqlx::Error::Database(d) = &e {
            // 1146: table doesn't exist. 1049: unknown database.
            if matches!(d.code().as_deref(), Some("42S02") | Some("42000")) && d.message().contains("exist") {
                return DbError::NotReady;
            }
        }
        DbError::Failed(e.to_string())
    }
}

pub type DbResult<T> = Result<T, DbError>;

/// What the site reads and writes in the game database. The MariaDB version is
/// [`MariaDb`]; tests use an in-memory one.
#[async_trait]
pub trait Game: Send + Sync {
    async fn ping(&self) -> DbResult<()>;
    async fn account_by_login(&self, login: &str) -> DbResult<Option<AccountAuth>>;
    async fn account_by_id(&self, id: i64) -> DbResult<Option<AccountAuth>>;
    async fn totp(&self, accid: i64) -> DbResult<Option<TotpRow>>;
    /// `accounts.status`, `login` and `timecreate`, re-read on every request.
    async fn standing(&self, accid: i64) -> DbResult<Option<Standing>>;
    /// Every account with a character at GM level 3 or more: (id, login, creation time,
    /// highest level). Only read once, for the one-time copy of the roles GM levels used to give.
    async fn staff_by_gm_level(&self) -> DbResult<Vec<(i64, String, Option<i64>, i64)>>;

    async fn my_account(&self, accid: i64) -> DbResult<Option<MyAccount>>;
    async fn my_characters(&self, accid: i64) -> DbResult<Vec<CharSummary>>;
    /// `None` when the character isn't on this account (or doesn't exist).
    async fn my_character(&self, accid: i64, charid: i64) -> DbResult<Option<CharDetail>>;
    /// `None` when the character isn't on this account (or doesn't exist).
    async fn my_inventory(&self, accid: i64, charid: i64) -> DbResult<Option<Vec<InventoryItem>>>;
    async fn set_password(&self, accid: i64, hash: &str) -> DbResult<()>;
    /// Makes an account exactly as LSB's login server does (`LOGIN_CREATE`): the login must be
    /// unused, the id is `MAX(id) + 1` but at least 1000, status NORMAL, privilege USER.
    async fn create_account(&self, login: &str, hash: &str) -> DbResult<Created>;

    async fn counts(&self) -> DbResult<Counts>;
    async fn online(&self) -> DbResult<Vec<OnlineRow>>;
    /// Characters a visitor may be told are in game: no GMs, no GM-hidden ones, no banned
    /// accounts (the boards leave them out, so listing them online would show who is banned),
    /// nobody still connecting. Anonymous ones come back without job, level or zone.
    /// The home page's count is this list's length, so the two always agree.
    async fn public_online(&self) -> DbResult<Vec<PublicPlayer>>;
    /// The top [`RANKING_TOP`] of one board: no GMs, no banned accounts, no
    /// characters without an account; the level and job boards leave out /anon.
    async fn public_ranking(&self, board: Board) -> DbResult<Vec<RankRow>>;

    // ---- the admin panel ----

    async fn admin_accounts(&self, q: &AccountQuery) -> DbResult<Page<AccountRow>>;
    /// One account with its characters and, when banned, its ban record.
    async fn admin_account(&self, id: i64) -> DbResult<Option<AdminAccount>>;
    async fn admin_characters(&self, q: &CharQuery) -> DbResult<Page<AdminChar>>;
    async fn admin_character(&self, charid: i64) -> DbResult<Option<AdminCharDetail>>;
    async fn admin_inventory(&self, charid: i64) -> DbResult<Option<Vec<InventoryItem>>>;
    /// Banned accounts, newest ban first.
    async fn admin_bans(&self, q: &str, page: i64, per: i64) -> DbResult<Page<BanRow>>;

    /// The writes below each run in one transaction that locks the account row. Who may
    /// make them (by website role) is checked by the caller, under `AppState::role_gate`.
    /// With `no_gm` (a Moderator acting), a ban or unban is refused (`HasGm`) when any of
    /// the account's characters has an in-game GM level, read in the same transaction.
    ///
    /// Sets `status = 2`, drops the account's trust tokens and writes `accounts_banned`.
    async fn ban_account(&self, id: i64, until: Option<i64>, reason: &str, no_gm: bool) -> DbResult<Acted>;
    /// Sets `status = 1` and removes the `accounts_banned` row; `NoChange` when not banned.
    async fn unban_account(&self, id: i64, no_gm: bool) -> DbResult<Acted>;
    async fn reset_password(&self, id: i64, hash: &str) -> DbResult<Acted>;
    /// Removes two-step sign-in and the trust tokens.
    async fn reset_totp(&self, id: i64) -> DbResult<Acted>;
    /// Sets one character's in-game GM level.
    async fn set_gm_level(&self, charid: i64, level: i64) -> DbResult<Acted>;
    /// Ends every ban whose end has passed (`now`, Unix seconds) and returns (account, login) of each.
    async fn lift_expired_bans(&self, now: i64) -> DbResult<Vec<(i64, String)>>;

    // ---- the help desk ----

    /// Tickets, newest first.
    async fn admin_tickets(&self, q: &TicketQuery) -> DbResult<Page<Ticket>>;
    async fn ticket_counts(&self) -> DbResult<TicketCounts>;
    async fn admin_ticket(&self, id: i64) -> DbResult<Option<Ticket>>;
    /// Sets the reply LSB sends the player at their next zone-in; `NoChange` unless the ticket
    /// is open (a reply is never replaced: the player may have seen it already).
    async fn answer_ticket(&self, id: i64, reply: &str) -> DbResult<Acted>;
    /// Closes a ticket without a reply; `NoChange` unless it is open.
    async fn close_ticket(&self, id: i64) -> DbResult<Acted>;

    // ---- the item database (static game data, any signed-in account) ----

    /// Items by name then id; GM-only items are never listed.
    async fn items(&self, q: &ItemQuery) -> DbResult<Page<ItemRaw>>;
    /// `None` for a missing or GM-only item.
    async fn item(&self, id: i64) -> DbResult<Option<ItemDetailRaw>>;
}

pub struct MariaDb {
    pool: MySqlPool,
    jobs_sql: String,
    exp_sql: String,
}

impl MariaDb {
    /// Connects lazily, so the site starts (and says so) while the database is still coming up.
    pub fn connect(cfg: &crate::config::Config) -> MariaDb {
        let opts = MySqlConnectOptions::new()
            .host(&cfg.db_host)
            .port(cfg.db_port)
            .username(&cfg.db_user)
            .password(&cfg.db_password)
            .database(&cfg.db_name)
            .charset("utf8mb4")
            // sqlx otherwise sets the session to +00:00, and LSB writes local time.
            .timezone(None);
        let pool = MySqlPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(5))
            .connect_lazy_with(opts);
        let cols = |table: &str, extra: &str| {
            let list: Vec<String> = JOBS.iter().map(|j| format!("CAST(`{j}` AS SIGNED) AS `{j}`")).collect();
            format!("SELECT {extra}{} FROM {table} WHERE charid = ?", list.join(", "))
        };
        MariaDb {
            pool,
            jobs_sql: cols("char_jobs", "CAST(genkai AS SIGNED) AS cap, "),
            exp_sql: cols("char_exp", ""),
        }
    }
}


impl MariaDb {
    /// Job levels, experience, gil and level cap for a character already looked up.
    async fn load_detail(&self, summary: CharSummary) -> DbResult<CharDetail> {
        let charid = summary.charid;
        let jobs = sqlx::query(&self.jobs_sql).bind(charid).fetch_optional(&self.pool).await?;
        let exp = sqlx::query(&self.exp_sql).bind(charid).fetch_optional(&self.pool).await?;
        let gil = sqlx::query(
            "SELECT CAST(quantity AS SIGNED) AS gil FROM char_inventory
             WHERE charid = ? AND location = 0 AND slot = 0 AND itemId = 65535",
        )
        .bind(charid)
        .fetch_optional(&self.pool)
        .await?;
        let mut levels = Vec::with_capacity(JOBS.len());
        for j in JOBS {
            let level = match &jobs { Some(r) => r.try_get::<i64, _>(j)?, None => 0 };
            let xp = match &exp { Some(r) => r.try_get::<i64, _>(j)?, None => 0 };
            levels.push(JobLevel { job: j, level, exp: xp });
        }
        Ok(CharDetail {
            summary,
            gil: match gil { Some(r) => r.try_get("gil")?, None => 0 },
            cap: match &jobs { Some(r) => r.try_get("cap")?, None => 0 },
            jobs: levels,
        })
    }
}

/// `%`, `_` and the escape character itself, escaped for `LIKE ... ESCAPE '|'`.
pub fn like_pattern(q: &str) -> String {
    let mut out = String::with_capacity(q.len() + 2);
    out.push('%');
    for c in q.chars() {
        if matches!(c, '|' | '%' | '_') {
            out.push('|');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// What the admin list queries add to a character: its GM level, account id and login.
const ADMIN_CHAR_SELECT: &str = "SELECT CAST(c.charid AS SIGNED) AS charid, c.charname,
       CAST(c.pos_zone AS SIGNED) AS pos_zone, z.name AS zone, CAST(c.nation AS SIGNED) AS nation,
       CAST(s.mjob AS SIGNED) AS mjob, CAST(s.mlvl AS SIGNED) AS mlvl,
       CAST(s.sjob AS SIGNED) AS sjob, CAST(s.slvl AS SIGNED) AS slvl,
       CAST(c.playtime AS SIGNED) AS playtime,
       CAST(UNIX_TIMESTAMP(NULLIF(c.last_logout, '0000-00-00 00:00:00')) AS SIGNED) AS last_logout,
       CAST(EXISTS(SELECT 1 FROM accounts_sessions x WHERE x.charid = c.charid) AS SIGNED) AS online,
       CAST(c.gmlevel AS SIGNED) AS gmlevel, CAST(c.accid AS SIGNED) AS accid, a.login AS login,
       CAST(UNIX_TIMESTAMP(NULLIF(c.timecreated, '0000-00-00 00:00:00')) AS SIGNED) AS char_created
FROM chars c
JOIN accounts a ON a.id = c.accid
LEFT JOIN char_stats s ON s.charid = c.charid
LEFT JOIN zone_settings z ON z.zoneid = c.pos_zone";

fn admin_char(r: &sqlx::mysql::MySqlRow) -> Result<AdminChar, sqlx::Error> {
    Ok(AdminChar { summary: char_summary(r)?, accid: r.try_get("accid")?, login: r.try_get("login")?, gm: r.try_get("gmlevel")? })
}

fn name_list(joined: String) -> Vec<String> {
    joined.split('\u{1f}').filter(|n| !n.is_empty()).map(String::from).collect()
}

fn ban_record(r: &sqlx::mysql::MySqlRow) -> Result<Option<BanRecord>, sqlx::Error> {
    let at: Option<i64> = r.try_get("banned_at")?;
    let reason: Option<String> = r.try_get("ban_reason")?;
    let present: Option<i64> = r.try_get("ban_present")?;
    if present.is_none() {
        return Ok(None);
    }
    let until: Option<i64> = r.try_get("ban_until")?;
    // A zero date (no end) comes out of NULLIF as NULL; a UNIX_TIMESTAMP of 0 can't be an end either.
    Ok(Some(BanRecord { banned_at: at.filter(|t| *t > 0), until: until.filter(|t| *t > 0), reason }))
}

/// Columns `ban_record` reads, joined from `accounts_banned b`.
const BAN_COLS: &str = "CAST(b.accid AS SIGNED) AS ban_present,
       CAST(UNIX_TIMESTAMP(NULLIF(b.timebann, '0000-00-00 00:00:00')) AS SIGNED) AS banned_at,
       CAST(UNIX_TIMESTAMP(NULLIF(b.timeunbann, '0000-00-00 00:00:00')) AS SIGNED) AS ban_until,
       b.banncomment AS ban_reason";

const ACCOUNT_COLS: &str = "CAST(a.id AS SIGNED) AS id, a.login, CAST(a.status AS SIGNED) AS status,
       CAST(UNIX_TIMESTAMP(NULLIF(a.timecreate, '0000-00-00 00:00:00')) AS SIGNED) AS created,
       CAST(UNIX_TIMESTAMP(NULLIF(a.timelastmodify, '0000-00-00 00:00:00')) AS SIGNED) AS last_login,
       CAST(COALESCE((SELECT MAX(c.gmlevel) FROM chars c WHERE c.accid = a.id), 0) AS SIGNED) AS gm,
       CAST(COALESCE((SELECT GROUP_CONCAT(c.charname ORDER BY c.charname SEPARATOR '\u{1f}')
                      FROM chars c WHERE c.accid = a.id), '') AS CHAR) AS names,
       CAST(EXISTS(SELECT 1 FROM accounts_sessions s WHERE s.accid = a.id) AS SIGNED) AS online";

/// The account list's WHERE: `?` x3 for the search text, `?` x3 for the filter.
const ACCOUNT_WHERE: &str = "WHERE (? = '' OR a.login LIKE ? ESCAPE '|'
         OR EXISTS(SELECT 1 FROM chars c WHERE c.accid = a.id AND c.charname LIKE ? ESCAPE '|'))
       AND (? = 0 OR (? = 1 AND EXISTS(SELECT 1 FROM accounts_sessions s WHERE s.accid = a.id))
            OR (? = 2 AND (a.status & 1) = 0))";

fn account_row(r: &sqlx::mysql::MySqlRow) -> Result<AccountRow, sqlx::Error> {
    Ok(AccountRow {
        id: r.try_get("id")?,
        login: r.try_get("login")?,
        status: r.try_get("status")?,
        created: r.try_get("created")?,
        last_login: r.try_get("last_login")?,
        characters: name_list(r.try_get("names")?),
        gm: r.try_get("gm")?,
        online: r.try_get::<i64, _>("online")? != 0,
    })
}

const CHAR_SELECT: &str = "SELECT CAST(c.charid AS SIGNED) AS charid, c.charname,
       CAST(c.pos_zone AS SIGNED) AS pos_zone, z.name AS zone, CAST(c.nation AS SIGNED) AS nation,
       CAST(s.mjob AS SIGNED) AS mjob, CAST(s.mlvl AS SIGNED) AS mlvl,
       CAST(s.sjob AS SIGNED) AS sjob, CAST(s.slvl AS SIGNED) AS slvl,
       CAST(c.playtime AS SIGNED) AS playtime,
       CAST(UNIX_TIMESTAMP(NULLIF(c.last_logout, '0000-00-00 00:00:00')) AS SIGNED) AS last_logout,
       CAST(EXISTS(SELECT 1 FROM accounts_sessions x WHERE x.charid = c.charid) AS SIGNED) AS online
FROM chars c
LEFT JOIN char_stats s ON s.charid = c.charid
LEFT JOIN zone_settings z ON z.zoneid = c.pos_zone";

fn char_summary(r: &sqlx::mysql::MySqlRow) -> Result<CharSummary, sqlx::Error> {
    let zone: Option<String> = r.try_get("zone")?;
    Ok(CharSummary {
        charid: r.try_get("charid")?,
        name: r.try_get("charname")?,
        zone_id: r.try_get("pos_zone")?,
        zone: zone.map(|z| zone_name(&z)),
        nation: r.try_get("nation")?,
        mjob: r.try_get("mjob")?,
        mlvl: r.try_get("mlvl")?,
        sjob: r.try_get("sjob")?,
        slvl: r.try_get("slvl")?,
        playtime: r.try_get("playtime")?,
        last_logout: r.try_get("last_logout")?,
        online: r.try_get::<i64, _>("online")? != 0,
    })
}

fn account_auth(r: &sqlx::mysql::MySqlRow) -> Result<AccountAuth, sqlx::Error> {
    Ok(AccountAuth {
        id: r.try_get("id")?,
        login: r.try_get("login")?,
        status: r.try_get("status")?,
        password: r.try_get("password")?,
    })
}

#[async_trait]
impl Game for MariaDb {
    async fn ping(&self) -> DbResult<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    async fn account_by_login(&self, login: &str) -> DbResult<Option<AccountAuth>> {
        // The column's collation (utf8mb4_general_ci) makes this case-insensitive, like LSB.
        let r = sqlx::query(
            "SELECT CAST(id AS SIGNED) AS id, login, CAST(status AS SIGNED) AS status, password
             FROM accounts WHERE login = ? ORDER BY id LIMIT 1",
        )
        .bind(login)
        .fetch_optional(&self.pool)
        .await?;
        Ok(r.as_ref().map(account_auth).transpose()?)
    }

    async fn account_by_id(&self, id: i64) -> DbResult<Option<AccountAuth>> {
        let r = sqlx::query(
            "SELECT CAST(id AS SIGNED) AS id, login, CAST(status AS SIGNED) AS status, password
             FROM accounts WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(r.as_ref().map(account_auth).transpose()?)
    }

    async fn totp(&self, accid: i64) -> DbResult<Option<TotpRow>> {
        let r = sqlx::query("SELECT CAST(validated AS SIGNED) AS validated, secret FROM accounts_totp WHERE accid = ?")
            .bind(accid)
            .fetch_optional(&self.pool)
            .await?;
        Ok(match r {
            Some(r) => Some(TotpRow { validated: r.try_get::<i64, _>("validated")? != 0, secret: r.try_get("secret")? }),
            None => None,
        })
    }

    async fn standing(&self, accid: i64) -> DbResult<Option<Standing>> {
        let r = sqlx::query(
            "SELECT CAST(status AS SIGNED) AS status, login,
                    CAST(UNIX_TIMESTAMP(NULLIF(timecreate, '0000-00-00 00:00:00')) AS SIGNED) AS created
             FROM accounts WHERE id = ?",
        )
            .bind(accid)
            .fetch_optional(&self.pool)
            .await?;
        Ok(match r {
            Some(r) => Some(Standing { status: r.try_get("status")?, login: r.try_get("login")?, created: r.try_get("created")? }),
            None => None,
        })
    }

    async fn staff_by_gm_level(&self) -> DbResult<Vec<(i64, String, Option<i64>, i64)>> {
        let rows = sqlx::query(
            "SELECT CAST(a.id AS SIGNED) AS id, a.login,
                    CAST(UNIX_TIMESTAMP(NULLIF(a.timecreate, '0000-00-00 00:00:00')) AS SIGNED) AS created,
                    CAST(MAX(c.gmlevel) AS SIGNED) AS gm
             FROM accounts a JOIN chars c ON c.accid = a.id
             GROUP BY a.id, a.login, a.timecreate HAVING MAX(c.gmlevel) >= 3",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(|r| Ok((r.try_get("id")?, r.try_get("login")?, r.try_get("created")?, r.try_get("gm")?))).collect()
    }

    async fn my_account(&self, accid: i64) -> DbResult<Option<MyAccount>> {
        let r = sqlx::query(
            "SELECT a.login,
                    CAST(UNIX_TIMESTAMP(NULLIF(a.timecreate, '0000-00-00 00:00:00')) AS SIGNED) AS created,
                    CAST(a.status AS SIGNED) AS status,
                    CAST(EXISTS(SELECT 1 FROM accounts_totp t WHERE t.accid = a.id AND t.validated = 1) AS SIGNED) AS has_totp
             FROM accounts a WHERE a.id = ?",
        )
        .bind(accid)
        .fetch_optional(&self.pool)
        .await?;
        Ok(match r {
            Some(r) => Some(MyAccount {
                login: r.try_get("login")?,
                created: r.try_get("created")?,
                status: r.try_get("status")?,
                has_totp: r.try_get::<i64, _>("has_totp")? != 0,
            }),
            None => None,
        })
    }

    async fn my_characters(&self, accid: i64) -> DbResult<Vec<CharSummary>> {
        let rows = sqlx::query(&format!("{CHAR_SELECT} WHERE c.accid = ? ORDER BY c.charname"))
            .bind(accid)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(char_summary).collect::<Result<_, _>>()?)
    }

    async fn my_character(&self, accid: i64, charid: i64) -> DbResult<Option<CharDetail>> {
        let row = sqlx::query(&format!("{CHAR_SELECT} WHERE c.accid = ? AND c.charid = ?"))
            .bind(accid)
            .bind(charid)
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else { return Ok(None) };
        let summary = char_summary(&row)?;
        // Only after the ownership check above.
        Ok(Some(self.load_detail(summary).await?))
    }

    async fn my_inventory(&self, accid: i64, charid: i64) -> DbResult<Option<Vec<InventoryItem>>> {
        let owned = sqlx::query("SELECT 1 FROM chars WHERE charid = ? AND accid = ?")
            .bind(charid)
            .bind(accid)
            .fetch_optional(&self.pool)
            .await?;
        if owned.is_none() {
            return Ok(None);
        }
        let rows = sqlx::query(
            "SELECT CAST(i.location AS SIGNED) AS location, CAST(i.slot AS SIGNED) AS slot,
                    CAST(i.itemId AS SIGNED) AS item_id, CAST(b.name AS CHAR) AS name,
                    CAST(i.quantity AS SIGNED) AS quantity
             FROM char_inventory i
             JOIN chars c ON c.charid = i.charid AND c.accid = ?
             LEFT JOIN item_basic b ON b.itemid = i.itemId
             WHERE i.charid = ? AND NOT (i.location = 0 AND i.slot = 0)
             ORDER BY i.location, i.slot",
        )
        .bind(accid)
        .bind(charid)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in &rows {
            let name: Option<String> = r.try_get("name")?;
            out.push(InventoryItem {
                location: r.try_get("location")?,
                slot: r.try_get("slot")?,
                item_id: r.try_get("item_id")?,
                name: name.map(|n| title_case(&n)),
                quantity: r.try_get("quantity")?,
            });
        }
        Ok(Some(out))
    }

    async fn set_password(&self, accid: i64, hash: &str) -> DbResult<()> {
        sqlx::query("UPDATE accounts SET password = ? WHERE id = ?")
            .bind(hash)
            .bind(accid)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn create_account(&self, login: &str, hash: &str) -> DbResult<Created> {
        let mut conn = self.pool.acquire().await?;
        // One sign-up at a time, so two of them can't be handed the same id. (LSB's own creation
        // takes no such lock; the retry below covers a race with it.)
        let got: Option<i64> = sqlx::query("SELECT CAST(GET_LOCK('ffxi_web_signup', 5) AS SIGNED) AS got").fetch_one(&mut *conn).await?.try_get("got")?;
        if got != Some(1) {
            return Err(DbError::Busy);
        }
        let out = insert_account(&mut conn, login, hash).await;
        // A failed release can only mean the connection is gone, and the lock goes with it.
        let _ = sqlx::query("SELECT RELEASE_LOCK('ffxi_web_signup')").execute(&mut *conn).await;
        out
    }

    async fn counts(&self) -> DbResult<Counts> {
        let r = sqlx::query(
            "SELECT CAST((SELECT COUNT(*) FROM accounts) AS SIGNED) AS accounts,
                    CAST((SELECT COUNT(*) FROM chars WHERE accid <> 0) AS SIGNED) AS characters,
                    CAST((SELECT COUNT(*) FROM accounts_sessions) AS SIGNED) AS online,
                    CAST((SELECT COUNT(*) FROM accounts WHERE (status & 1) = 0) AS SIGNED) AS banned",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(Counts {
            accounts: r.try_get("accounts")?,
            characters: r.try_get("characters")?,
            online: r.try_get("online")?,
            banned: r.try_get("banned")?,
        })
    }

    async fn online(&self) -> DbResult<Vec<OnlineRow>> {
        let rows = sqlx::query(
            "SELECT CAST(s.accid AS SIGNED) AS accid, a.login, CAST(c.charid AS SIGNED) AS charid, c.charname,
                    z.name AS zone, CAST(st.mjob AS SIGNED) AS mjob, CAST(st.mlvl AS SIGNED) AS mlvl,
                    CAST(st.sjob AS SIGNED) AS sjob, CAST(st.slvl AS SIGNED) AS slvl,
                    CAST(c.gmlevel AS SIGNED) AS gmlevel,
                    CAST(s.client_port = 0 AS SIGNED) AS connecting,
                    CAST(s.client_addr AS SIGNED) AS client_addr,
                    CAST(COALESCE(f.gmHiddenEnabled, 0) AS SIGNED) AS gm_hidden
             FROM accounts_sessions s
             JOIN chars c ON c.charid = s.charid
             JOIN accounts a ON a.id = s.accid
             LEFT JOIN char_stats st ON st.charid = c.charid
             LEFT JOIN zone_settings z ON z.zoneid = c.pos_zone
             LEFT JOIN char_flags f ON f.charid = c.charid
             ORDER BY c.charname",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in &rows {
            let zone: Option<String> = r.try_get("zone")?;
            out.push(OnlineRow {
                accid: r.try_get("accid")?,
                login: r.try_get("login")?,
                charid: r.try_get("charid")?,
                name: r.try_get("charname")?,
                zone: zone.map(|z| zone_name(&z)),
                mjob: r.try_get("mjob")?,
                mlvl: r.try_get("mlvl")?,
                sjob: r.try_get("sjob")?,
                slvl: r.try_get("slvl")?,
                gm: r.try_get("gmlevel")?,
                connecting: r.try_get::<i64, _>("connecting")? != 0,
                gm_hidden: r.try_get::<i64, _>("gm_hidden")? != 0,
                client_addr: r.try_get("client_addr")?,
            });
        }
        Ok(out)
    }

    async fn public_online(&self) -> DbResult<Vec<PublicPlayer>> {
        // /anon is bit 2 of chars.settings (SAVE_CONF.AnonymityFlg in LSB's mmo.h).
        let rows = sqlx::query(
            "SELECT c.charname, CAST(c.nation AS SIGNED) AS nation, z.name AS zone,
                    CAST((c.settings & 4) <> 0 AS SIGNED) AS anon,
                    CAST(st.mjob AS SIGNED) AS mjob, CAST(st.mlvl AS SIGNED) AS mlvl,
                    CAST(st.sjob AS SIGNED) AS sjob, CAST(st.slvl AS SIGNED) AS slvl
             FROM accounts_sessions x
             JOIN chars c ON c.charid = x.charid
             JOIN accounts a ON a.id = c.accid AND (a.status & 1) = 1
             LEFT JOIN char_stats st ON st.charid = c.charid
             LEFT JOIN zone_settings z ON z.zoneid = c.pos_zone
             LEFT JOIN char_flags f ON f.charid = c.charid
             WHERE x.client_port <> 0 AND c.accid <> 0
               AND COALESCE(f.gmHiddenEnabled, 0) = 0 AND c.gmlevel = 0
             ORDER BY c.charname",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in &rows {
            let anon = r.try_get::<i64, _>("anon")? != 0;
            let zone: Option<String> = r.try_get("zone")?;
            out.push(PublicPlayer {
                name: r.try_get("charname")?,
                nation: r.try_get("nation")?,
                zone: if anon { None } else { zone.map(|z| zone_name(&z)) },
                mjob: if anon { None } else { r.try_get("mjob")? },
                mlvl: if anon { None } else { r.try_get("mlvl")? },
                sjob: if anon { None } else { r.try_get("sjob")? },
                slvl: if anon { None } else { r.try_get("slvl")? },
                anonymous: anon,
            });
        }
        Ok(out)
    }

    async fn public_ranking(&self, board: Board) -> DbResult<Vec<RankRow>> {
        // Every board: characters with an account, not GMs, not GM-hidden, on an account
        // that is not banned. The only interpolated names are job columns from JOBS.
        const WHO: &str = "FROM chars c
             JOIN accounts a ON a.id = c.accid AND (a.status & 1) = 1
             LEFT JOIN char_flags f ON f.charid = c.charid";
        const OK: &str = "c.accid <> 0 AND c.gmlevel = 0 AND COALESCE(f.gmHiddenEnabled, 0) = 0";
        let top = RANKING_TOP as i64;
        let mut out = Vec::new();
        match board {
            Board::Level => {
                let cols: Vec<String> = JOBS.iter().map(|j| format!("CAST(j.`{j}` AS SIGNED) AS `{j}`")).collect();
                let greatest: Vec<String> = JOBS.iter().map(|j| format!("j.`{j}`")).collect();
                let sql = format!(
                    "SELECT c.charname, CAST(c.nation AS SIGNED) AS nation, {} {WHO}
                     JOIN char_jobs j ON j.charid = c.charid
                     WHERE {OK} AND (c.settings & 4) = 0
                     ORDER BY GREATEST({}) DESC, c.charname LIMIT {top}",
                    cols.join(", "),
                    greatest.join(", ")
                );
                for (i, r) in sqlx::query(&sql).fetch_all(&self.pool).await?.iter().enumerate() {
                    // The job this character is best at: the first of the highest levels.
                    let mut best = (0usize, -1i64);
                    for (n, j) in JOBS.iter().enumerate() {
                        let lvl: i64 = r.try_get(*j)?;
                        if lvl > best.1 {
                            best = (n, lvl);
                        }
                    }
                    out.push(RankRow { rank: i as i64 + 1, name: r.try_get("charname")?, nation: r.try_get("nation")?, job: Some(best.0 as i64 + 1), value: best.1 });
                }
            }
            Board::Job(n) => {
                let col = JOBS[n];
                let sql = format!(
                    "SELECT c.charname, CAST(c.nation AS SIGNED) AS nation, CAST(j.`{col}` AS SIGNED) AS lvl {WHO}
                     JOIN char_jobs j ON j.charid = c.charid
                     JOIN char_exp e ON e.charid = c.charid
                     WHERE {OK} AND (c.settings & 4) = 0 AND j.`{col}` > 1
                     ORDER BY j.`{col}` DESC, e.`{col}` DESC, c.charname LIMIT {top}"
                );
                for (i, r) in sqlx::query(&sql).fetch_all(&self.pool).await?.iter().enumerate() {
                    out.push(RankRow { rank: i as i64 + 1, name: r.try_get("charname")?, nation: r.try_get("nation")?, job: Some(n as i64 + 1), value: r.try_get("lvl")? });
                }
            }
            Board::Craft(skill) => {
                let sql = format!(
                    "SELECT c.charname, CAST(c.nation AS SIGNED) AS nation, CAST(k.value DIV 10 AS SIGNED) AS lvl {WHO}
                     JOIN char_skills k ON k.charid = c.charid AND k.skillid = ?
                     WHERE {OK} AND k.value > 0
                     ORDER BY k.value DESC, c.charname LIMIT {top}"
                );
                for (i, r) in sqlx::query(&sql).bind(skill).fetch_all(&self.pool).await?.iter().enumerate() {
                    out.push(RankRow { rank: i as i64 + 1, name: r.try_get("charname")?, nation: r.try_get("nation")?, job: None, value: r.try_get("lvl")? });
                }
            }
            Board::Playtime => {
                let sql = format!(
                    "SELECT c.charname, CAST(c.nation AS SIGNED) AS nation, CAST(c.playtime DIV 3600 AS SIGNED) AS hrs {WHO}
                     WHERE {OK} AND c.playtime >= 3600
                     ORDER BY c.playtime DESC, c.charname LIMIT {top}"
                );
                for (i, r) in sqlx::query(&sql).fetch_all(&self.pool).await?.iter().enumerate() {
                    out.push(RankRow { rank: i as i64 + 1, name: r.try_get("charname")?, nation: r.try_get("nation")?, job: None, value: r.try_get("hrs")? });
                }
            }
        }
        Ok(out)
    }

    // ---- the admin panel ----

    async fn admin_accounts(&self, q: &AccountQuery) -> DbResult<Page<AccountRow>> {
        // The ORDER BY comes from this fixed list, never from the request.
        let col = match q.sort {
            AccountSort::Login => "a.login",
            AccountSort::Id => "a.id",
            AccountSort::Created => "a.timecreate",
            AccountSort::LastLogin => "a.timelastmodify",
        };
        let dir = if q.desc { "DESC" } else { "ASC" };
        let like = like_pattern(&q.q);
        let f = match q.filter { AccountFilter::All => 0i64, AccountFilter::Online => 1, AccountFilter::Banned => 2 };
        let total: i64 = sqlx::query(&format!("SELECT CAST(COUNT(*) AS SIGNED) AS n FROM accounts a {ACCOUNT_WHERE}"))
            .bind(&q.q)
            .bind(&like)
            .bind(&like)
            .bind(f)
            .bind(f)
            .bind(f)
            .fetch_one(&self.pool)
            .await?
            .try_get("n")?;
        let rows = sqlx::query(&format!(
            "SELECT {ACCOUNT_COLS} FROM accounts a {ACCOUNT_WHERE} ORDER BY {col} {dir}, a.id {dir} LIMIT ? OFFSET ?"
        ))
        .bind(&q.q)
        .bind(&like)
        .bind(&like)
        .bind(f)
        .bind(f)
        .bind(f)
        .bind(q.per)
        .bind((q.page - 1) * q.per)
        .fetch_all(&self.pool)
        .await?;
        Ok(Page { items: rows.iter().map(account_row).collect::<Result<_, _>>()?, total })
    }

    async fn admin_account(&self, id: i64) -> DbResult<Option<AdminAccount>> {
        let r = sqlx::query(&format!(
            "SELECT {ACCOUNT_COLS}, {BAN_COLS},
                    CAST(EXISTS(SELECT 1 FROM accounts_totp t WHERE t.accid = a.id AND t.validated = 1) AS SIGNED) AS has_totp
             FROM accounts a LEFT JOIN accounts_banned b ON b.accid = a.id WHERE a.id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(r) = r else { return Ok(None) };
        let row = account_row(&r)?;
        let chars = sqlx::query(&format!("{ADMIN_CHAR_SELECT} WHERE c.accid = ? ORDER BY c.charname"))
            .bind(id)
            .fetch_all(&self.pool)
            .await?;
        // Only a banned account's record means anything.
        let ban = if row.status & 1 == 0 { ban_record(&r)? } else { None };
        Ok(Some(AdminAccount {
            id: row.id,
            login: row.login,
            status: row.status,
            created: row.created,
            last_login: row.last_login,
            gm: row.gm,
            has_totp: r.try_get::<i64, _>("has_totp")? != 0,
            online: row.online,
            ban,
            characters: chars.iter().map(admin_char).collect::<Result<_, _>>()?,
        }))
    }

    async fn admin_characters(&self, q: &CharQuery) -> DbResult<Page<AdminChar>> {
        let col = match q.sort {
            CharSort::Name => "c.charname",
            CharSort::Account => "a.login",
            CharSort::Playtime => "c.playtime",
            CharSort::LastLogout => "c.last_logout",
        };
        let dir = if q.desc { "DESC" } else { "ASC" };
        let like = like_pattern(&q.q);
        let wh = "WHERE c.accid <> 0 AND (? = '' OR c.charname LIKE ? ESCAPE '|')
                  AND (? = 0 OR EXISTS(SELECT 1 FROM accounts_sessions x WHERE x.charid = c.charid))";
        let online = q.online as i64;
        let total: i64 = sqlx::query(&format!("SELECT CAST(COUNT(*) AS SIGNED) AS n FROM chars c JOIN accounts a ON a.id = c.accid {wh}"))
            .bind(&q.q)
            .bind(&like)
            .bind(online)
            .fetch_one(&self.pool)
            .await?
            .try_get("n")?;
        let rows = sqlx::query(&format!("{ADMIN_CHAR_SELECT} {wh} ORDER BY {col} {dir}, c.charid {dir} LIMIT ? OFFSET ?"))
            .bind(&q.q)
            .bind(&like)
            .bind(online)
            .bind(q.per)
            .bind((q.page - 1) * q.per)
            .fetch_all(&self.pool)
            .await?;
        Ok(Page { items: rows.iter().map(admin_char).collect::<Result<_, _>>()?, total })
    }

    async fn admin_character(&self, charid: i64) -> DbResult<Option<AdminCharDetail>> {
        let row = sqlx::query(&format!("{ADMIN_CHAR_SELECT} WHERE c.charid = ? AND c.accid <> 0"))
            .bind(charid)
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else { return Ok(None) };
        let c = admin_char(&row)?;
        let created: Option<i64> = row.try_get("char_created")?;
        let detail = self.load_detail(c.summary).await?;
        Ok(Some(AdminCharDetail { detail, accid: c.accid, login: c.login, gm: c.gm, created }))
    }

    async fn admin_inventory(&self, charid: i64) -> DbResult<Option<Vec<InventoryItem>>> {
        let exists = sqlx::query("SELECT 1 FROM chars WHERE charid = ? AND accid <> 0").bind(charid).fetch_optional(&self.pool).await?;
        if exists.is_none() {
            return Ok(None);
        }
        let rows = sqlx::query(
            "SELECT CAST(i.location AS SIGNED) AS location, CAST(i.slot AS SIGNED) AS slot,
                    CAST(i.itemId AS SIGNED) AS item_id, CAST(b.name AS CHAR) AS name,
                    CAST(i.quantity AS SIGNED) AS quantity
             FROM char_inventory i
             LEFT JOIN item_basic b ON b.itemid = i.itemId
             WHERE i.charid = ? AND NOT (i.location = 0 AND i.slot = 0)
             ORDER BY i.location, i.slot",
        )
        .bind(charid)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in &rows {
            let name: Option<String> = r.try_get("name")?;
            out.push(InventoryItem {
                location: r.try_get("location")?,
                slot: r.try_get("slot")?,
                item_id: r.try_get("item_id")?,
                name: name.map(|n| title_case(&n)),
                quantity: r.try_get("quantity")?,
            });
        }
        Ok(Some(out))
    }

    async fn admin_bans(&self, q: &str, page: i64, per: i64) -> DbResult<Page<BanRow>> {
        let like = like_pattern(q);
        let wh = "WHERE (a.status & 1) = 0 AND (? = '' OR a.login LIKE ? ESCAPE '|'
                  OR EXISTS(SELECT 1 FROM chars c WHERE c.accid = a.id AND c.charname LIKE ? ESCAPE '|'))";
        let total: i64 = sqlx::query(&format!("SELECT CAST(COUNT(*) AS SIGNED) AS n FROM accounts a {wh}"))
            .bind(q)
            .bind(&like)
            .bind(&like)
            .fetch_one(&self.pool)
            .await?
            .try_get("n")?;
        let rows = sqlx::query(&format!(
            "SELECT {ACCOUNT_COLS}, {BAN_COLS} FROM accounts a LEFT JOIN accounts_banned b ON b.accid = a.id {wh}
             ORDER BY b.timebann DESC, a.id DESC LIMIT ? OFFSET ?"
        ))
        .bind(q)
        .bind(&like)
        .bind(&like)
        .bind(per)
        .bind((page - 1) * per)
        .fetch_all(&self.pool)
        .await?;
        let mut items = Vec::with_capacity(rows.len());
        for r in &rows {
            let a = account_row(r)?;
            items.push(BanRow { accid: a.id, login: a.login, characters: a.characters, ban: ban_record(r)? });
        }
        Ok(Page { items, total })
    }

    async fn ban_account(&self, id: i64, until: Option<i64>, reason: &str, no_gm: bool) -> DbResult<Acted> {
        let mut tx = self.pool.begin().await?;
        match lock_account(&mut tx, id).await? {
            Locked::Ok { gm, .. } if no_gm && gm > 0 => return Ok(Acted::HasGm),
            Locked::Ok { .. } => {}
            Locked::Missing => return Ok(Acted::NotFound),
        }
        sqlx::query("UPDATE accounts SET status = 2 WHERE id = ?").bind(id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM accounts_trust_tokens WHERE accid = ?").bind(id).execute(&mut *tx).await?;
        // A zero date means "until someone lifts it", as everywhere in LSB.
        sqlx::query(
            "INSERT INTO accounts_banned (accid, timebann, timeunbann, banncomment)
             VALUES (?, NOW(), COALESCE(FROM_UNIXTIME(?), '0000-00-00 00:00:00'), ?)
             ON DUPLICATE KEY UPDATE timebann = VALUES(timebann), timeunbann = VALUES(timeunbann), banncomment = VALUES(banncomment)",
        )
        .bind(id)
        .bind(until)
        .bind(reason)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Acted::Done)
    }

    async fn unban_account(&self, id: i64, no_gm: bool) -> DbResult<Acted> {
        let mut tx = self.pool.begin().await?;
        match lock_account(&mut tx, id).await? {
            Locked::Ok { gm, .. } if no_gm && gm > 0 => return Ok(Acted::HasGm),
            Locked::Ok { status, .. } if status & 1 == 0 => {}
            Locked::Ok { .. } => return Ok(Acted::NoChange),
            Locked::Missing => return Ok(Acted::NotFound),
        }
        sqlx::query("UPDATE accounts SET status = 1 WHERE id = ?").bind(id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM accounts_banned WHERE accid = ?").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(Acted::Done)
    }

    async fn reset_password(&self, id: i64, hash: &str) -> DbResult<Acted> {
        let mut tx = self.pool.begin().await?;
        match lock_account(&mut tx, id).await? {
            Locked::Ok { .. } => {}
            Locked::Missing => return Ok(Acted::NotFound),
        }
        sqlx::query("UPDATE accounts SET password = ? WHERE id = ?").bind(hash).bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(Acted::Done)
    }

    async fn reset_totp(&self, id: i64) -> DbResult<Acted> {
        let mut tx = self.pool.begin().await?;
        match lock_account(&mut tx, id).await? {
            Locked::Ok { .. } => {}
            Locked::Missing => return Ok(Acted::NotFound),
        }
        sqlx::query("DELETE FROM accounts_totp WHERE accid = ?").bind(id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM accounts_trust_tokens WHERE accid = ?").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(Acted::Done)
    }

    async fn set_gm_level(&self, charid: i64, level: i64) -> DbResult<Acted> {
        let mut tx = self.pool.begin().await?;
        let r = sqlx::query("SELECT CAST(gmlevel AS SIGNED) AS gm FROM chars WHERE charid = ? AND accid <> 0 FOR UPDATE")
            .bind(charid)
            .fetch_optional(&mut *tx)
            .await?;
        if r.is_none() {
            return Ok(Acted::NotFound);
        }
        sqlx::query("UPDATE chars SET gmlevel = ? WHERE charid = ? AND accid <> 0").bind(level).bind(charid).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(Acted::Done)
    }

    async fn lift_expired_bans(&self, now: i64) -> DbResult<Vec<(i64, String)>> {
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT CAST(a.id AS SIGNED) AS id, a.login
             FROM accounts a JOIN accounts_banned b ON b.accid = a.id
             WHERE (a.status & 1) = 0 AND UNIX_TIMESTAMP(NULLIF(b.timeunbann, '0000-00-00 00:00:00')) <= ?
             FOR UPDATE",
        )
        .bind(now)
        .fetch_all(&mut *tx)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in &rows {
            let id: i64 = r.try_get("id")?;
            sqlx::query("UPDATE accounts SET status = 1 WHERE id = ?").bind(id).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM accounts_banned WHERE accid = ?").bind(id).execute(&mut *tx).await?;
            out.push((id, r.try_get("login")?));
        }
        tx.commit().await?;
        Ok(out)
    }

    async fn admin_tickets(&self, q: &TicketQuery) -> DbResult<Page<Ticket>> {
        let like = like_pattern(&q.q);
        let state = match q.filter {
            TicketFilter::Open => "h.deleted_at IS NULL AND h.response IS NULL",
            TicketFilter::Answered => "h.deleted_at IS NULL AND h.response IS NOT NULL",
            TicketFilter::Read => "h.deleted_at IS NOT NULL AND h.response IS NOT NULL",
            TicketFilter::Closed => "h.deleted_at IS NOT NULL AND h.response IS NULL",
            TicketFilter::All => "TRUE",
        };
        let wh = format!(
            "WHERE {state} AND (? = '' OR c.charname LIKE ? ESCAPE '|' OR a.login LIKE ? ESCAPE '|')"
        );
        let total: i64 = sqlx::query(&format!("SELECT CAST(COUNT(*) AS SIGNED) AS n {TICKET_FROM} {wh}"))
            .bind(&q.q)
            .bind(&like)
            .bind(&like)
            .fetch_one(&self.pool)
            .await?
            .try_get("n")?;
        let rows = sqlx::query(&format!("SELECT {TICKET_COLS} {TICKET_FROM} {wh} ORDER BY h.id DESC LIMIT ? OFFSET ?"))
            .bind(&q.q)
            .bind(&like)
            .bind(&like)
            .bind(q.per)
            .bind((q.page - 1) * q.per)
            .fetch_all(&self.pool)
            .await?;
        let items = rows.iter().map(ticket_row).collect::<Result<_, _>>()?;
        Ok(Page { items, total })
    }

    async fn ticket_counts(&self) -> DbResult<TicketCounts> {
        let r = sqlx::query(
            "SELECT CAST(COALESCE(SUM(deleted_at IS NULL AND response IS NULL), 0) AS SIGNED) AS open,
                    CAST(COALESCE(SUM(deleted_at IS NULL AND response IS NOT NULL), 0) AS SIGNED) AS answered,
                    CAST(COALESCE(SUM(deleted_at IS NOT NULL AND response IS NOT NULL), 0) AS SIGNED) AS rd,
                    CAST(COALESCE(SUM(deleted_at IS NOT NULL AND response IS NULL), 0) AS SIGNED) AS closed
             FROM help_desk",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(TicketCounts { open: r.try_get("open")?, answered: r.try_get("answered")?, read: r.try_get("rd")?, closed: r.try_get("closed")? })
    }

    async fn admin_ticket(&self, id: i64) -> DbResult<Option<Ticket>> {
        let r = sqlx::query(&format!("SELECT {TICKET_COLS} {TICKET_FROM} WHERE h.id = ?")).bind(id).fetch_optional(&self.pool).await?;
        Ok(r.as_ref().map(ticket_row).transpose()?)
    }

    async fn answer_ticket(&self, id: i64, reply: &str) -> DbResult<Acted> {
        // One statement: two staff answering at once can't both win.
        let done = sqlx::query("UPDATE help_desk SET response = ?, responded_at = NOW() WHERE id = ? AND response IS NULL AND deleted_at IS NULL")
            .bind(reply.as_bytes())
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        self.ticket_outcome(id, done).await
    }

    async fn close_ticket(&self, id: i64) -> DbResult<Acted> {
        let done = sqlx::query("UPDATE help_desk SET deleted_at = NOW() WHERE id = ? AND response IS NULL AND deleted_at IS NULL")
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        self.ticket_outcome(id, done).await
    }

    async fn items(&self, q: &ItemQuery) -> DbResult<Page<ItemRaw>> {
        let like = like_pattern(&q.q);
        let total: i64 = sqlx::query(&format!("SELECT CAST(COUNT(*) AS SIGNED) AS n FROM item_basic b LEFT JOIN item_equipment e ON e.itemId = b.itemid {ITEM_WHERE}"))
            .bind(&q.q).bind(&like)
            .bind(q.category).bind(q.category)
            .bind(q.job_bit).bind(q.job_bit)
            .bind(q.slot_bit).bind(q.slot_bit)
            .bind(q.min_level).bind(q.min_level)
            .bind(q.max_level).bind(q.max_level)
            .fetch_one(&self.pool)
            .await?
            .try_get("n")?;
        let rows = sqlx::query(&format!("{ITEM_SELECT} {ITEM_WHERE} ORDER BY b.name, b.itemid LIMIT ? OFFSET ?"))
            .bind(&q.q).bind(&like)
            .bind(q.category).bind(q.category)
            .bind(q.job_bit).bind(q.job_bit)
            .bind(q.slot_bit).bind(q.slot_bit)
            .bind(q.min_level).bind(q.min_level)
            .bind(q.max_level).bind(q.max_level)
            .bind(q.per)
            .bind((q.page - 1) * q.per)
            .fetch_all(&self.pool)
            .await?;
        let items = rows.iter().map(item_row).collect::<Result<_, _>>()?;
        Ok(Page { items, total })
    }

    async fn item(&self, id: i64) -> DbResult<Option<ItemDetailRaw>> {
        let Some(r) = sqlx::query(&format!("{ITEM_SELECT} WHERE b.itemid = ? AND (b.flags & 2) = 0"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
        else {
            return Ok(None);
        };
        let item = item_row(&r)?;
        let weapon = sqlx::query(
            "SELECT CAST(skill AS SIGNED) AS skill, CAST(dmg AS SIGNED) AS dmg, CAST(delay AS SIGNED) AS delay
             FROM item_weapon WHERE itemId = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .map(|w| Ok::<_, sqlx::Error>((w.try_get("skill")?, w.try_get("dmg")?, w.try_get("delay")?)))
        .transpose()?;
        let mods = sqlx::query("SELECT CAST(modId AS SIGNED) AS m, CAST(value AS SIGNED) AS v FROM item_mods WHERE itemId = ? ORDER BY modId")
            .bind(id)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|m| Ok((m.try_get("m")?, m.try_get("v")?)))
            .collect::<Result<_, sqlx::Error>>()?;
        Ok(Some(ItemDetailRaw { item, weapon, mods }))
    }
}

/// An item: `item_basic b`, and `item_equipment e` when it is equipment.
const ITEM_SELECT: &str = "SELECT CAST(b.itemid AS SIGNED) AS id, b.name AS name, CAST(b.type AS SIGNED) AS kind,
       CAST(b.aH AS SIGNED) AS category, CAST(b.stackSize AS SIGNED) AS stack, CAST(b.flags AS SIGNED) AS flags,
       CAST(e.level AS SIGNED) AS level, CAST(e.ilevel AS SIGNED) AS ilevel,
       CAST(COALESCE(e.jobs, 0) AS SIGNED) AS jobs, CAST(COALESCE(e.slot, 0) AS SIGNED) AS slots
FROM item_basic b LEFT JOIN item_equipment e ON e.itemId = b.itemid";
/// The item search's WHERE: name (2 binds), then category, job bit, slot bit, lowest and
/// highest level, each bound twice (0 = any). GM-only items (flag 0x2) never show.
const ITEM_WHERE: &str = "WHERE b.itemid > 0 AND (b.flags & 2) = 0
         AND (? = '' OR b.name LIKE ? ESCAPE '|')
         AND (? = 0 OR b.aH = ?)
         AND (? = 0 OR (e.jobs & ?) <> 0)
         AND (? = 0 OR (e.slot & ?) <> 0)
         AND (? = 0 OR e.level >= ?)
         AND (? = 0 OR e.level <= ?)";

fn item_row(r: &sqlx::mysql::MySqlRow) -> Result<ItemRaw, sqlx::Error> {
    Ok(ItemRaw {
        id: r.try_get("id")?,
        name: r.try_get("name")?,
        kind: r.try_get("kind")?,
        category: r.try_get("category")?,
        stack: r.try_get("stack")?,
        flags: r.try_get("flags")?,
        level: r.try_get("level")?,
        ilevel: r.try_get("ilevel")?,
        jobs: r.try_get("jobs")?,
        slots: r.try_get("slots")?,
    })
}

impl MariaDb {
    /// `Done` when an UPDATE on an open ticket changed a row; else whether the ticket exists.
    async fn ticket_outcome(&self, id: i64, rows: u64) -> DbResult<Acted> {
        if rows > 0 {
            return Ok(Acted::Done);
        }
        let exists = sqlx::query("SELECT 1 AS x FROM help_desk WHERE id = ?").bind(id).fetch_optional(&self.pool).await?;
        Ok(if exists.is_some() { Acted::NoChange } else { Acted::NotFound })
    }
}

/// Columns `ticket_row` reads, from [`TICKET_FROM`].
const TICKET_COLS: &str = "CAST(h.id AS SIGNED) AS id, CAST(h.charid AS SIGNED) AS charid, c.charname,
       CAST(NULLIF(c.accid, 0) AS SIGNED) AS accid, a.login,
       CAST(COALESCE((SELECT MAX(g.gmlevel) FROM chars g WHERE g.accid = c.accid AND c.accid <> 0), 0) AS SIGNED) AS gm,
       h.message, h.response,
       CAST(UNIX_TIMESTAMP(h.created_at) AS SIGNED) AS created,
       CAST(UNIX_TIMESTAMP(h.responded_at) AS SIGNED) AS responded,
       CAST(UNIX_TIMESTAMP(h.deleted_at) AS SIGNED) AS closed";

const TICKET_FROM: &str = "FROM help_desk h LEFT JOIN chars c ON c.charid = h.charid LEFT JOIN accounts a ON a.id = c.accid";

/// Ticket text as stored: the client's bytes, NUL-padded. Non-ASCII (Shift-JIS) bytes show as U+FFFD.
fn ticket_text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim_end().to_string()
}

fn ticket_row(r: &sqlx::mysql::MySqlRow) -> Result<Ticket, sqlx::Error> {
    let message: Vec<u8> = r.try_get("message")?;
    let response: Option<Vec<u8>> = r.try_get("response")?;
    Ok(Ticket {
        id: r.try_get("id")?,
        charid: r.try_get("charid")?,
        character: r.try_get("charname")?,
        accid: r.try_get("accid")?,
        login: r.try_get("login")?,
        gm: r.try_get("gm")?,
        message: ticket_text(&message),
        response: response.as_deref().map(ticket_text),
        created: r.try_get("created")?,
        responded: r.try_get("responded")?,
        closed: r.try_get("closed")?,
    })
}

/// The body of [`MariaDb::create_account`], run with the lock held. The statements are LSB's
/// (`auth_session.cpp`, `LOGIN_CREATE`): `timecreate` is local time as a `DATETIME`, and
/// `timelastmodify` is given `NULL`, which this TIMESTAMP column turns into the current time.
async fn insert_account(conn: &mut sqlx::MySqlConnection, login: &str, hash: &str) -> DbResult<Created> {
    for attempt in 0..2 {
        if sqlx::query("SELECT id FROM accounts WHERE login = ?").bind(login).fetch_optional(&mut *conn).await?.is_some() {
            return Ok(Created::Taken);
        }
        let max: i64 = sqlx::query("SELECT CAST(COALESCE(MAX(id), 0) AS SIGNED) AS max_id FROM accounts").fetch_one(&mut *conn).await?.try_get("max_id")?;
        let id = (max + 1).max(1000);
        let done = sqlx::query(
            "INSERT INTO accounts (id, login, password, timecreate, timelastmodify, status, priv) VALUES (?, ?, ?, NOW(), NULL, 1, 1)",
        )
        .bind(id)
        .bind(login)
        .bind(hash)
        .execute(&mut *conn)
        .await;
        match done {
            Ok(_) => return Ok(Created::Account(id)),
            // 1062: the id is taken, by an account LSB's login server made in between. Look again once.
            Err(sqlx::Error::Database(d)) if attempt == 0 && d.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>().is_some_and(|m| m.number() == 1062) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(DbError::Failed("the account id was taken twice".into()))
}

enum Locked {
    /// `gm`: the highest in-game GM level among the account's characters.
    Ok { status: i64, gm: i64 },
    Missing,
}

/// Locks an account row (and reads its characters' GM levels) in the caller's transaction.
async fn lock_account(tx: &mut sqlx::Transaction<'_, sqlx::MySql>, id: i64) -> DbResult<Locked> {
    let r = sqlx::query(
        "SELECT CAST(a.status AS SIGNED) AS status,
                CAST(COALESCE((SELECT MAX(c.gmlevel) FROM chars c WHERE c.accid = a.id), 0) AS SIGNED) AS gm
         FROM accounts a WHERE a.id = ? FOR UPDATE",
    )
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?;
    let Some(r) = r else { return Ok(Locked::Missing) };
    Ok(Locked::Ok { status: r.try_get("status")?, gm: r.try_get("gm")? })
}
