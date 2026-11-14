//! The site's own SQLite file (`DATA_DIR/web.db`): sessions, spent owner-link
//! nonces and the action log. Nothing the site owns goes into `xidb`.

use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::path::Path;
use std::str::FromStr;

const SCHEMA_V1: &str = "
CREATE TABLE sessions (
  id_hash    BLOB PRIMARY KEY,
  kind       TEXT NOT NULL CHECK (kind IN ('owner', 'account')),
  accid      INTEGER,
  login      TEXT,
  csrf       TEXT NOT NULL,
  created    INTEGER NOT NULL,
  last_seen  INTEGER NOT NULL,
  expires    INTEGER NOT NULL,
  ip         TEXT NOT NULL,
  user_agent TEXT
);
CREATE INDEX sessions_accid ON sessions (accid);
CREATE TABLE owner_nonces (
  nonce TEXT PRIMARY KEY,
  exp   INTEGER NOT NULL
);
CREATE TABLE action_log (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  at          INTEGER NOT NULL,
  actor_kind  TEXT NOT NULL,
  actor_accid INTEGER,
  actor_login TEXT,
  actor_role  TEXT NOT NULL,
  ip          TEXT,
  action      TEXT NOT NULL,
  target_kind TEXT,
  target_id   INTEGER,
  target_name TEXT,
  reason      TEXT,
  detail      TEXT,
  result      TEXT NOT NULL
);
CREATE INDEX action_log_at ON action_log (at);
CREATE INDEX action_log_target ON action_log (target_kind, target_id);
";

const SCHEMA_V2: &str = "
CREATE TABLE news (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  title      TEXT NOT NULL,
  body       TEXT NOT NULL,
  author     TEXT NOT NULL,
  published  INTEGER,
  pinned     INTEGER NOT NULL DEFAULT 0,
  created    INTEGER NOT NULL,
  updated    INTEGER NOT NULL,
  updated_by TEXT
);
CREATE INDEX news_published ON news (published);
CREATE TABLE site_content (
  key        TEXT PRIMARY KEY,
  body       TEXT NOT NULL,
  updated    INTEGER NOT NULL,
  updated_by TEXT
);
";

const SCHEMA_V3: &str = "
CREATE TABLE settings (
  key        TEXT PRIMARY KEY,
  value      TEXT NOT NULL,
  updated    INTEGER NOT NULL,
  updated_by TEXT
);
CREATE TABLE invites (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  code_hash  BLOB NOT NULL UNIQUE,
  hint       TEXT NOT NULL,
  note       TEXT NOT NULL DEFAULT '',
  max_uses   INTEGER NOT NULL,
  uses       INTEGER NOT NULL DEFAULT 0,
  expires    INTEGER,
  created    INTEGER NOT NULL,
  created_by TEXT NOT NULL,
  revoked    INTEGER NOT NULL DEFAULT 0
);
";

/// An invite code as the owner's list shows it. The code itself is never stored: only its
/// SHA-256, and the last four characters (`hint`) so the list tells codes apart.
#[derive(Debug, Clone, Serialize)]
pub struct Invite {
    pub id: i64,
    pub hint: String,
    pub note: String,
    pub max_uses: i64,
    pub uses: i64,
    pub expires: Option<i64>,
    pub created: i64,
    pub created_by: String,
    pub revoked: bool,
}

/// What a new invite is made with.
pub struct NewInvite<'a> {
    pub code_hash: Vec<u8>,
    pub hint: &'a str,
    pub note: &'a str,
    pub max_uses: i64,
    pub expires: Option<i64>,
    pub by: &'a str,
    pub now: i64,
}

/// The code as typed -> what is hashed: letters and digits only, upper case, so spaces, dashes
/// and case don't matter. `None` when nothing usable is left or it is absurdly long.
pub fn normalize_code(raw: &str) -> Option<String> {
    let c: String = raw.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect();
    (!c.is_empty() && c.len() <= 40).then_some(c)
}

