//! The public queries against a real MariaDB. These are the rules the fake in
//! `tests/common` only imitates, so run them whenever a query changes:
//!
//! 1. Start `mariadb:12.3` with a database `xidb`, load LSB's `sql/` tables at the
//!    pinned commit (accounts, chars, char_stats, char_jobs, char_exp, char_flags,
//!    char_skills, char_inventory, accounts_sessions, zone_settings, item_basic,
//!    item_equipment, auction_house, delivery_box and its `delivery_box_insert` trigger from
//!    `triggers.sql`), `tests/fixtures/public.sql`, then `tests/fixtures/auction.sql`.
//! 2. Create the website's user with the grants from `docker/lsb/db-init.sh`
//!    (the xiweb block), so a missing grant fails here.
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

/// A plain connection as the site's user.
async fn xiweb() -> sqlx::MySqlConnection {
    use sqlx::mysql::MySqlConnectOptions;
    use sqlx::ConnectOptions;
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    MySqlConnectOptions::new()
        .host(&env("WEB_TEST_DB_HOST", "127.0.0.1"))
        .port(env("WEB_TEST_DB_PORT", "3306").parse().unwrap())
        .username(&env("WEB_TEST_DB_USER", "xiweb"))
        .password(&env("WEB_TEST_DB_PASSWORD", ""))
        .database("xidb")
        .connect()
        .await
        .unwrap()
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
    // A banned account (Brannoc) is in game but not listed: the boards omit it, so listing it would show who is banned.
    assert_eq!(who, ["Awayfox", "Eaglet", "Pipiru", "Torvald", "Yrsa"]);
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
    use sqlx::Connection;
    let mut conn = xiweb().await;
    assert!(sqlx::query("UPDATE chars SET playtime = 0").execute(&mut conn).await.is_err());
    assert!(sqlx::query("DELETE FROM char_skills").execute(&mut conn).await.is_err());
    assert!(sqlx::query("DELETE FROM auction_house").execute(&mut conn).await.is_err());
    assert!(sqlx::query("UPDATE auction_house SET price = 1").execute(&mut conn).await.is_err());
    conn.close().await.unwrap();
}

#[tokio::test]
#[ignore = "needs the fixture database, see the file header"]
async fn gives_queue_in_the_delivery_box_without_deadlocks() {
    use std::sync::Arc;
    let db = Arc::new(db());
    let mut conn = xiweb().await;
    // A character with an empty incoming box if the fixture still has one: that's where two
    // transactions holding only gap locks used to deadlock.
    let charid: i64 = sqlx::query_scalar(
        "SELECT CAST(c.charid AS SIGNED) FROM chars c WHERE c.accid <> 0
         ORDER BY (SELECT COUNT(*) FROM delivery_box d WHERE d.charid = c.charid AND d.box = 1), c.charid LIMIT 1",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM delivery_box WHERE charid = ? AND box = 1").bind(charid).fetch_one(&mut conn).await.unwrap();

    let gives = (0..8).map(|_| {
        let db = db.clone();
        tokio::spawn(async move { db.give_item(charid, 4096, 1, "GM", false).await })
    });
    for g in gives.collect::<Vec<_>>() {
        assert_eq!(g.await.unwrap().unwrap(), Given::Done);
    }
    let slots: Vec<i64> = sqlx::query_scalar("SELECT CAST(slot AS SIGNED) FROM delivery_box WHERE charid = ? AND box = 1").bind(charid).fetch_all(&mut conn).await.unwrap();
    assert_eq!(slots.len() as i64, before + 8);
    assert!(slots.iter().all(|s| *s >= 8), "gives queue behind the 8 visible slots: {slots:?}");

    // A Rare item goes once; an item id nothing else uses, so reruns start clean.
    let rare = 20_000 + (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() % 40_000) as i64;
    assert_eq!(db.give_item(charid, rare, 1, "GM", true).await.unwrap(), Given::Done);
    assert_eq!(db.give_item(charid, rare, 1, "GM", true).await.unwrap(), Given::AlreadyHas);
    assert_eq!(db.give_item(-1, 4096, 1, "GM", false).await.unwrap(), Given::NoCharacter);
    // Still no way to change or remove what is in a box.
    assert!(sqlx::query("DELETE FROM delivery_box WHERE charid = ?").bind(charid).execute(&mut conn).await.is_err());
    assert!(sqlx::query("UPDATE delivery_box SET quantity = 99 WHERE charid = ?").bind(charid).execute(&mut conn).await.is_err());
}

#[tokio::test]
#[ignore = "needs the fixture database, see the file header"]
async fn the_auction_house_lists_shown_items_and_their_sides() {
    let db = db();
    let query = ItemQuery { level_cap: 99, page: 1, per: 25, ..Default::default() };
    let up = db.auction_listed(&query).await.unwrap();
    // Distilled water only has sales; the Judge's shield (GM only) and gil never show.
    let ids: Vec<i64> = up.items.iter().map(|l| l.item.id).collect();
    assert_eq!(ids, [16535, 4096]);
    assert_eq!(up.total, 2);
    let fire = &up.items[1];
    assert_eq!((fire.singles, fire.single_low, fire.stacks, fire.stack_low), (2, Some(250), 1, Some(3000)));
    let named = db.auction_listed(&ItemQuery { q: "fire".into(), ..query.clone() }).await.unwrap();
    assert_eq!(named.total, 1);

    let singles = db.auction_side(4096, false, 1, 20).await.unwrap();
    assert_eq!(singles.listed, 2);
    // Capped at one, the cheapest; a listing has no buyer.
    assert_eq!(singles.listings, [AuctionRow { price: 250, at: 1763000100, seller: 0, seller_name: Some("M.H.M.U.".into()), buyer: 0, buyer_name: None }]);
    // Newest sale first, with what it sold for and when.
    let sold: Vec<(i64, i64, i64, i64)> = singles.sales.iter().map(|r| (r.price, r.at, r.seller, r.buyer)).collect();
    assert_eq!(sold, [(260, 1762100000, 0, 10001), (280, 1762000500, 10005, 0)]);
    let stacks = db.auction_side(4096, true, 50, 20).await.unwrap();
    assert_eq!((stacks.listed, stacks.listings.len(), stacks.sales.len()), (1, 1, 0));
}

#[tokio::test]
#[ignore = "needs the fixture database, see the file header"]
async fn the_gm_filter_lists_only_characters_with_a_gm_level() {
    let q = CharQuery { q: String::new(), online: false, gm: true, sort: CharSort::Name, desc: false, page: 1, per: 25 };
    let list = db().admin_characters(&q).await.unwrap();
    let who: Vec<&str> = list.items.iter().map(|c| c.summary.name.as_str()).collect();
    assert_eq!(who, ["Ghost", "Halvard"]);
    assert_eq!(list.total, 2);
    assert_eq!(list.items[1].gm, 3);
}
