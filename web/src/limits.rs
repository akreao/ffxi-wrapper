//! Rate limits: in memory, per process, with the clock passed in so tests need no sleeping.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// Counts events per key over a sliding window.
#[derive(Default)]
pub struct Windows {
    hits: Mutex<HashMap<String, VecDeque<i64>>>,
}

impl Windows {
    /// How many events `key` has in the last `window` seconds.
    pub fn count(&self, key: &str, window: i64, now: i64) -> usize {
        let mut m = self.hits.lock().unwrap();
        match m.get_mut(key) {
            Some(q) => {
                while q.front().is_some_and(|t| *t <= now - window) {
                    q.pop_front();
                }
                q.len()
            }
            None => 0,
        }
    }

    pub fn record(&self, key: &str, now: i64) {
        self.hits.lock().unwrap().entry(key.to_string()).or_default().push_back(now);
    }

    /// Records an event unless the key is already at `limit` in the window. True when allowed.
    pub fn allow(&self, key: &str, limit: usize, window: i64, now: i64) -> bool {
        if self.count(key, window, now) >= limit {
            return false;
        }
        self.record(key, now);
        true
    }

    /// Forgets keys with no events in the last `window` seconds.
    pub fn sweep(&self, window: i64, now: i64) {
        self.hits.lock().unwrap().retain(|_, q| q.back().is_some_and(|t| *t > now - window));
    }
}

const MIN: i64 = 60;
const HOUR: i64 = 60 * MIN;

/// What sign-in is limited by (see the spec's rate-limit table).
pub const FAIL_PAIR: (usize, i64) = (5, 15 * MIN);
pub const FAIL_LOGIN: (usize, i64) = (20, HOUR);
pub const FAIL_ADDR: (usize, i64) = (30, HOUR);

pub struct Limiter {
    pub failures: Windows,
    pub public_gets: Windows,
    pub writes: Windows,
}

impl Limiter {
    pub fn new() -> Limiter {
        Limiter { failures: Windows::default(), public_gets: Windows::default(), writes: Windows::default() }
    }

    fn keys(addr: &str, login: &str) -> [String; 3] {
        let login = login.to_lowercase();
        [format!("pair:{addr}|{login}"), format!("login:{login}"), format!("addr:{addr}")]
    }

    /// Is this (address, login) allowed to try a sign-in now?
    pub fn sign_in_allowed(&self, addr: &str, login: &str, now: i64) -> bool {
        let [pair, by_login, by_addr] = Limiter::keys(addr, login);
        self.failures.count(&pair, FAIL_PAIR.1, now) < FAIL_PAIR.0
            && self.failures.count(&by_login, FAIL_LOGIN.1, now) < FAIL_LOGIN.0
            && self.failures.count(&by_addr, FAIL_ADDR.1, now) < FAIL_ADDR.0
    }

    /// How many more failures this pair may make before it is locked.
    pub fn pair_tries_left(&self, addr: &str, login: &str, now: i64) -> usize {
        let [pair, _, _] = Limiter::keys(addr, login);
        FAIL_PAIR.0.saturating_sub(self.failures.count(&pair, FAIL_PAIR.1, now))
    }

    pub fn sign_in_failed(&self, addr: &str, login: &str, now: i64) {
        for k in Limiter::keys(addr, login) {
            self.failures.record(&k, now);
        }
    }

    /// 120 public GETs a minute per address.
    pub fn public_get_allowed(&self, addr: &str, now: i64) -> bool {
        self.public_gets.allow(addr, 120, MIN, now)
    }

    /// 30 changes a minute per session.
    pub fn write_allowed(&self, session: &str, now: i64) -> bool {
        self.writes.allow(session, 30, MIN, now)
    }

    pub fn sweep(&self, now: i64) {
        self.failures.sweep(HOUR, now);
        self.public_gets.sweep(MIN, now);
        self.writes.sweep(MIN, now);
    }
}

impl Default for Limiter {
    fn default() -> Self {
        Limiter::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_failures_for_one_pair_lock_it_for_fifteen_minutes() {
        let l = Limiter::new();
        for i in 0..5 {
            assert!(l.sign_in_allowed("1.2.3.4", "Kaelyn", i));
            l.sign_in_failed("1.2.3.4", "Kaelyn", i);
        }
        assert!(!l.sign_in_allowed("1.2.3.4", "kaelyn", 10), "login case doesn't matter");
        // Another address, or another login, is unaffected.
        assert!(l.sign_in_allowed("5.6.7.8", "kaelyn", 10));
        assert!(l.sign_in_allowed("1.2.3.4", "someone", 10));
        // The window ends 15 minutes after the failures.
        assert!(!l.sign_in_allowed("1.2.3.4", "kaelyn", 15 * 60 - 1));
        assert!(l.sign_in_allowed("1.2.3.4", "kaelyn", 15 * 60 + 5));
    }

    #[test]
    fn a_login_is_locked_after_twenty_failures_from_any_addresses() {
        let l = Limiter::new();
        for i in 0..20 {
            l.sign_in_failed(&format!("10.0.0.{i}"), "victim", i);
        }
        assert!(!l.sign_in_allowed("10.9.9.9", "victim", 100));
        assert!(l.sign_in_allowed("10.9.9.9", "other", 100));
        assert!(l.sign_in_allowed("10.9.9.9", "victim", 3601 + 20));
    }

    #[test]
    fn an_address_is_locked_after_thirty_failures_on_any_logins() {
        let l = Limiter::new();
        for i in 0..30 {
            l.sign_in_failed("9.9.9.9", &format!("user{i}"), i);
        }
        assert!(!l.sign_in_allowed("9.9.9.9", "fresh", 100));
        assert!(l.sign_in_allowed("8.8.8.8", "fresh", 100));
    }

    #[test]
    fn public_gets_and_writes_have_their_own_ceilings() {
        let l = Limiter::new();
        for _ in 0..120 {
            assert!(l.public_get_allowed("a", 0));
        }
        assert!(!l.public_get_allowed("a", 30));
        assert!(l.public_get_allowed("a", 61));
        for _ in 0..30 {
            assert!(l.write_allowed("s", 0));
        }
        assert!(!l.write_allowed("s", 59));
        assert!(l.write_allowed("t", 59));
    }

    #[test]
    fn sweeping_forgets_quiet_keys() {
        let l = Limiter::new();
        l.sign_in_failed("a", "b", 0);
        l.sweep(10_000);
        assert_eq!(l.failures.count("addr:a", 3600, 10_000), 0);
    }
}
