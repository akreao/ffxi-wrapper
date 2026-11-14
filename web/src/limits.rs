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

    /// Atomically: when every `(key, limit, window)` is below its limit, records an event on
    /// each and returns true; otherwise records nothing. Concurrent callers can't both slip in
    /// under the same limit.
    pub fn reserve(&self, checks: &[(&str, usize, i64)], now: i64) -> bool {
        let mut m = self.hits.lock().unwrap();
        for (key, limit, window) in checks {
            if let Some(q) = m.get_mut(*key) {
                while q.front().is_some_and(|t| *t <= now - window) {
                    q.pop_front();
                }
                if q.len() >= *limit {
                    return false;
                }
            }
        }
        for (key, _, _) in checks {
            m.entry((*key).to_string()).or_default().push_back(now);
        }
        true
    }

    /// Takes back the newest event of a key (a reservation that turned out fine).
    pub fn release(&self, key: &str) {
        if let Some(q) = self.hits.lock().unwrap().get_mut(key) {
            q.pop_back();
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
const DAY: i64 = 24 * HOUR;

/// Sign-up: 3 an hour from one address, 30 a day for the whole server. Behind the proxy
/// address nobody can be told apart, so those attempts share one bucket.
pub const SIGNUP_ADDR: (usize, i64) = (3, HOUR);
pub const SIGNUP_PROXIED: (usize, i64) = (10, HOUR);
pub const SIGNUP_DAY: (usize, i64) = (30, DAY);
/// Every attempt that gets as far as the invite check, from anywhere: bounds how fast codes can be guessed.
pub const SIGNUP_ATTEMPTS: (usize, i64) = (60, HOUR);
const SIGNUP_ATTEMPTS_KEY: &str = "signup:attempts";
const SIGNUP_DAY_KEY: &str = "signup:day";

#[derive(Debug, PartialEq, Eq)]
pub enum SignupLimit {
    Address,
    Day,
    Attempts,
}

fn signup_addr_key(addr: &str) -> (String, usize) {
    // An IPv6 customer gets a whole /64: one machine has billions of addresses in it.
    if let Ok(std::net::IpAddr::V6(v6)) = addr.parse::<std::net::IpAddr>() {
        if !v6.is_loopback() {
            let s = v6.segments();
            return (format!("signup:{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3]), SIGNUP_ADDR.0);
        }
    }
    if is_proxy_addr(addr) {
        ("signup:proxied".to_string(), SIGNUP_PROXIED.0)
    } else {
        (format!("signup:{addr}"), SIGNUP_ADDR.0)
    }
}

/// What sign-in is limited by (see the spec's rate-limit table).
pub const FAIL_PAIR: (usize, i64) = (5, 15 * MIN);
pub const FAIL_LOGIN: (usize, i64) = (20, HOUR);
pub const FAIL_ADDR: (usize, i64) = (30, HOUR);
/// Behind the proxy address nobody can be told apart, so logins that match no account share one
/// high ceiling (a script making up logins hits it; people mistyping a name don't).
pub const FAIL_UNKNOWN_PROXIED: (usize, i64) = (300, HOUR);

/// Who a sign-in attempt is aimed at: the account when the login matched one (the database
/// matches case-, accent- and trailing-space-insensitively, so every spelling of a login is the
/// same account), otherwise the typed login, folded and cut short so keys stay small.
#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    Account(i64),
    Unknown(String),
}

impl Target {
    pub fn unknown(typed: &str) -> Target {
        Target::Unknown(typed.trim_end().to_lowercase().chars().take(32).collect())
    }
    fn key(&self) -> String {
        match self {
            Target::Account(id) => format!("acc:{id}"),
            Target::Unknown(l) => format!("login:{l}"),
        }
    }
}

/// An address that says nothing about who is asking: Docker's userland proxy makes every
/// connection look like it comes from the bridge gateway (an address ending in .1), and a reverse
/// proxy or Tailscale Serve from loopback. The per-address limit would lock everyone out together.
pub fn is_proxy_addr(ip: &str) -> bool {
    match ip.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => v4.is_loopback() || (v4.is_private() && v4.octets()[3] == 1),
        Ok(std::net::IpAddr::V6(v6)) => v6.is_loopback(),
        Err(_) => true,
    }
}

/// The three sign-in buckets of one attempt (the address one only for a real client address).
pub struct SignInKeys {
    pair: String,
    login: String,
    addr: Option<(String, usize, i64)>,
}

pub struct Limiter {
    pub failures: Windows,
    pub public_gets: Windows,
    pub writes: Windows,
    pub signups: Windows,
}

impl Limiter {
    pub fn new() -> Limiter {
        Limiter { failures: Windows::default(), public_gets: Windows::default(), writes: Windows::default(), signups: Windows::default() }
    }

    pub fn sign_in_keys(addr: &str, target: &Target) -> SignInKeys {
        let t = target.key();
        SignInKeys {
            pair: format!("pair:{addr}|{t}"),
            login: t,
            addr: if !is_proxy_addr(addr) {
                Some((format!("addr:{addr}"), FAIL_ADDR.0, FAIL_ADDR.1))
            } else if matches!(target, Target::Unknown(_)) {
                Some(("proxied-unknown".to_string(), FAIL_UNKNOWN_PROXIED.0, FAIL_UNKNOWN_PROXIED.1))
            } else {
                None
            },
        }
    }

    /// Counts an attempt on every bucket, all at once, before the password is checked; false
    /// (and nothing counted) when any bucket is full. Call [`Limiter::sign_in_succeeded`] to
    /// take the count back when the password was right.
    pub fn reserve_sign_in(&self, k: &SignInKeys, now: i64) -> bool {
        let mut checks = vec![(k.pair.as_str(), FAIL_PAIR.0, FAIL_PAIR.1), (k.login.as_str(), FAIL_LOGIN.0, FAIL_LOGIN.1)];
        if let Some((a, limit, window)) = &k.addr {
            checks.push((a.as_str(), *limit, *window));
        }
        self.failures.reserve(&checks, now)
    }

    pub fn sign_in_succeeded(&self, k: &SignInKeys) {
        self.failures.release(&k.pair);
        self.failures.release(&k.login);
        if let Some((a, _, _)) = &k.addr {
            self.failures.release(a);
        }
    }

    /// How many more attempts this pair may make before it is locked.
    pub fn pair_tries_left(&self, k: &SignInKeys, now: i64) -> usize {
        FAIL_PAIR.0.saturating_sub(self.failures.count(&k.pair, FAIL_PAIR.1, now))
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
        self.signups.sweep(DAY, now);
    }

    /// Counts a sign-up attempt against its address and against the day's total at once;
    /// false (nothing counted) when either is full. [`Limiter::signup_not_created`] takes the
    /// day's count back when no account came of it; the address keeps its count, so guessing
    /// at invite codes or names stays slow.
    pub fn reserve_signup(&self, addr: &str, now: i64) -> Result<(), SignupLimit> {
        let (addr_key, addr_limit) = signup_addr_key(addr);
        if self.signups.reserve(&[(addr_key.as_str(), addr_limit, HOUR), (SIGNUP_DAY_KEY, SIGNUP_DAY.0, DAY), (SIGNUP_ATTEMPTS_KEY, SIGNUP_ATTEMPTS.0, SIGNUP_ATTEMPTS.1)], now) {
            return Ok(());
        }
        Err(if self.signups.count(&addr_key, HOUR, now) >= addr_limit {
            SignupLimit::Address
        } else if self.signups.count(SIGNUP_DAY_KEY, DAY, now) >= SIGNUP_DAY.0 {
            SignupLimit::Day
        } else {
            SignupLimit::Attempts
        })
    }

    pub fn signup_not_created(&self) {
        self.signups.release(SIGNUP_DAY_KEY);
    }

    /// Takes back an attempt that did nothing at all (the site was busy): both counts.
    pub fn signup_undo(&self, addr: &str) {
        self.signups.release(&signup_addr_key(addr).0);
        self.signups.release(SIGNUP_ATTEMPTS_KEY);
        self.signup_not_created();
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

    fn attempt(l: &Limiter, addr: &str, t: &Target, now: i64) -> bool {
        l.reserve_sign_in(&Limiter::sign_in_keys(addr, t), now)
    }

    #[test]
    fn five_attempts_for_one_pair_lock_it_for_fifteen_minutes() {
        let l = Limiter::new();
        let t = Target::Account(7);
        for i in 0..5 {
            assert!(attempt(&l, "1.2.3.4", &t, i));
        }
        assert!(!attempt(&l, "1.2.3.4", &t, 10));
        // Another address, or another account, is unaffected.
        assert!(attempt(&l, "5.6.7.8", &t, 10));
        assert!(attempt(&l, "1.2.3.4", &Target::Account(8), 10));
        // The window ends 15 minutes after the attempts.
        assert!(!attempt(&l, "1.2.3.4", &t, 15 * 60 - 1));
        assert!(attempt(&l, "1.2.3.4", &t, 15 * 65));
    }

    #[test]
    fn a_refused_attempt_counts_nothing() {
        let l = Limiter::new();
        let t = Target::Account(1);
        for i in 0..5 {
            assert!(attempt(&l, "1.2.3.4", &t, i));
        }
        for _ in 0..50 {
            assert!(!attempt(&l, "1.2.3.4", &t, 20));
        }
        // Only the five real attempts are on the login and address buckets.
        assert_eq!(l.failures.count("acc:1", HOUR, 20), 5);
        assert_eq!(l.failures.count("addr:1.2.3.4", HOUR, 20), 5);
    }

    #[test]
    fn a_burst_of_simultaneous_attempts_gets_only_five_through() {
        let l = std::sync::Arc::new(Limiter::new());
        let handles: Vec<_> = (0..64)
            .map(|_| {
                let l = l.clone();
                std::thread::spawn(move || attempt(&l, "1.2.3.4", &Target::Account(7), 0))
            })
            .collect();
        let allowed = handles.into_iter().map(|h| h.join().unwrap()).filter(|ok| *ok).count();
        assert_eq!(allowed, 5);
    }

    #[test]
    fn a_right_password_gives_its_attempt_back() {
        let l = Limiter::new();
        let t = Target::Account(3);
        for _ in 0..4 {
            assert!(attempt(&l, "1.2.3.4", &t, 0));
        }
        let k = Limiter::sign_in_keys("1.2.3.4", &t);
        assert!(l.reserve_sign_in(&k, 1));
        l.sign_in_succeeded(&k);
        assert_eq!(l.pair_tries_left(&k, 2), 1);
        assert!(attempt(&l, "1.2.3.4", &t, 2));
        assert!(!attempt(&l, "1.2.3.4", &t, 3));
    }

    #[test]
    fn an_account_is_locked_after_twenty_attempts_from_any_addresses() {
        let l = Limiter::new();
        for i in 0..20 {
            assert!(attempt(&l, &format!("10.0.5.{}", i + 2), &Target::Account(9), i));
        }
        assert!(!attempt(&l, "10.9.9.9", &Target::Account(9), 100));
        assert!(attempt(&l, "10.9.9.9", &Target::Account(10), 100));
        assert!(attempt(&l, "10.9.9.9", &Target::Account(9), 3601 + 20));
    }

    #[test]
    fn an_address_is_locked_after_thirty_attempts_on_any_logins() {
        let l = Limiter::new();
        for i in 0..30 {
            assert!(attempt(&l, "9.9.9.9", &Target::Account(100 + i), i));
        }
        assert!(!attempt(&l, "9.9.9.9", &Target::Account(1), 100));
        assert!(attempt(&l, "8.8.8.8", &Target::Account(1), 100));
    }

    #[test]
    fn the_proxys_address_gets_no_address_bucket() {
        // Every visitor looks like the Docker gateway: strangers must not be able to lock
        // everyone out together. The account's own buckets still apply.
        for gw in ["172.19.0.1", "192.168.64.1", "10.0.0.1", "127.0.0.1", "::1", "unknown"] {
            assert!(is_proxy_addr(gw), "{gw}");
            let l = Limiter::new();
            for i in 0..100 {
                assert!(attempt(&l, gw, &Target::Account(1000 + i), i), "{gw} {i}");
            }
            // Made-up logins share one high ceiling instead, so a script can't fan out for free.
            let l = Limiter::new();
            let allowed = (0..400).filter(|i| attempt(&l, gw, &Target::unknown(&format!("fake{i}")), *i)).count();
            assert_eq!(allowed, FAIL_UNKNOWN_PROXIED.0, "{gw}");
            // Real accounts are not held up by it.
            assert!(attempt(&l, gw, &Target::Account(5), 401));
        }
        assert!(!is_proxy_addr("192.168.0.50") && !is_proxy_addr("203.0.113.9") && !is_proxy_addr("172.19.0.2"));
    }

    #[test]
    fn unknown_logins_are_folded_and_kept_short() {
        assert_eq!(Target::unknown("Claude "), Target::unknown("claude"));
        let long = "x".repeat(10_000);
        let Target::Unknown(k) = Target::unknown(&long) else { panic!() };
        assert_eq!(k.len(), 32);
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
        attempt(&l, "1.2.3.4", &Target::Account(1), 0);
        l.sweep(10_000);
        assert_eq!(l.failures.count("addr:1.2.3.4", 3600, 10_000), 0);
    }

    #[test]
    fn signup_keys_group_an_ipv6_network_and_keep_ipv4_apart() {
        assert_eq!(signup_addr_key("2001:db8:1:2:aaaa::1").0, signup_addr_key("2001:db8:1:2:bbbb::9").0);
        assert_ne!(signup_addr_key("2001:db8:1:2::1").0, signup_addr_key("2001:db8:1:3::1").0);
        assert_ne!(signup_addr_key("198.51.100.1").0, signup_addr_key("198.51.100.2").0);
    }

    #[test]
    fn sixty_signup_attempts_an_hour_from_anywhere() {
        let l = Limiter::new();
        for i in 0..60 {
            assert_eq!(l.reserve_signup(&format!("198.51.{}.{}", i / 200, i % 200 + 2), 0), Ok(()), "{i}");
            l.signup_not_created(); // refused tries give the day's slot back; the attempt stays counted
        }
        assert_eq!(l.reserve_signup("203.0.113.5", 0), Err(SignupLimit::Attempts));
        assert_eq!(l.reserve_signup("203.0.113.5", 3601), Ok(()));
    }
}