pub fn code_hash(normalized: &str) -> Vec<u8> {
    Sha256::digest(normalized.as_bytes()).to_vec()
}

const ACTION_COLS: &str = "id, at, actor_kind, actor_login, actor_role, ip, action, target_kind, target_id, target_name, reason, detail, result";

fn action_row(r: &sqlx::sqlite::SqliteRow) -> Result<ActionRow, sqlx::Error> {
    Ok(ActionRow {
        id: r.try_get("id")?,
        at: r.try_get("at")?,
        actor_kind: r.try_get("actor_kind")?,
        actor_login: r.try_get("actor_login")?,
        actor_role: r.try_get("actor_role")?,
        ip: r.try_get("ip")?,
        action: r.try_get("action")?,
        target_kind: r.try_get("target_kind")?,
        target_id: r.try_get("target_id")?,
        target_name: r.try_get("target_name")?,
        reason: r.try_get("reason")?,
        detail: r.try_get("detail")?,
        result: r.try_get("result")?,
    })
}

/// How a session row is found: the cookie holds the id, the table only its SHA-256.
fn hash_id(id: &str) -> Vec<u8> {
    Sha256::digest(id.as_bytes()).to_vec()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionKind {
    Owner,
    Account,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id_hash: Vec<u8>,
    pub kind: SessionKind,
    pub accid: Option<i64>,
    pub login: Option<String>,
    pub csrf: String,
    pub created: i64,
    pub last_seen: i64,
    pub expires: i64,
}

/// What a new session records.
pub struct NewSession<'a> {
    pub kind: SessionKind,
    pub accid: Option<i64>,
    pub login: Option<&'a str>,
    pub ip: &'a str,
    pub user_agent: Option<&'a str>,
    pub now: i64,
    /// Seconds until the session ends whatever happens.
    pub lifetime: i64,
}

#[derive(Debug, Clone, Default)]
pub struct NewAction {
    pub actor_kind: &'static str,
    pub actor_accid: Option<i64>,
    pub actor_login: Option<String>,
    pub actor_role: &'static str,
    pub ip: Option<String>,
    pub action: &'static str,
    pub target_kind: Option<&'static str>,
    pub target_id: Option<i64>,
    pub target_name: Option<String>,
    pub reason: Option<String>,
    pub detail: Option<String>,
    pub result: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActionRow {
    pub id: i64,
    pub at: i64,
    pub actor_kind: String,
    pub actor_login: Option<String>,
    pub actor_role: String,
    /// Kept in the log but not sent to the admin pages.
    #[serde(skip)]
    pub ip: Option<String>,
    pub action: String,
    pub target_kind: Option<String>,
    pub target_id: Option<i64>,
    pub target_name: Option<String>,
    pub reason: Option<String>,
    /// JSON the action wrote (for a ban: its end), never a password.
    pub detail: Option<String>,
    pub result: String,
}

/// What the Admin log page asks for. Page numbers start at 1.
#[derive(Debug, Clone, Default)]
pub struct LogQuery {
    pub q: String,
    pub actor: Option<String>,
    pub action: Option<String>,
    /// Only rows at or after this time.
    pub since: Option<i64>,
    /// News and site content rows are the owner's: others don't see them.
    pub owner: bool,
    pub page: i64,
    pub per: i64,
}

/// A news post as the admin editor sees it. `updated_by` is the editor's login: staff only.
#[derive(Debug, Clone, Serialize)]
pub struct NewsPost {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub author: String,
    pub published: Option<i64>,
    pub pinned: bool,
    pub created: i64,
    pub updated: i64,
    pub updated_by: Option<String>,
}

/// What a post is written with.
#[derive(Debug, Clone)]
pub struct PostFields {
    pub title: String,
    pub body: String,
    pub author: String,
    pub pinned: bool,
}

/// One stored block of site content.
#[derive(Debug, Clone, Serialize)]
pub struct ContentRow {
    pub key: String,
    pub body: String,
    pub updated: i64,
    pub updated_by: Option<String>,
}

fn post_row(r: &sqlx::sqlite::SqliteRow) -> Result<NewsPost, sqlx::Error> {
    Ok(NewsPost {
        id: r.try_get("id")?,
        title: r.try_get("title")?,
        body: r.try_get("body")?,
        author: r.try_get("author")?,
        published: r.try_get("published")?,
        pinned: r.try_get::<i64, _>("pinned")? != 0,
        created: r.try_get("created")?,
        updated: r.try_get("updated")?,
        updated_by: r.try_get("updated_by")?,
    })
}

const POST_COLS: &str = "id, title, body, author, published, pinned, created, updated, updated_by";

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

/// A random cookie value or CSRF token: 32 bytes, base64url.
pub fn random_token() -> String {
    use base64::Engine;
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).expect("the system random source works");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

impl Store {
    /// Opens (creating if needed) `dir/web.db` in WAL mode and brings its schema up to date.
    pub async fn open(dir: &Path) -> Result<Store, sqlx::Error> {
        let opts = SqliteConnectOptions::new()
            .filename(dir.join("web.db"))
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal);
        Store::with(opts, 4).await
    }

