//! The public who-is-online list and rankings: who is left out, what a row may
//! hold, the fixed lists behind `job` and `skill`, the 30 s cache and the key lists.

mod common;

use axum::http::StatusCode;
use common::*;
use ffxi_web::routes::public::*;
use serde_json::Value;
use std::sync::atomic::Ordering;

fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}
fn sorted(a: &[&str]) -> Vec<String> {
    let mut k: Vec<String> = a.iter().map(|s| s.to_string()).collect();
    k.sort();
    k
}
fn names(j: &Value, list: &str) -> Vec<String> {
    j[list].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect()
}

/// Everyone a rule has to catch, next to ordinary players.
async fn crowd() -> Site {
    let s = site().await;
    let f = &s.fake;
    f.add_person(Person::new("Torvald").online(7, 52, "Batallia_Downs").level(1, 26).played(400_000).skill(49, 612));
    f.add_person(Person::new("Pipiru").online(4, 61, "Garlaige_Citadel").played(300_000).skill(49, 905).skill(55, 100));
    f.add_person(Person::new("Yrsa").anon().online(5, 44, "Valkurm_Dunes").played(900_000).skill(49, 300));
    f.add_person(Person::new("Halvard").gm(3).online(12, 75, "Ru'Lude_Gardens").played(9_000_000).skill(49, 1000));
    f.add_person(Person::new("Ghost").gm(5).hidden().online(12, 99, "Ru'Lude_Gardens").played(9_000_000));
    f.add_person(Person::new("Brannoc").banned().online(1, 70, "Bastok_Mines").played(5_000_000).skill(49, 999));
    f.add_person(Person::new("Orphan").no_account().online(3, 80, "Bastok_Mines").played(5_000_000).skill(49, 999));
    let mut late = Person::new("Linking").online(2, 30, "Port_Jeuno");
    late.connecting = true;
    f.add_person(late);
    f.add_person(Person::new("Offline").level(11, 40).played(7200));
    s
}

#[tokio::test]
async fn the_online_list_leaves_out_staff_hidden_staff_and_unfinished_logins() {
    let s = crowd().await;
    let j = s.send(Req::get("/api/public/online")).await.json();
    assert_eq!(names(&j, "players"), ["Pipiru", "Torvald", "Yrsa"], "GMs, hidden GMs, banned accounts, accid 0 and connecting ones are not listed (a banned one would show who is banned, as the boards omit them)");
    assert_eq!(j["count"], 3);
    let text = j.to_string();
    for hidden in ["Halvard", "Ghost", "Brannoc", "Orphan", "Linking", "Offline"] {
        assert!(!text.contains(hidden), "{hidden} must not be listed");
    }
}

#[tokio::test]
async fn an_anonymous_character_shows_name_and_nation_only() {
    let s = crowd().await;
    let j = s.send(Req::get("/api/public/online")).await.json();
    let yrsa = j["players"].as_array().unwrap().iter().find(|p| p["name"] == "Yrsa").unwrap();
    assert_eq!(yrsa["anonymous"], true);
    for k in ["mjob", "mlvl", "sjob", "slvl", "zone"] {
        assert!(yrsa[k].is_null(), "{k} of an anonymous character must be null");
    }
    assert_eq!(yrsa["nation"], 1);
    let torvald = j["players"].as_array().unwrap().iter().find(|p| p["name"] == "Torvald").unwrap();
    assert_eq!(torvald["zone"], "Batallia Downs", "underscores in the database name become spaces");
    assert_eq!(torvald["mjob"], 7);
    assert_eq!(torvald["mlvl"], 52);
}

#[tokio::test]
async fn public_routes_show_only_their_listed_keys() {
    let s = crowd().await;
    let j = s.send(Req::get("/api/public/online")).await.json();
    assert_eq!(keys(&j), sorted(&ONLINE_KEYS), "a new key on /api/public/online must be added to ONLINE_KEYS on purpose");
    for p in j["players"].as_array().unwrap() {
        assert_eq!(keys(p), sorted(&ONLINE_PLAYER_KEYS), "a new key on an online player must be added to ONLINE_PLAYER_KEYS on purpose");
    }
    for path in ["level", "job?job=blm", "craft?skill=49", "playtime"] {
        let j = s.send(Req::get(&format!("/api/public/rankings/{path}"))).await.json();
        assert_eq!(keys(&j), sorted(&RANKING_KEYS), "{path}: add new keys to RANKING_KEYS on purpose");
        assert!(!j["rows"].as_array().unwrap().is_empty(), "{path} has rows");
        for r in j["rows"].as_array().unwrap() {
            assert_eq!(keys(r), sorted(&RANK_ROW_KEYS), "{path}: add new row keys to RANK_ROW_KEYS on purpose");
        }
    }
}

