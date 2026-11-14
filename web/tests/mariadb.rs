//! The public queries against a real MariaDB. These are the rules the fake in
//! `tests/common` only imitates, so run them whenever a query changes:
//!
//! 1. Start `mariadb:12.3` with a database `xidb`, load LSB's `sql/` tables at the
//!    pinned commit (accounts, chars, char_stats, char_jobs, char_exp, char_flags,
//!    char_skills, accounts_sessions, zone_settings) and `tests/fixtures/public.sql`.
//! 2. Create the website's user with the grants from `docker/lsb/db-init.sh`
//!    (read-only on those tables), so a missing grant fails here.
//! 3. `WEB_TEST_DB_PORT=33307 WEB_TEST_DB_PASSWORD=... cargo test --test mariadb -- --ignored`
//!    (`WEB_TEST_DB_HOST` defaults to 127.0.0.1, `WEB_TEST_DB_USER` to xiweb).

use ffxi_web::config::Config;
use ffxi_web::game::{Game, MariaDb};
use ffxi_web::model::*;

fn db() -> MariaDb {
    let mut cfg = Config::for_tests();
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    cfg.db_host = env("WEB_TEST_DB_HOST", "127.0.0.1");
    cfg.db_port = env("WEB_TEST_DB_PORT", "3306").parse().unwrap();
    cfg.db_user = env("WEB_TEST_DB_USER", "xiweb");
    cfg.db_password = env("WEB_TEST_DB_PASSWORD", "");
    MariaDb::connect(&cfg)
}

fn names(rows: &[RankRow]) -> Vec<&str> {
    rows.iter().map(|r| r.name.as_str()).collect()
}

#[tokio::test]
#[ignore = "needs the fixture database, see the file header"]
async fn who_is_online_follows_the_rules() {
    let list = db().public_online().await.unwrap();
    let who: Vec<&str> = list.iter().map(|p| p.name.as_str()).collect();
    // GM (Halvard), hidden GM (Ghost), a hidden non-GM (Sneaky), a character without an
    // account (Orphan), one still connecting (Linking) and one offline are not listed.
    // A banned account (Brannoc) is in game, so it is.
    assert_eq!(who, ["Awayfox", "Brannoc", "Eaglet", "Pipiru", "Torvald", "Yrsa"]);
    let get = |n: &str| list.iter().find(|p| p.name == n).unwrap();
    for anon in ["Yrsa", "Eaglet"] {
        let p = get(anon);
        assert!(p.anonymous, "{anon}: settings & 4");
        assert!(p.zone.is_none() && p.mjob.is_none() && p.mlvl.is_none() && p.sjob.is_none() && p.slvl.is_none(), "{anon} must carry no job, level or zone");
    }
    assert!(!get("Awayfox").anonymous, "settings 3 has the away bit, not the anon bit");
    let t = get("Torvald");
    assert_eq!((t.zone.as_deref(), t.mjob, t.mlvl, t.sjob, t.slvl, t.nation), (Some("Batallia Downs"), Some(7), Some(52), Some(1), Some(26), 1));
}

#[tokio::test]
#[ignore = "needs the fixture database, see the file header"]
async fn every_board_leaves_out_gms_hidden_banned_and_accountless() {
    let db = db();
    let boards = [Board::Level, Board::Job(0), Board::Job(11), Board::Job(2), Board::Craft(49), Board::Craft(50), Board::Playtime];
    for b in boards {
        let rows = db.public_ranking(b).await.unwrap();
        let who = names(&rows);
        for out in ["Halvard", "Ghost", "Sneaky", "Brannoc", "Orphan"] {
            assert!(!who.contains(&out), "{b:?}: {out} must not be ranked, got {who:?}");
        }
        for (i, r) in rows.iter().enumerate() {
            assert_eq!(r.rank, i as i64 + 1);
        }
    }
}

#[tokio::test]
#[ignore = "needs the fixture database, see the file header"]
async fn the_boards_rank_what_they_say() {
    let db = db();
    let level = db.public_ranking(Board::Level).await.unwrap();
    // Yrsa and Eaglet are /anon, so their levels stay hidden.
    assert_eq!(names(&level), ["Pipiru", "Torvald", "Offline", "Linking", "Awayfox"]);
    assert_eq!((level[0].job, level[0].value), (Some(4), 61));
    assert_eq!((level[1].job, level[1].value), (Some(7), 52));
    assert_eq!((level[2].job, level[2].value), (Some(11), 40));
    let war = db.public_ranking(Board::Job(0)).await.unwrap();
    assert_eq!(names(&war), ["Torvald", "Offline"], "level 1 is not a ranking; levels above 1 are");
    let craft = db.public_ranking(Board::Craft(49)).await.unwrap();
    assert_eq!(names(&craft), ["Pipiru", "Torvald", "Yrsa"], "anonymous characters stay on craft boards: a skill shows no job or level");
    assert_eq!(craft[0].value, 90, "tenths become whole levels");
    assert_eq!(names(&db.public_ranking(Board::Craft(55)).await.unwrap()), ["Pipiru"]);
    let play = db.public_ranking(Board::Playtime).await.unwrap();
    assert_eq!(names(&play), ["Yrsa", "Torvald", "Pipiru", "Awayfox", "Offline", "Eaglet"]);
    assert_eq!(play[0].value, 250, "hours");
}

#[tokio::test]
#[ignore = "needs the fixture database, see the file header"]
async fn the_site_user_can_read_what_the_queries_need_and_write_nothing() {
    use sqlx::mysql::MySqlConnectOptions;
    use sqlx::{ConnectOptions, Connection};
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let mut conn = MySqlConnectOptions::new()
        .host(&env("WEB_TEST_DB_HOST", "127.0.0.1"))
        .port(env("WEB_TEST_DB_PORT", "3306").parse().unwrap())
        .username(&env("WEB_TEST_DB_USER", "xiweb"))
        .password(&env("WEB_TEST_DB_PASSWORD", ""))
        .database("xidb")
        .connect()
        .await
        .unwrap();
    assert!(sqlx::query("UPDATE chars SET playtime = 0").execute(&mut conn).await.is_err());
    assert!(sqlx::query("DELETE FROM char_skills").execute(&mut conn).await.is_err());
    conn.close().await.unwrap();
}
