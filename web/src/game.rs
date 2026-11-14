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
    /// `accounts.status` and the account's highest `chars.gmlevel`, in one round trip.
    async fn standing(&self, accid: i64) -> DbResult<Option<Standing>>;

    async fn my_account(&self, accid: i64) -> DbResult<Option<MyAccount>>;
    async fn my_characters(&self, accid: i64) -> DbResult<Vec<CharSummary>>;
    /// `None` when the character isn't on this account (or doesn't exist).
    async fn my_character(&self, accid: i64, charid: i64) -> DbResult<Option<CharDetail>>;
    /// `None` when the character isn't on this account (or doesn't exist).
    async fn my_inventory(&self, accid: i64, charid: i64) -> DbResult<Option<Vec<InventoryItem>>>;
    async fn set_password(&self, accid: i64, hash: &str) -> DbResult<()>;

    async fn counts(&self) -> DbResult<Counts>;
    async fn online(&self) -> DbResult<Vec<OnlineRow>>;
    /// Characters a visitor may be told are in game (not GMs, not hidden).
    async fn public_online(&self) -> DbResult<i64>;
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
            "SELECT CAST(a.status AS SIGNED) AS status,
                    CAST(COALESCE((SELECT MAX(c.gmlevel) FROM chars c WHERE c.accid = a.id), 0) AS SIGNED) AS gm
             FROM accounts a WHERE a.id = ?",
        )
        .bind(accid)
        .fetch_optional(&self.pool)
        .await?;
        Ok(match r {
            Some(r) => Some(Standing { status: r.try_get("status")?, gm: r.try_get("gm")? }),
            None => None,
        })
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
        Ok(Some(CharDetail {
            summary,
            gil: match gil { Some(r) => r.try_get("gil")?, None => 0 },
            cap: match &jobs { Some(r) => r.try_get("cap")?, None => 0 },
            jobs: levels,
        }))
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

    async fn public_online(&self) -> DbResult<i64> {
        let r = sqlx::query(
            "SELECT CAST(COUNT(*) AS SIGNED) AS n
             FROM accounts_sessions x
             JOIN chars c ON c.charid = x.charid
             LEFT JOIN char_flags f ON f.charid = c.charid
             WHERE x.client_port <> 0 AND COALESCE(f.gmHiddenEnabled, 0) = 0 AND c.gmlevel = 0",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(r.try_get("n")?)
    }
}
