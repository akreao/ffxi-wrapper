//! The website (compose.yaml's `web` service): public pages, players' own
//! accounts and the admin panel. It runs only while `.env`'s
//! `COMPOSE_PROFILES` names it. The app turns it on and off, picks who can
//! reach it, and signs the owner in with a one-minute link.
//!
//! `.env` lines: `WEB_BIND_IP` (empty: this computer only), `WEB_PORT`
//! (default 54080) and `ADMIN_SECRET`, the 64 hex characters that sign
//! owner links (see `wrapper_core::owner_link`).

use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::envfile::{self, EnvFile};
use crate::net;

pub const SERVICE: &str = "web";
const PROFILE: &str = "web";
pub const DEFAULT_PORT: u16 = 54080;

/// What the screen lets the user choose.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Choice {
    pub enabled: bool,
    /// An address of this computer the site listens on, or empty for this
    /// computer only.
    pub bind_ip: String,
}

/// The saved choice and what follows from it.
#[derive(Serialize, Debug, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    pub bind_ip: String,
    pub port: u16,
    /// Where the site is: the address a browser on this computer opens.
    pub url: String,
    /// `ADMIN_SECRET` is a usable key, so the app can sign the owner in.
    pub owner_link_ready: bool,
}

pub fn load(env: &EnvFile) -> Settings {
    let get = |key: &str| env.get(key).filter(|v| !v.is_empty());
    let bind_ip = get("WEB_BIND_IP").filter(|ip| ip.parse::<Ipv4Addr>().is_ok()).unwrap_or_default();
    let port = get("WEB_PORT").and_then(|p| p.parse().ok()).filter(|p| *p > 0).unwrap_or(DEFAULT_PORT);
    let host = if bind_ip.is_empty() { "127.0.0.1" } else { bind_ip.as_str() };
    Settings {
        enabled: envfile::profiles(env).iter().any(|p| p == PROFILE),
        url: format!("http://{host}:{port}"),
        owner_link_ready: wrapper_core::owner_link::parse_secret(&env.get("ADMIN_SECRET").unwrap_or_default()).is_ok(),
        bind_ip,
        port,
    }
}

/// Checks `new` and writes it into `env`, making the owner-link key when the
/// site is turned on without one. Returns whether anything changed.
pub fn save(env: &mut EnvFile, new: &Choice) -> Result<bool, String> {
    let bind = bind_ip(&new.bind_ip)?;
    let before = load(env);

    envfile::set_profile(env, PROFILE, new.enabled);
    env.set("WEB_BIND_IP", &bind);
    if new.enabled && !before.owner_link_ready {
        env.set("ADMIN_SECRET", &new_secret()?);
    }
    Ok(load(env) != before)
}

/// The address to store for what the screen sent: loopback and empty both
/// mean this computer only.
fn bind_ip(typed: &str) -> Result<String, String> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Ok(String::new());
    }
    net::check_address(typed)?;
    let ip: Ipv4Addr = typed.parse().map_err(|_| format!("\"{typed}\" isn't an IPv4 address."))?;
    Ok(if ip.is_loopback() { String::new() } else { ip.to_string() })
}

