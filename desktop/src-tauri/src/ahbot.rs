//! The auction house bot (ffxiahbot): compose.yaml's `ahbot` service, which
//! runs only while `.env`'s `COMPOSE_PROFILES` names it. Its settings are
//! the `AHBOT_*` lines in `.env`, which docker/ahbot/run.sh turns into the
//! bot's config.

use serde::{Deserialize, Serialize};

use crate::envfile::EnvFile;

pub const SERVICE: &str = "ahbot";
const PROFILE: &str = "ahbot";

/// The price lists `AHBOT_ITEMS` can pick.
const ITEM_LISTS: [&str; 3] = ["items75", "items", "custom"];

/// The bot's settings, with the defaults run.sh uses for missing ones.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Settings {
    pub enabled: bool,
    /// Shown as the seller (and buyer) at the auction house.
    pub name: String,
    /// Seconds between buying rounds.
    pub tick: u32,
    /// Seconds between restocks.
    pub restock: u32,
    pub buy: bool,
    pub sell: bool,
    /// One of [`ITEM_LISTS`].
    pub items: String,
}

pub fn load(env: &EnvFile) -> Settings {
    let get = |key: &str| env.get(key).filter(|v| !v.is_empty());
    let number = |key: &str, default: u32| get(key).and_then(|v| v.parse().ok()).filter(|n| *n > 0).unwrap_or(default);
    let yes = |key: &str| get(key).is_none_or(|v| v != "no");
    Settings {
        enabled: profiles(env).any(|p| p == PROFILE),
        name: get("AHBOT_NAME").unwrap_or_else(|| "M.H.M.U.".to_owned()),
        tick: number("AHBOT_TICK", 30),
        restock: number("AHBOT_RESTOCK", 3600),
        buy: yes("AHBOT_BUY"),
        sell: yes("AHBOT_SELL"),
        items: get("AHBOT_ITEMS").filter(|v| ITEM_LISTS.contains(&v.as_str())).unwrap_or_else(|| "items75".to_owned()),
    }
}

fn profiles(env: &EnvFile) -> impl Iterator<Item = String> {
    let list = env.get("COMPOSE_PROFILES").unwrap_or_default();
    list.split(',').map(|p| p.trim().to_owned()).filter(|p| !p.is_empty()).collect::<Vec<_>>().into_iter()
}

/// Checks `new` and writes it into `env`. Returns whether anything changed.
pub fn save(env: &mut EnvFile, new: &Settings) -> Result<bool, String> {
    let name = new.name.trim();
    check_name(name)?;
    if !(5..=3600).contains(&new.tick) {
        return Err("Look for player listings every 5 to 3600 seconds.".into());
    }
    if !(60..=7 * 24 * 3600).contains(&new.restock) {
        return Err("Restock every minute at most, and at least once a week.".into());
    }
    if !ITEM_LISTS.contains(&new.items.as_str()) {
        return Err(format!("Unknown price list {}.", new.items));
    }
    let before = load(env);

    let mut profiles: Vec<String> = profiles(env).filter(|p| p != PROFILE).collect();
    if new.enabled {
        profiles.push(PROFILE.to_owned());
    }
    env.set("COMPOSE_PROFILES", &profiles.join(","));
    let yes_no = |on: bool| if on { "yes" } else { "no" };
    env.set("AHBOT_NAME", name);
    env.set("AHBOT_TICK", &new.tick.to_string());
    env.set("AHBOT_RESTOCK", &new.restock.to_string());
    env.set("AHBOT_BUY", yes_no(new.buy));
    env.set("AHBOT_SELL", yes_no(new.sell));
    env.set("AHBOT_ITEMS", &new.items);
    Ok(load(env) != before)
}

/// LSB keeps seller and buyer names in 15 characters. Quotes and the like
/// are kept out, since the name goes through `.env`.
fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.chars().count() > 15 {
        return Err("The bot's name needs 1 to 15 characters.".into());
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || " .-_".contains(c)) {
        return Err("Use letters, digits, spaces, dots, dashes or underscores in the bot's name.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_lines_mean_off_with_the_defaults() {
        let s = load(&EnvFile::parse("DB_PASSWORD=x\n"));
        assert!(!s.enabled);
        assert_eq!((s.name.as_str(), s.tick, s.restock, s.buy, s.sell), ("M.H.M.U.", 30, 3600, true, true));
        assert_eq!(s.items, "items75");
    }

    #[test]
    fn saving_turns_the_profile_on_and_off_and_keeps_others() {
        let mut env = EnvFile::parse("COMPOSE_PROFILES=other\n");
        let mut s = load(&env);
        s.enabled = true;
        s.sell = false;
        s.items = "custom".into();
        assert!(save(&mut env, &s).unwrap());
        assert_eq!(env.get("COMPOSE_PROFILES").as_deref(), Some("other,ahbot"));
        assert_eq!(load(&env), s);
        assert!(!save(&mut env, &s).unwrap());
        s.enabled = false;
        assert!(save(&mut env, &s).unwrap());
        assert_eq!(env.get("COMPOSE_PROFILES").as_deref(), Some("other"));
    }

    #[test]
    fn refuses_bad_values() {
        let mut env = EnvFile::parse("");
        let good = load(&env);
        for bad in [
            Settings { name: "A name that is far too long".into(), ..good.clone() },
            Settings { name: "Bot\"; rm".into(), ..good.clone() },
            Settings { tick: 1, ..good.clone() },
            Settings { restock: 10, ..good.clone() },
            Settings { items: "mine".into(), ..good.clone() },
        ] {
            assert!(save(&mut env, &bad).is_err(), "{bad:?}");
        }
    }
}
