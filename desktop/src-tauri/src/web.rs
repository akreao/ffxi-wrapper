//! The website (compose.yaml's `web` service): public pages, players' own
//! accounts and the admin panel. It runs only while `.env`'s
//! `COMPOSE_PROFILES` names it. The app turns it on and off, picks who can
//! reach it, and signs the owner in with a one-minute link.
//!
//! `.env` lines: `WEB_DB_PASSWORD` (the site's own database user's password,
//! made when the site is turned on without one), `WEB_BIND_IP` (empty: this computer only), `WEB_PORT`
//! (default 54080), `WEB_LOCAL_PORT` (default 54081, the site on 127.0.0.1
//! whatever `WEB_BIND_IP` says, where the owner link opens) and `ADMIN_SECRET`, the 64 hex characters that sign
//! owner links (see `wrapper_core::owner_link`).

use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::envfile::{self, EnvFile};
use crate::net;

pub const SERVICE: &str = "web";
const PROFILE: &str = "web";
/// The profiles that only make sense with the site on: compose.yaml's Caddy
/// proxy (`public`, service `proxy`) and its DNS updater (`ddns`, service `ddns`).
pub const COMPANIONS: [&str; 2] = ["public", "ddns"];
pub const DEFAULT_PORT: u16 = 54080;
pub const DEFAULT_LOCAL_PORT: u16 = 54081;

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
    let port = port(env, "WEB_PORT", DEFAULT_PORT);
    let host = if bind_ip.is_empty() { "127.0.0.1" } else { bind_ip.as_str() };
    Settings {
        enabled: envfile::profiles(env).iter().any(|p| p == PROFILE),
        url: format!("http://{host}:{port}"),
        owner_link_ready: wrapper_core::owner_link::parse_secret(&env.get("ADMIN_SECRET").unwrap_or_default()).is_ok(),
        bind_ip,
        port,
    }
}

fn port(env: &EnvFile, key: &str, default: u16) -> u16 {
    env.get(key).and_then(|p| p.parse().ok()).filter(|p| *p > 0).unwrap_or(default)
}

/// The site's port on 127.0.0.1, published next to `WEB_PORT`.
pub fn local_port(env: &EnvFile) -> u16 {
    port(env, "WEB_LOCAL_PORT", DEFAULT_LOCAL_PORT)
}

/// The compose services `web-apply` starts or removes with the site: `web`,
/// and the proxy and DNS updater while their profiles are on. Turning the
/// site off takes those profiles out (see `save`), so then it's only `web`
/// here and the caller removes the companions by name.
pub fn services(env: &EnvFile) -> Vec<&'static str> {
    let on = envfile::profiles(env);
    let mut list = vec![SERVICE];
    if on.iter().any(|p| p == "public") {
        list.push("proxy");
    }
    if on.iter().any(|p| p == "ddns") {
        list.push("ddns");
    }
    list
}

/// Checks `new` and writes it into `env`, making the owner-link key when the
/// site is turned on without one. Returns whether anything changed.
pub fn save(env: &mut EnvFile, new: &Choice) -> Result<bool, String> {
    let bind = bind_ip(&new.bind_ip)?;
    let before = load(env);
    let profiles_before = envfile::profiles(env);

    envfile::set_profile(env, PROFILE, new.enabled);
    // Without the site, Caddy would answer on 80 and 443 with nothing behind it.
    if !new.enabled {
        for p in COMPANIONS {
            envfile::set_profile(env, p, false);
        }
    }
    env.set("WEB_BIND_IP", &bind);
    if new.enabled && !before.owner_link_ready {
        env.set("ADMIN_SECRET", &new_secret()?);
    }
    let made_password = if new.enabled { ensure_db_password(env)? } else { false };
    // Taking out public or ddns is a change too: the restart removes their containers.
    Ok(made_password || load(env) != before || envfile::profiles(env) != profiles_before)
}