#[tokio::test]
async fn the_home_count_is_the_length_of_the_online_list() {
    let s = crowd().await;
    let home = s.send(Req::get("/api/public/home")).await.json();
    let list = s.send(Req::get("/api/public/online")).await.json();
    assert_eq!(home["online"], list["players"].as_array().unwrap().len());
    assert_eq!(home["online"], list["count"]);
}

#[tokio::test]
async fn rankings_leave_out_gms_banned_accounts_and_characters_without_an_account() {
    let s = crowd().await;
    for path in ["level", "job?job=war", "job?job=blm", "craft?skill=49", "playtime"] {
        let j = s.send(Req::get(&format!("/api/public/rankings/{path}"))).await.json();
        let text = j.to_string();
        for out in ["Halvard", "Ghost", "Brannoc", "Orphan"] {
            assert!(!text.contains(out), "{path}: {out} must not be ranked");
        }
    }
    let craft = s.send(Req::get("/api/public/rankings/craft?skill=49")).await.json();
    assert_eq!(names(&craft, "rows"), ["Pipiru", "Torvald", "Yrsa"]);
    assert_eq!(craft["rows"][0]["value"], 90, "skill is shown in whole levels, not tenths");
    let play = s.send(Req::get("/api/public/rankings/playtime")).await.json();
    assert_eq!(names(&play, "rows"), ["Yrsa", "Torvald", "Pipiru", "Offline"]);
    assert_eq!(play["rows"][0]["value"], 250, "hours");
    assert_eq!(play["rows"][0]["rank"], 1);
}

#[tokio::test]
async fn anonymous_characters_lose_their_place_on_the_level_and_job_boards_only() {
    let s = crowd().await;
    let level = s.send(Req::get("/api/public/rankings/level")).await.json();
    assert_eq!(names(&level, "rows")[..2], ["Pipiru", "Torvald"]);
    assert!(!names(&level, "rows").contains(&"Yrsa".to_string()), "/anon hides level, so the level board must too");
    assert_eq!(level["rows"][0]["job"], 4);
    assert_eq!(level["rows"][0]["value"], 61);
    let rdm = s.send(Req::get("/api/public/rankings/job?job=rdm")).await.json();
    assert!(rdm["rows"].as_array().unwrap().is_empty());
    assert!(names(&s.send(Req::get("/api/public/rankings/craft?skill=49")).await.json(), "rows").contains(&"Yrsa".to_string()), "a craft skill reveals no job or level");
}

#[tokio::test]
async fn the_job_board_ranks_one_job_and_takes_the_job_from_a_fixed_list() {
    let s = crowd().await;
    let blm = s.send(Req::get("/api/public/rankings/job?job=blm")).await.json();
    assert_eq!(names(&blm, "rows"), ["Pipiru"]);
    assert_eq!(blm["rows"][0]["job"], 4);
    let upper = s.send(Req::get("/api/public/rankings/job?job=BLM")).await.json();
    assert_eq!(names(&upper, "rows"), ["Pipiru"]);
    for bad in ["job?job=war%60%20FROM%20accounts%3B--", "job?job=", "job?job=mon", "job?job=wa", "job?job=1"] {
        let r = s.send(Req::get(&format!("/api/public/rankings/{bad}"))).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{bad}");
    }
}

#[tokio::test]
async fn the_craft_board_takes_only_skills_48_to_57() {
    let s = crowd().await;
    for ok in [48, 49, 55, 57] {
        assert_eq!(s.send(Req::get(&format!("/api/public/rankings/craft?skill={ok}"))).await.status, StatusCode::OK);
    }
    for bad in ["47", "58", "0", "-1", "49%20OR%201", "abc", "", "1000000000000000000000"] {
        let r = s.send(Req::get(&format!("/api/public/rankings/craft?skill={bad}"))).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "skill={bad}");
    }
    let alchemy = s.send(Req::get("/api/public/rankings/craft?skill=55")).await.json();
    assert_eq!(names(&alchemy, "rows"), ["Pipiru"]);
}

