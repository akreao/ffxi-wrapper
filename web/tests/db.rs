//! Account creation against a real MariaDB with LSB's tables. Run by hand:
//!
//!   TEST_DB_PORT=33316 TEST_DB_USER=xiweb TEST_DB_PASSWORD=... TEST_DB_ROOT_PASSWORD=... \
//!   TEST_DB_ALLOW_WIPE=1 cargo test --features db-tests --test db -- --test-threads=1
//!
//! The database must be a scratch one: every test starts by emptying `accounts`, which the
//! site's user (`xiweb`, as `db-init.sh` creates it) must be able to insert into and read.

#![cfg(feature = "db-tests")]

use ffxi_web::config::Config;
use ffxi_web::game::{DbError, Game, MariaDb};
use ffxi_web::model::Created;
use sqlx::mysql::MySqlConnectOptions;
use sqlx::{ConnectOptions, MySqlConnection, Row};

fn env(k: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| panic!("set {k}"))
}

fn game() -> MariaDb {
    let mut cfg = Config::for_tests();
    cfg.db_host = std::env::var("TEST_DB_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    cfg.db_port = env("TEST_DB_PORT").parse().unwrap();
    cfg.db_user = env("TEST_DB_USER");
    cfg.db_password = env("TEST_DB_PASSWORD");
    MariaDb::connect(&cfg)
}

/// A root session set up the way LSB's own connections are (`libmariadb_connection.cpp`).
async fn root() -> MySqlConnection {
    let host = std::env::var("TEST_DB_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let mut c = MySqlConnectOptions::new()
        .host(&host)
        .port(env("TEST_DB_PORT").parse().unwrap())
        .username("root")
        .password(&env("TEST_DB_ROOT_PASSWORD"))
        .database("xidb")
        .connect()
        .await
        .unwrap();
    sqlx::query("SET SESSION sql_mode = CONCAT_WS(',', @@SESSION.sql_mode, 'STRICT_TRANS_TABLES')").execute(&mut c).await.unwrap();
    c
}

async fn fresh() -> MySqlConnection {
    assert_eq!(env("TEST_DB_ALLOW_WIPE"), "1", "these tests empty `accounts`: only point them at a scratch database");
    let mut c = root().await;
    sqlx::query("DROP TRIGGER IF EXISTS test_dup_id").execute(&mut c).await.unwrap();
    sqlx::query("DELETE FROM accounts").execute(&mut c).await.unwrap();
    c
}

const HASH: &str = "$2a$12$CA4XK4wkxyBW5BP7DCxJ3u9cdfFPbtyUAFJp4llXB7AKULe6bvlmO";

/// Every column but the id, login and password, as text, plus how far the two times are from now.
const SHAPE: &str = "SELECT CONCAT_WS('|', content_ids, expansions, features, status, priv, current_email, registration_email) AS fixed,
       ABS(TIMESTAMPDIFF(SECOND, timecreate, NOW())) AS create_age,
       ABS(TIMESTAMPDIFF(SECOND, timelastmodify, NOW())) AS modify_age,
       CAST(timelastmodify AS CHAR) AS modified
  FROM accounts WHERE login = ?";

#[tokio::test]
async fn an_account_has_the_shape_lsb_gives_it() {
    let mut r = fresh().await;
    let g = game();
    assert_eq!(g.create_account("fromsite", HASH).await.unwrap(), Created::Account(1000), "the first id is at least 1000");
    // The same statement LSB runs, in a session set up like LSB's.
    sqlx::query("INSERT INTO accounts(id,login,password,timecreate,timelastmodify,status,priv) VALUES(?, ?, ?, DATE_FORMAT(NOW(), '%Y:%m:%d %H:%i:%S'), NULL, ?, ?)")
        .bind(1001u32).bind("fromlsb").bind(HASH).bind(1u8).bind(1u8)
        .execute(&mut r).await.unwrap();
    let a = sqlx::query(SHAPE).bind("fromsite").fetch_one(&mut r).await.unwrap();
    let b = sqlx::query(SHAPE).bind("fromlsb").fetch_one(&mut r).await.unwrap();
    assert_eq!(a.get::<String, _>("fixed"), b.get::<String, _>("fixed"));
    assert_eq!(a.get::<String, _>("fixed"), "16|4094|253|1|1||");
    for row in [&a, &b] {
        assert!(row.get::<i64, _>("create_age") <= 5, "timecreate is the current time");
        assert!(row.get::<i64, _>("modify_age") <= 5, "NULL became the current time");
        assert_ne!(row.get::<String, _>("modified"), "0000-00-00 00:00:00");
    }
    let site_pw: String = sqlx::query_scalar("SELECT password FROM accounts WHERE login = 'fromsite'").fetch_one(&mut r).await.unwrap();
    assert_eq!(site_pw, HASH, "the hash is stored as given, all 60 characters");
}

#[tokio::test]
async fn a_login_in_any_case_is_taken_and_ids_follow_the_highest() {
    let mut r = fresh().await;
    let g = game();
    assert_eq!(g.create_account("Wren", HASH).await.unwrap(), Created::Account(1000));
    assert_eq!(g.create_account("wREN", HASH).await.unwrap(), Created::Taken);
    sqlx::query("INSERT INTO accounts(id,login,password,timecreate,status,priv) VALUES(4000,'big','x',NOW(),1,1)").execute(&mut r).await.unwrap();
    assert_eq!(g.create_account("next", HASH).await.unwrap(), Created::Account(4001));
}

#[tokio::test]
async fn parallel_sign_ups_get_distinct_ids_and_one_login_wins_once() {
    let _r = fresh().await;
    let g = std::sync::Arc::new(game());
    let tasks: Vec<_> = (0..12).map(|i| { let g = g.clone(); tokio::spawn(async move { g.create_account(&format!("par{i}"), HASH).await.unwrap() }) }).collect();
    let mut ids: Vec<i64> = vec![];
    for t in tasks {
        match t.await.unwrap() { Created::Account(id) => ids.push(id), Created::Taken => panic!("distinct logins") }
    }
    ids.sort();
    assert_eq!(ids, (1000..1012).collect::<Vec<i64>>());

    let tasks: Vec<_> = (0..10).map(|_| { let g = g.clone(); tokio::spawn(async move { g.create_account("samelogin", HASH).await.unwrap() }) }).collect();
    let mut made = 0;
    for t in tasks {
        if matches!(t.await.unwrap(), Created::Account(_)) { made += 1; }
    }
    assert_eq!(made, 1, "ten tries at one login make one account");
}

#[tokio::test]
async fn the_site_user_can_add_accounts_but_not_remove_or_rewrite_them() {
    let _r = fresh().await;
    let host = std::env::var("TEST_DB_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let mut web = MySqlConnectOptions::new().host(&host).port(env("TEST_DB_PORT").parse().unwrap()).username(&env("TEST_DB_USER")).password(&env("TEST_DB_PASSWORD")).database("xidb").connect().await.unwrap();
    game().create_account("granted", HASH).await.unwrap();
    assert!(sqlx::query("DELETE FROM accounts").execute(&mut web).await.is_err());
    assert!(sqlx::query("UPDATE accounts SET priv = 4").execute(&mut web).await.is_err());
    assert!(sqlx::query("INSERT INTO accounts (id, login, password, timecreate, timelastmodify, status, priv, content_ids) VALUES (7, 'x', 'y', NOW(), NULL, 1, 1, 0)").execute(&mut web).await.is_err(), "no column beyond LSB's seven");
}

#[tokio::test]
async fn a_held_lock_makes_the_second_sign_up_give_up_busy() {
    let mut r = fresh().await;
    let got: i64 = sqlx::query_scalar("SELECT GET_LOCK('ffxi_web_signup', 1)").fetch_one(&mut r).await.unwrap();
    assert_eq!(got, 1);
    let out = game().create_account("blocked", HASH).await;
    assert!(matches!(out, Err(DbError::Busy)), "{out:?}");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts").fetch_one(&mut r).await.unwrap();
    assert_eq!(n, 0);
    sqlx::query("SELECT RELEASE_LOCK('ffxi_web_signup')").execute(&mut r).await.unwrap();
    assert_eq!(game().create_account("free", HASH).await.unwrap(), Created::Account(1000));
}

/// LSB's own login server can take the id between the site's look and its insert. The first
/// duplicate-id error is retried once; a second one is an error.
#[tokio::test]
async fn a_duplicate_id_is_retried_once() {
    let mut r = fresh().await;
    sqlx::query("DROP TABLE IF EXISTS test_dup_flag").execute(&mut r).await.unwrap();
    // MyISAM: the counter survives the failed statement's rollback.
    sqlx::query("CREATE TABLE test_dup_flag (n INT NOT NULL) ENGINE=MyISAM").execute(&mut r).await.unwrap();
    sqlx::query("INSERT INTO test_dup_flag VALUES (1)").execute(&mut r).await.unwrap();
    sqlx::query(
        "CREATE TRIGGER test_dup_id BEFORE INSERT ON accounts FOR EACH ROW
         BEGIN
           IF NEW.login = 'racer' AND (SELECT n FROM test_dup_flag) > 0 THEN
             UPDATE test_dup_flag SET n = n - 1;
             SIGNAL SQLSTATE '23000' SET MYSQL_ERRNO = 1062, MESSAGE_TEXT = 'Duplicate entry for key PRIMARY';
           END IF;
         END",
    )
    .execute(&mut r).await.unwrap();
    let g = game();
    assert_eq!(g.create_account("racer", HASH).await.unwrap(), Created::Account(1000), "the second look works");
    sqlx::query("DELETE FROM accounts").execute(&mut r).await.unwrap();
    sqlx::query("UPDATE test_dup_flag SET n = 2").execute(&mut r).await.unwrap();
    assert!(matches!(g.create_account("racer", HASH).await, Err(DbError::Failed(_))), "twice is an error");
    sqlx::query("DROP TRIGGER test_dup_id").execute(&mut r).await.unwrap();
    sqlx::query("DROP TABLE test_dup_flag").execute(&mut r).await.unwrap();
    // And the lock was let go: a new sign-up isn't left waiting.
    assert_eq!(g.create_account("after", HASH).await.unwrap(), Created::Account(1000));
}