/// Clears a `WEB_PORT` or `WEB_LOCAL_PORT` that isn't a port from 1 to 65535,
/// so compose.yaml's default applies: Compose refuses the whole file over a
/// bad one, and 0 would publish the site on a random port. Returns whether it
/// cleared any. The app does this at startup.
pub fn fix_ports(env: &mut EnvFile) -> Result<bool, String> {
    let mut fixed = false;
    for key in ["WEB_PORT", "WEB_LOCAL_PORT"] {
        let value = env.get(key).unwrap_or_default();
        if !value.is_empty() && !value.parse::<u16>().is_ok_and(|p| p > 0) {
            env.set(key, "");
            fixed = true;
        }
    }
    Ok(fixed)
}

/// Gives the site's own database user (made by db-init) a password in `.env`
/// when there's none, which compose.yaml reads for every service. Returns
/// whether it made one. The app does this at startup for existing installs.
pub fn ensure_db_password(env: &mut EnvFile) -> Result<bool, String> {
    if env.get("WEB_DB_PASSWORD").is_some_and(|p| !p.is_empty() && p != "change-me-web") {
        return Ok(false);
    }
    env.set("WEB_DB_PASSWORD", &envfile::random_password(24)?);
    Ok(true)
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

/// The admin panel's character list, game masters only.
pub const GAME_MASTERS_PAGE: &str = "/admin/characters?gm=1";

/// The link that signs the owner in to the running site: its 127.0.0.1
/// address (`WEB_LOCAL_PORT`) and a token good for a minute, once. Never the
/// `WEB_BIND_IP` address: on a LAN that is plain HTTP, and the token and then
/// the owner's session cookie would cross it unencrypted. It lands on `next`
/// (an admin panel path) instead of `/admin` when given; the site ignores any other path.
pub fn owner_url(env: &EnvFile, now: u64, next: Option<&str>) -> Result<String, String> {
    if !load(env).enabled {
        return Err("Turn the website on first.".into());
    }
    let secret = env.get("ADMIN_SECRET").unwrap_or_default();
    let token = wrapper_core::owner_link::sign(&secret, now)
        .map_err(|_| "The website has no owner key yet. Save the settings once more, then restart the website.".to_owned())?;
    let next = next.map(|n| {
        let encoded: String =
            n.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
        format!("&next={encoded}")
    });
    Ok(format!("http://127.0.0.1:{}/auth/owner?t={token}{}", local_port(env), next.unwrap_or_default()))
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
        let db = env.get("WEB_DB_PASSWORD").unwrap();
        assert_eq!(db.len(), 24);
        // Saving the same choice again changes nothing, and keeps both.
        assert!(!save(&mut env, &choice(true, "")).unwrap());
        assert_eq!(env.get("ADMIN_SECRET").unwrap(), key);
        assert_eq!(env.get("WEB_DB_PASSWORD").unwrap(), db);
        // Off again: the key stays for the next time.
        assert!(save(&mut env, &choice(false, "")).unwrap());
        assert_eq!(env.get("COMPOSE_PROFILES").as_deref(), Some("ahbot"));
        assert_eq!(env.get("ADMIN_SECRET").unwrap(), key);
    }

    #[test]
    fn an_existing_env_gets_a_database_password_once() {
        let mut env = EnvFile::parse("DB_PASSWORD=x\nCOMPOSE_PROFILES=\n");
        assert!(ensure_db_password(&mut env).unwrap());
        let made = env.get("WEB_DB_PASSWORD").unwrap();
        assert_eq!(made.len(), 24);
        assert_ne!(Some(made.clone()), env.get("DB_PASSWORD"));
        assert!(!ensure_db_password(&mut env).unwrap());
        assert_eq!(env.get("WEB_DB_PASSWORD").unwrap(), made);
        // Nothing else changes, and the site stays off.
        assert!(!load(&env).enabled);
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
        assert_eq!(off.get("WEB_DB_PASSWORD"), None);
        // The template's placeholder and an empty value are not passwords.
        for old in ["WEB_DB_PASSWORD=change-me-web", "WEB_DB_PASSWORD="] {
            let mut env = EnvFile::parse(old);
            assert!(save(&mut env, &choice(true, "")).unwrap());
            assert_eq!(env.get("WEB_DB_PASSWORD").unwrap().len(), 24);
        }
        let mut own = EnvFile::parse("WEB_DB_PASSWORD=mine");
        save(&mut own, &choice(true, "")).unwrap();
        assert_eq!(own.get("WEB_DB_PASSWORD").as_deref(), Some("mine"));
    }

    #[test]
    fn turning_it_off_also_turns_off_the_proxy_and_dns_updater() {
        let mut env = EnvFile::parse("COMPOSE_PROFILES=ahbot,web,public,ddns\n");
        assert_eq!(services(&env), ["web", "proxy", "ddns"]);
        assert!(save(&mut env, &choice(false, "")).unwrap());
        assert_eq!(env.get("COMPOSE_PROFILES").as_deref(), Some("ahbot"));
        assert_eq!(services(&env), ["web"]);
        // Already off, with the proxy's profile left on by hand: taking it out is a change.
        let mut stray = EnvFile::parse("COMPOSE_PROFILES=public\n");
        assert!(save(&mut stray, &choice(false, "")).unwrap());
        assert_eq!(stray.get("COMPOSE_PROFILES").as_deref(), Some(""));
        assert!(!save(&mut stray, &choice(false, "")).unwrap());
        // Turning it on doesn't bring them back: going public is a choice of its own.
        save(&mut env, &choice(true, "")).unwrap();
        assert_eq!(env.get("COMPOSE_PROFILES").as_deref(), Some("ahbot,web"));
        let mut public = EnvFile::parse("COMPOSE_PROFILES=web,public\n");
        save(&mut public, &choice(true, "100.101.102.103")).unwrap();
        assert_eq!(services(&public), ["web", "proxy"]);
    }

    #[test]
    fn a_bad_port_is_cleared_so_the_default_applies() {
        for bad in ["0", "abc", "70000", "-1"] {
            let mut env = EnvFile::parse(&format!("WEB_PORT=8080\nWEB_LOCAL_PORT={bad}\n"));
            assert!(fix_ports(&mut env).unwrap(), "{bad}");
            assert_eq!(env.get("WEB_LOCAL_PORT").as_deref(), Some(""));
            assert_eq!(env.get("WEB_PORT").as_deref(), Some("8080"));
            assert_eq!(local_port(&env), DEFAULT_LOCAL_PORT);
        }
        let mut good = EnvFile::parse("WEB_PORT=\nWEB_LOCAL_PORT=65535\n");
        assert!(!fix_ports(&mut good).unwrap());
        assert!(!fix_ports(&mut EnvFile::parse("")).unwrap());
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
        let url = owner_url(&env, 1_800_000_000, None).unwrap();
        // This computer's own port, never the LAN address the site is opened to.
        let token = url.strip_prefix("http://127.0.0.1:54081/auth/owner?t=").expect(&url);
        assert!(!token.contains(['&', '#', ' ']));
        assert!(wrapper_core::owner_link::verify(KEY, token, 1_800_000_030).is_ok());
        assert!(wrapper_core::owner_link::verify(KEY, token, 1_800_000_100).is_err());

        env.set("WEB_LOCAL_PORT", "8443");
        assert!(owner_url(&env, 1_800_000_000, None).unwrap().starts_with("http://127.0.0.1:8443/auth/owner?t="));
        let deep = owner_url(&env, 1_800_000_000, Some(GAME_MASTERS_PAGE)).unwrap();
        assert!(deep.ends_with("&next=%2Fadmin%2Fcharacters%3Fgm%3D1"), "{deep}");

        env.set("COMPOSE_PROFILES", "ahbot");
        assert!(owner_url(&env, 1_800_000_000, None).is_err());
        let keyless = EnvFile::parse("COMPOSE_PROFILES=web\n");
        assert!(owner_url(&keyless, 1_800_000_000, None).unwrap_err().contains("owner key"));
    }
}