#[tokio::test]
async fn unknown_boards_and_pages_are_refused() {
    let s = crowd().await;
    assert_eq!(s.send(Req::get("/api/public/rankings/gil")).await.status, StatusCode::NOT_FOUND);
    assert_eq!(s.send(Req::get("/api/public/rankings/level?page=0")).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(s.send(Req::get("/api/public/rankings/level?page=x")).await.status, StatusCode::BAD_REQUEST);
    let far = s.send(Req::get("/api/public/rankings/level?page=99999999999999")).await;
    assert_eq!(far.status, StatusCode::OK);
    assert!(far.json()["rows"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn a_board_lists_the_top_100_in_pages_of_25() {
    let s = site().await;
    for i in 0..130 {
        s.fake.add_person(Person::new(&format!("Hero{i:03}")).level(1, 2 + (i % 70)).played(3600 * (1 + i)));
    }
    let p1 = s.send(Req::get("/api/public/rankings/playtime")).await.json();
    assert_eq!(p1["total"], 100);
    assert_eq!(p1["pages"], 4);
    assert_eq!(p1["per_page"], 25);
    assert_eq!(p1["rows"].as_array().unwrap().len(), 25);
    assert_eq!(p1["rows"][0]["name"], "Hero129");
    let p4 = s.send(Req::get("/api/public/rankings/playtime?page=4")).await.json();
    assert_eq!(p4["rows"].as_array().unwrap().len(), 25);
    assert_eq!(p4["rows"][24]["rank"], 100);
    let p5 = s.send(Req::get("/api/public/rankings/playtime?page=5")).await.json();
    assert!(p5["rows"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn answers_are_cached_for_30_seconds() {
    let s = crowd().await;
    for _ in 0..5 {
        s.send(Req::get("/api/public/online")).await;
        s.send(Req::get("/api/public/home")).await;
        s.send(Req::get("/api/public/rankings/level")).await;
        s.send(Req::get("/api/public/rankings/level?page=2")).await;
    }
    assert_eq!(s.fake.0.lock().unwrap().public_reads, 2, "one online read, one level-board read");
    s.fake.add_person(Person::new("Newcomer").online(1, 5, "Bastok_Mines"));
    s.clock.fetch_add(29, Ordering::SeqCst);
    assert_eq!(s.send(Req::get("/api/public/online")).await.json()["count"], 3);
    s.clock.fetch_add(2, Ordering::SeqCst);
    assert_eq!(s.send(Req::get("/api/public/online")).await.json()["count"], 4);
    s.send(Req::get("/api/public/rankings/job?job=war")).await;
    s.send(Req::get("/api/public/rankings/job?job=mnk")).await;
    assert_eq!(s.fake.0.lock().unwrap().public_reads, 2 + 1 + 2, "each board has its own entry");
}

#[tokio::test]
async fn the_public_read_limit_applies() {
    let s = crowd().await;
    for _ in 0..120 {
        assert_eq!(s.send(Req::get("/api/public/rankings/level")).await.status, StatusCode::OK);
    }
    assert_eq!(s.send(Req::get("/api/public/rankings/level")).await.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(s.send(Req::get("/api/public/online")).await.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn the_database_being_down_is_an_error_not_an_empty_list() {
    let s = crowd().await;
    s.fake.0.lock().unwrap().down = true;
    assert_eq!(s.send(Req::get("/api/public/online")).await.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(s.send(Req::get("/api/public/rankings/level")).await.status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn the_new_pages_are_public_and_have_no_inline_script() {
    let s = site().await;
    for path in ["/online", "/rankings"] {
        let r = s.send(Req::get(path)).await;
        assert_eq!(r.status, StatusCode::OK, "{path}");
        assert!(!r.text().contains("/admin"), "{path} must not link the admin panel");
    }
}

/// Many visitors on a cold cache cost one query, not one each.
#[tokio::test]
async fn a_cold_cache_is_filled_by_one_query() {
    let s = crowd().await;
    let mut jobs = Vec::new();
    for _ in 0..40 {
        jobs.push(s.send(Req::get("/api/public/rankings/level")));
        jobs.push(s.send(Req::get("/api/public/online")));
    }
    for r in futures_util_join(jobs).await {
        assert_eq!(r, StatusCode::OK);
    }
    assert_eq!(s.fake.0.lock().unwrap().public_reads, 2, "one read per cache key");
}

/// Polls every request together so they overlap, like real visitors.
async fn futures_util_join<F: std::future::Future<Output = Resp>>(jobs: Vec<F>) -> Vec<StatusCode> {
    use std::task::Poll;
    let mut jobs: Vec<Option<std::pin::Pin<Box<F>>>> = jobs.into_iter().map(|j| Some(Box::pin(j))).collect();
    let mut out: Vec<Option<StatusCode>> = vec![None; jobs.len()];
    std::future::poll_fn(|cx| {
        let mut pending = false;
        for (i, slot) in jobs.iter_mut().enumerate() {
            if let Some(f) = slot {
                match f.as_mut().poll(cx) {
                    Poll::Ready(r) => {
                        out[i] = Some(r.status);
                        *slot = None;
                    }
                    Poll::Pending => pending = true,
                }
            }
        }
        if pending { Poll::Pending } else { Poll::Ready(()) }
    })
    .await;
    out.into_iter().map(|s| s.unwrap()).collect()
}