    /// An empty store in memory, for tests.
    pub async fn memory() -> Result<Store, sqlx::Error> {
        // One connection: every connection to ":memory:" would be its own database.
        Store::with(SqliteConnectOptions::from_str("sqlite::memory:")?, 1).await
    }

    async fn with(opts: SqliteConnectOptions, max: u32) -> Result<Store, sqlx::Error> {
        let pool = SqlitePoolOptions::new().max_connections(max).connect_with(opts).await?;
        let store = Store { pool };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> Result<(), sqlx::Error> {
        let v: i64 = sqlx::query("PRAGMA user_version").fetch_one(&self.pool).await?.try_get(0)?;
        if v < 1 {
            let mut tx = self.pool.begin().await?;
            for stmt in SCHEMA_V1.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                sqlx::query(stmt).execute(&mut *tx).await?;
            }
            sqlx::query("PRAGMA user_version = 1").execute(&mut *tx).await?;
            tx.commit().await?;
        }
        if v < 2 {
            let mut tx = self.pool.begin().await?;
            for stmt in SCHEMA_V2.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                sqlx::query(stmt).execute(&mut *tx).await?;
            }
            sqlx::query("PRAGMA user_version = 2").execute(&mut *tx).await?;
            tx.commit().await?;
        }
        if v < 3 {
            let mut tx = self.pool.begin().await?;
            for stmt in SCHEMA_V3.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                sqlx::query(stmt).execute(&mut *tx).await?;
            }
            sqlx::query("PRAGMA user_version = 3").execute(&mut *tx).await?;
            tx.commit().await?;
        }
        Ok(())
    }

    // ---- sessions ----

    /// Starts a session and returns its cookie value and CSRF token.
    pub async fn create_session(&self, n: NewSession<'_>) -> Result<(String, String), sqlx::Error> {
        let NewSession { kind, accid, login, ip, user_agent, now, lifetime } = n;
        let id = random_token();
        let csrf = random_token();
        sqlx::query(
            "INSERT INTO sessions (id_hash, kind, accid, login, csrf, created, last_seen, expires, ip, user_agent)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(hash_id(&id))
        .bind(match kind { SessionKind::Owner => "owner", SessionKind::Account => "account" })
        .bind(accid)
        .bind(login)
        .bind(&csrf)
        .bind(now)
        .bind(now)
        .bind(now + lifetime)
        .bind(ip)
        .bind(user_agent.map(|u| u.chars().take(200).collect::<String>()))
        .execute(&self.pool)
        .await?;
        Ok((id, csrf))
    }

    pub async fn session(&self, cookie: &str) -> Result<Option<Session>, sqlx::Error> {
        let r = sqlx::query(
            "SELECT id_hash, kind, accid, login, csrf, created, last_seen, expires FROM sessions WHERE id_hash = ?",
        )
        .bind(hash_id(cookie))
        .fetch_optional(&self.pool)
        .await?;
        Ok(match r {
            Some(r) => Some(Session {
                id_hash: r.try_get("id_hash")?,
                kind: if r.try_get::<String, _>("kind")? == "owner" { SessionKind::Owner } else { SessionKind::Account },
                accid: r.try_get("accid")?,
                login: r.try_get("login")?,
                csrf: r.try_get("csrf")?,
                created: r.try_get("created")?,
                last_seen: r.try_get("last_seen")?,
                expires: r.try_get("expires")?,
            }),
            None => None,
        })
    }

    /// Records activity, at most once a minute.
    pub async fn touch(&self, s: &Session, now: i64) -> Result<(), sqlx::Error> {
        if now - s.last_seen >= 60 {
            sqlx::query("UPDATE sessions SET last_seen = ? WHERE id_hash = ?")
                .bind(now)
                .bind(&s.id_hash)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    pub async fn end_session(&self, id_hash: &[u8]) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM sessions WHERE id_hash = ?").bind(id_hash).execute(&self.pool).await?;
        Ok(())
    }

    /// Ends every session of an account except `keep` (a password change keeps the one that made it).
    pub async fn end_account_sessions(&self, accid: i64, keep: Option<&[u8]>) -> Result<u64, sqlx::Error> {
        let r = sqlx::query("DELETE FROM sessions WHERE accid = ? AND id_hash IS NOT ?")
            .bind(accid)
            .bind(keep)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    pub async fn session_count(&self) -> Result<i64, sqlx::Error> {
        sqlx::query("SELECT COUNT(*) FROM sessions").fetch_one(&self.pool).await?.try_get(0)
    }

    // ---- owner links ----

    /// Remembers an owner-link nonce; false when it was already used.
    pub async fn use_nonce(&self, nonce: &str, exp: i64) -> Result<bool, sqlx::Error> {
        let r = sqlx::query("INSERT OR IGNORE INTO owner_nonces (nonce, exp) VALUES (?, ?)")
            .bind(nonce)
            .bind(exp)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() == 1)
    }

    // ---- action log ----

    pub async fn log(&self, a: NewAction, now: i64) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO action_log (at, actor_kind, actor_accid, actor_login, actor_role, ip, action,
                                     target_kind, target_id, target_name, reason, detail, result)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(now)
        .bind(a.actor_kind)
        .bind(a.actor_accid)
        .bind(a.actor_login)
        .bind(a.actor_role)
        .bind(a.ip)
        .bind(a.action)
        .bind(a.target_kind)
        .bind(a.target_id)
        .bind(a.target_name)
        .bind(a.reason)
        .bind(a.detail)
        .bind(a.result)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Newest first; `before` is the id of the last row of the previous page.
    pub async fn actions(&self, before: Option<i64>, per: i64, only_admin: bool, owner: bool) -> Result<Vec<ActionRow>, sqlx::Error> {
        let rows = sqlx::query(&format!(
            "SELECT {ACTION_COLS} FROM action_log
             WHERE (? IS NULL OR id < ?)
               AND (? = 0 OR (actor_role IN ('owner', 'moderator', 'system') AND action NOT IN ('sign_in', 'sign_in_failed')))
               AND (? = 1 OR target_kind IS NULL OR target_kind NOT IN ('news', 'content', 'invite', 'setting'))
             ORDER BY id DESC LIMIT ?"
        ))
        .bind(before)
        .bind(before)
        .bind(only_admin as i64)
        .bind(owner as i64)
        .bind(per)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(action_row).collect()
    }

    /// One page of the log, newest first, with what matched in all.
    pub async fn log_page(&self, q: &LogQuery) -> Result<(Vec<ActionRow>, i64), sqlx::Error> {
        let like = format!("%{}%", q.q.replace('|', "||").replace('%', "|%").replace('_', "|_"));
        let wh = "WHERE (? = '' OR target_name LIKE ? ESCAPE '|' OR reason LIKE ? ESCAPE '|' OR actor_login LIKE ? ESCAPE '|')
                    AND (? IS NULL OR actor_login = ?) AND (? IS NULL OR action = ?) AND (? IS NULL OR at >= ?)
                    AND (? = 1 OR target_kind IS NULL OR target_kind NOT IN ('news', 'content', 'invite', 'setting'))";
        let total: i64 = sqlx::query(&format!("SELECT COUNT(*) FROM action_log {wh}"))
            .bind(&q.q).bind(&like).bind(&like).bind(&like)
            .bind(&q.actor).bind(&q.actor).bind(&q.action).bind(&q.action).bind(q.since).bind(q.since).bind(q.owner as i64)
            .fetch_one(&self.pool)
            .await?
            .try_get(0)?;
        let rows = sqlx::query(&format!("SELECT {ACTION_COLS} FROM action_log {wh} ORDER BY id DESC LIMIT ? OFFSET ?"))
            .bind(&q.q).bind(&like).bind(&like).bind(&like)
            .bind(&q.actor).bind(&q.actor).bind(&q.action).bind(&q.action).bind(q.since).bind(q.since).bind(q.owner as i64)
            .bind(q.per)
            .bind((q.page - 1) * q.per)
            .fetch_all(&self.pool)
            .await?;
        Ok((rows.iter().map(action_row).collect::<Result<_, _>>()?, total))
    }

    /// Everyone who has acted and every action done, for the log's filters.
    pub async fn log_facets(&self, owner: bool) -> Result<(Vec<String>, Vec<String>), sqlx::Error> {
        let actors = sqlx::query("SELECT DISTINCT actor_login FROM action_log WHERE actor_login IS NOT NULL AND actor_role IN ('owner', 'moderator') ORDER BY actor_login")
            .fetch_all(&self.pool)
            .await?;
        let actions = sqlx::query("SELECT DISTINCT action FROM action_log WHERE ? = 1 OR target_kind IS NULL OR target_kind NOT IN ('news', 'content', 'invite', 'setting') ORDER BY action").bind(owner as i64).fetch_all(&self.pool).await?;
        Ok((
            actors.iter().map(|r| r.try_get(0)).collect::<Result<_, _>>()?,
            actions.iter().map(|r| r.try_get(0)).collect::<Result<_, _>>()?,
        ))
    }

    /// What was done to these targets (`("account", 1001)`), newest first.
    pub async fn history(&self, targets: &[(&str, i64)], limit: i64) -> Result<Vec<ActionRow>, sqlx::Error> {
        if targets.is_empty() {
            return Ok(vec![]);
        }
        // Placeholders only: the kinds and ids are bound like any other value.
        let cond = vec!["(target_kind = ? AND target_id = ?)"; targets.len()].join(" OR ");
        let sql = format!("SELECT {ACTION_COLS} FROM action_log WHERE ({cond}) AND result = 'ok' ORDER BY id DESC LIMIT ?");
        let mut query = sqlx::query(&sql);
        for (kind, id) in targets {
            query = query.bind(*kind).bind(*id);
        }
        let rows = query.bind(limit).fetch_all(&self.pool).await?;
        rows.iter().map(action_row).collect()
    }

    pub async fn action_count(&self) -> Result<i64, sqlx::Error> {
        sqlx::query("SELECT COUNT(*) FROM action_log").fetch_one(&self.pool).await?.try_get(0)
    }

    // ---- news ----

    /// Published posts, pinned first and then newest first, with how many there are in all.
    pub async fn news_public(&self, page: i64, per: i64) -> Result<(Vec<NewsPost>, i64), sqlx::Error> {
        let total: i64 = sqlx::query("SELECT COUNT(*) FROM news WHERE published IS NOT NULL").fetch_one(&self.pool).await?.try_get(0)?;
        let rows = sqlx::query(&format!(
            "SELECT {POST_COLS} FROM news WHERE published IS NOT NULL ORDER BY pinned DESC, published DESC, id DESC LIMIT ? OFFSET ?"
        ))
        .bind(per)
        .bind((page - 1) * per)
        .fetch_all(&self.pool)
        .await?;
        Ok((rows.iter().map(post_row).collect::<Result<_, _>>()?, total))
    }

    pub async fn news_published_post(&self, id: i64) -> Result<Option<NewsPost>, sqlx::Error> {
        let r = sqlx::query(&format!("SELECT {POST_COLS} FROM news WHERE id = ? AND published IS NOT NULL"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        r.as_ref().map(post_row).transpose()
    }

    pub async fn news_post(&self, id: i64) -> Result<Option<NewsPost>, sqlx::Error> {
        let r = sqlx::query(&format!("SELECT {POST_COLS} FROM news WHERE id = ?")).bind(id).fetch_optional(&self.pool).await?;
        r.as_ref().map(post_row).transpose()
    }

    /// Every post, drafts too, newest edit first (the admin list).
    pub async fn news_all(&self) -> Result<Vec<NewsPost>, sqlx::Error> {
        let rows = sqlx::query(&format!("SELECT {POST_COLS} FROM news ORDER BY updated DESC, id DESC LIMIT 500"))
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(post_row).collect()
    }

    pub async fn news_create(&self, f: &PostFields, publish: bool, by: &str, now: i64) -> Result<i64, sqlx::Error> {
        let r = sqlx::query("INSERT INTO news (title, body, author, published, pinned, created, updated, updated_by) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&f.title)
            .bind(&f.body)
            .bind(&f.author)
            .bind(publish.then_some(now))
            .bind(f.pinned as i64)
            .bind(now)
            .bind(now)
            .bind(by)
            .execute(&self.pool)
            .await?;
        Ok(r.last_insert_rowid())
    }

    /// Rewrites a post. `publish`: `Some(true)` publishes (keeping the original date if it
    /// was already out), `Some(false)` turns it back into a draft, `None` leaves it alone.
    /// False when the post doesn't exist.
    pub async fn news_update(&self, id: i64, f: &PostFields, publish: Option<bool>, by: &str, now: i64) -> Result<bool, sqlx::Error> {
        let r = sqlx::query(
            "UPDATE news SET title = ?, body = ?, author = ?, pinned = ?, updated = ?, updated_by = ?,
                 published = CASE ? WHEN 1 THEN COALESCE(published, ?) WHEN 0 THEN NULL ELSE published END
             WHERE id = ?",
        )
        .bind(&f.title)
        .bind(&f.body)
        .bind(&f.author)
        .bind(f.pinned as i64)
        .bind(now)
        .bind(by)
        .bind(publish.map(|p| p as i64))
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn news_delete(&self, id: i64) -> Result<bool, sqlx::Error> {
        Ok(sqlx::query("DELETE FROM news WHERE id = ?").bind(id).execute(&self.pool).await?.rows_affected() == 1)
    }

    // ---- site content ----

    pub async fn content_all(&self) -> Result<Vec<ContentRow>, sqlx::Error> {
        let rows = sqlx::query("SELECT key, body, updated, updated_by FROM site_content").fetch_all(&self.pool).await?;
        rows.iter()
            .map(|r| Ok(ContentRow { key: r.try_get("key")?, body: r.try_get("body")?, updated: r.try_get("updated")?, updated_by: r.try_get("updated_by")? }))
            .collect()
    }

    pub async fn content_set(&self, key: &str, body: &str, by: &str, now: i64) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO site_content (key, body, updated, updated_by) VALUES (?, ?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET body = excluded.body, updated = excluded.updated, updated_by = excluded.updated_by",
        )
        .bind(key)
        .bind(body)
        .bind(now)
        .bind(by)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Back to the default: the stored row goes. False when there was none.
    pub async fn content_reset(&self, key: &str) -> Result<bool, sqlx::Error> {
        Ok(sqlx::query("DELETE FROM site_content WHERE key = ?").bind(key).execute(&self.pool).await?.rows_affected() == 1)
    }

    // ---- settings and sign-up invites ----

    pub async fn setting(&self, key: &str) -> Result<Option<String>, sqlx::Error> {
        sqlx::query("SELECT value FROM settings WHERE key = ?").bind(key).fetch_optional(&self.pool).await?.map(|r| r.try_get("value")).transpose()
    }

    pub async fn set_setting(&self, key: &str, value: &str, by: &str, now: i64) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO settings (key, value, updated, updated_by) VALUES (?, ?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated = excluded.updated, updated_by = excluded.updated_by",
        )
        .bind(key)
        .bind(value)
        .bind(now)
        .bind(by)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Does sign-up need an invite code? On unless the owner switched it off.
    pub async fn invite_required(&self) -> Result<bool, sqlx::Error> {
        Ok(self.setting("signup_needs_invite").await?.as_deref() != Some("0"))
    }

    pub async fn invite_create(&self, n: NewInvite<'_>) -> Result<i64, sqlx::Error> {
        let r = sqlx::query("INSERT INTO invites (code_hash, hint, note, max_uses, expires, created, created_by) VALUES (?, ?, ?, ?, ?, ?, ?)")
            .bind(n.code_hash)
            .bind(n.hint)
            .bind(n.note)
            .bind(n.max_uses)
            .bind(n.expires)
            .bind(n.now)
            .bind(n.by)
            .execute(&self.pool)
            .await?;
        Ok(r.last_insert_rowid())
    }

    pub async fn invites(&self) -> Result<Vec<Invite>, sqlx::Error> {
        let rows = sqlx::query("SELECT id, hint, note, max_uses, uses, expires, created, created_by, revoked FROM invites ORDER BY id DESC").fetch_all(&self.pool).await?;
        rows.iter()
            .map(|r| {
                Ok(Invite {
                    id: r.try_get("id")?,
                    hint: r.try_get("hint")?,
                    note: r.try_get("note")?,
                    max_uses: r.try_get("max_uses")?,
                    uses: r.try_get("uses")?,
                    expires: r.try_get("expires")?,
                    created: r.try_get("created")?,
                    created_by: r.try_get("created_by")?,
                    revoked: r.try_get::<i64, _>("revoked")? != 0,
                })
            })
            .collect()
    }

    pub async fn invite(&self, id: i64) -> Result<Option<Invite>, sqlx::Error> {
        Ok(self.invites().await?.into_iter().find(|i| i.id == id))
    }

    /// Stops a code working. False when there is no such invite.
    pub async fn invite_revoke(&self, id: i64) -> Result<bool, sqlx::Error> {
        Ok(sqlx::query("UPDATE invites SET revoked = 1 WHERE id = ?").bind(id).execute(&self.pool).await?.rows_affected() == 1)
    }

    /// Spends one use of a code, in one statement, so two sign-ups can't both take the last use.
    /// `None` for a code that doesn't exist, is used up, has expired or was revoked: the caller
    /// can't tell which, and neither can anyone it answers.
    pub async fn invite_take(&self, code_hash: &[u8], now: i64) -> Result<Option<i64>, sqlx::Error> {
        sqlx::query(
            "UPDATE invites SET uses = uses + 1
             WHERE code_hash = ? AND revoked = 0 AND uses < max_uses AND (expires IS NULL OR expires > ?)
             RETURNING id",
        )
        .bind(code_hash)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?
        .map(|r| r.try_get("id"))
        .transpose()
    }

    /// Gives back a use taken for a sign-up that didn't make an account.
    pub async fn invite_return(&self, id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE invites SET uses = uses - 1 WHERE id = ? AND uses > 0").bind(id).execute(&self.pool).await?;
        Ok(())
    }

    // ---- housekeeping ----

    /// Drops expired sessions and spent nonces. Returns (sessions, nonces) removed.
    pub async fn purge(&self, now: i64) -> Result<(u64, u64), sqlx::Error> {
        let s = sqlx::query("DELETE FROM sessions WHERE expires <= ?").bind(now).execute(&self.pool).await?;
        let n = sqlx::query("DELETE FROM owner_nonces WHERE exp < ?").bind(now).execute(&self.pool).await?;
        Ok((s.rows_affected(), n.rows_affected()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acct(id: i64, login: &str) -> NewSession<'_> {
        NewSession { kind: SessionKind::Account, accid: Some(id), login: Some(login), ip: "ip", user_agent: None, now: 0, lifetime: 100 }
    }

    #[tokio::test]
    async fn sessions_round_trip_and_store_only_a_hash() {
        let s = Store::memory().await.unwrap();
        let (id, csrf) = s.create_session(NewSession { kind: SessionKind::Account, accid: Some(7), login: Some("kaelyn"), ip: "10.0.0.2", user_agent: Some("ua"), now: 1000, lifetime: 600 }).await.unwrap();
        let got = s.session(&id).await.unwrap().unwrap();
        assert_eq!(got.accid, Some(7));
        assert_eq!(got.csrf, csrf);
        assert_eq!(got.expires, 1600);
        assert_eq!(got.id_hash, hash_id(&id));
        assert_ne!(got.id_hash, id.as_bytes());
        assert!(s.session("nope").await.unwrap().is_none());
        s.end_session(&got.id_hash).await.unwrap();
        assert!(s.session(&id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn nonces_work_once_and_purge_removes_the_old() {
        let s = Store::memory().await.unwrap();
        assert!(s.use_nonce("abc", 100).await.unwrap());
        assert!(!s.use_nonce("abc", 100).await.unwrap());
        s.create_session(NewSession { kind: SessionKind::Owner, accid: None, login: None, ip: "ip", user_agent: None, now: 0, lifetime: 50 }).await.unwrap();
        assert_eq!(s.purge(99).await.unwrap(), (1, 0));
        assert_eq!(s.purge(101).await.unwrap(), (0, 1));
    }

    #[tokio::test]
    async fn ending_an_accounts_sessions_can_keep_one() {
        let s = Store::memory().await.unwrap();
        let (a, _) = s.create_session(acct(1, "a")).await.unwrap();
        let (_b, _) = s.create_session(acct(1, "a")).await.unwrap();
        let (_c, _) = s.create_session(acct(2, "b")).await.unwrap();
        let keep = s.session(&a).await.unwrap().unwrap().id_hash;
        assert_eq!(s.end_account_sessions(1, Some(&keep)).await.unwrap(), 1);
        assert_eq!(s.session_count().await.unwrap(), 2);
        assert_eq!(s.end_account_sessions(1, None).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn the_action_log_pages_by_id() {
        let s = Store::memory().await.unwrap();
        for i in 0..5 {
            s.log(NewAction { actor_kind: "account", actor_role: "owner", action: if i == 0 { "sign_in" } else { "ban" }, result: "ok", ..Default::default() }, 100 + i).await.unwrap();
        }
        let page = s.actions(None, 2, false, true).await.unwrap();
        assert_eq!(page.iter().map(|r| r.id).collect::<Vec<_>>(), vec![5, 4]);
        let next = s.actions(Some(4), 10, false, true).await.unwrap();
        assert_eq!(next.len(), 3);
        // The dashboard's list leaves sign-ins out.
        assert_eq!(s.actions(None, 10, true, true).await.unwrap().len(), 4);
    }
}