/// 64 hex characters from the system's random source.
fn new_secret() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// The link that signs the owner in to the running site: its address and a
/// token good for a minute, once.
pub fn owner_url(env: &EnvFile, now: u64) -> Result<String, String> {
    let settings = load(env);
    if !settings.enabled {
        return Err("Turn the website on first.".into());
    }
    let secret = env.get("ADMIN_SECRET").unwrap_or_default();
    let token = wrapper_core::owner_link::sign(&secret, now)
        .map_err(|_| "The website has no owner key yet. Save the settings once more, then restart the website.".to_owned())?;
    Ok(format!("{}/auth/owner?t={token}", settings.url))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    fn choice(enabled: bool, bind_ip: &str) -> Choice {
        Choice { enabled, bind_ip: bind_ip.into() }
    }

    #[test]
    fn missing_lines_mean_off_on_this_computer_with_the_default_port() {
        let s = load(&EnvFile::parse("DB_PASSWORD=x\n"));
        assert!(!s.enabled);
        assert_eq!((s.bind_ip.as_str(), s.port, s.url.as_str()), ("", 54080, "http://127.0.0.1:54080"));
        assert!(!s.owner_link_ready);
    }

    #[test]
    fn reads_the_address_and_port() {
        let s = load(&EnvFile::parse("COMPOSE_PROFILES=ahbot,web\nWEB_BIND_IP=192.168.1.20\nWEB_PORT=8081\n"));
        assert!(s.enabled);
        assert_eq!(s.url, "http://192.168.1.20:8081");
        // A value Compose would refuse as a bind address is read as none.
        let junk = load(&EnvFile::parse("WEB_BIND_IP=lan\nWEB_PORT=0\n"));
        assert_eq!((junk.bind_ip.as_str(), junk.port), ("", 54080));
    }

    #[test]
    fn turning_it_on_makes_a_key_and_keeps_other_profiles() {
        let mut env = EnvFile::parse("COMPOSE_PROFILES=ahbot\n");
        assert!(save(&mut env, &choice(true, "")).unwrap());
        assert_eq!(env.get("COMPOSE_PROFILES").as_deref(), Some("ahbot,web"));
        let key = env.get("ADMIN_SECRET").unwrap();
        assert_eq!(key.len(), 64);
        assert!(load(&env).owner_link_ready);
        // Saving the same choice again changes nothing, and keeps the key.
        assert!(!save(&mut env, &choice(true, "")).unwrap());
        assert_eq!(env.get("ADMIN_SECRET").unwrap(), key);
        // Off again: the key stays for the next time.
        assert!(save(&mut env, &choice(false, "")).unwrap());
        assert_eq!(env.get("COMPOSE_PROFILES").as_deref(), Some("ahbot"));
        assert_eq!(env.get("ADMIN_SECRET").unwrap(), key);
    }

    #[test]
    fn a_key_the_user_set_stays_and_a_broken_one_is_replaced() {
        let mut env = EnvFile::parse(&format!("ADMIN_SECRET={KEY}\n"));
        save(&mut env, &choice(true, "")).unwrap();
        assert_eq!(env.get("ADMIN_SECRET").as_deref(), Some(KEY));
        let mut broken = EnvFile::parse("ADMIN_SECRET=oops\n");
        save(&mut broken, &choice(true, "")).unwrap();
        assert!(load(&broken).owner_link_ready);
        // Off with no key: none is made.
        let mut off = EnvFile::parse("");
        save(&mut off, &choice(false, "")).unwrap();
        assert_eq!(off.get("ADMIN_SECRET"), None);
    }

    #[test]
    fn who_can_reach_it() {
        let mut env = EnvFile::parse("");
        save(&mut env, &choice(true, " 100.101.102.103 ")).unwrap();
        assert_eq!(load(&env).url, "http://100.101.102.103:54080");
        // Loopback is the same as this computer only.
        save(&mut env, &choice(true, "127.0.0.1")).unwrap();
        assert_eq!(env.get("WEB_BIND_IP").as_deref(), Some(""));
        for bad in ["0.0.0.0", "everyone", "192.168.1", "1.2.3.4\nX=1"] {
            assert!(save(&mut env, &choice(true, bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_owner_link_is_the_sites_address_with_a_token_that_verifies() {
        let mut env = EnvFile::parse(&format!("COMPOSE_PROFILES=web\nADMIN_SECRET={KEY}\nWEB_BIND_IP=192.168.1.20\n"));
        let url = owner_url(&env, 1_800_000_000).unwrap();
        let token = url.strip_prefix("http://192.168.1.20:54080/auth/owner?t=").expect(&url);
        assert!(!token.contains(['&', '#', ' ']));
        assert!(wrapper_core::owner_link::verify(KEY, token, 1_800_000_030).is_ok());
        assert!(wrapper_core::owner_link::verify(KEY, token, 1_800_000_100).is_err());

        env.set("COMPOSE_PROFILES", "ahbot");
        assert!(owner_url(&env, 1_800_000_000).is_err());
        let keyless = EnvFile::parse("COMPOSE_PROFILES=web\n");
        assert!(owner_url(&keyless, 1_800_000_000).unwrap_err().contains("owner key"));
    }
}
