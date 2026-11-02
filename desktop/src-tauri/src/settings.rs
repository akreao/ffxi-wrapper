//! Game settings changed from the app: a few of LSB's settings, kept in
//! `overrides/settings/app-settings.json` and written out as
//! `overrides/settings/zz_app.lua`, which LSB loads after every other
//! settings file there, so these win over the same keys in a hand-written
//! `map.lua` or `main.lua`. Only values that differ from LSB's defaults are
//! written.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

#[derive(Serialize, Clone, Copy)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Kind {
    /// A number from `min` to `max`; `whole` ones take no fraction.
    Number { min: f64, max: f64, whole: bool },
    /// LSB's `true`/`false`.
    Bool,
    /// LSB's `1`/`0` on and off.
    Switch,
    /// One of these numbers, with what each means.
    Choice { options: &'static [(i64, &'static str)] },
}

#[derive(Serialize, Clone, Copy)]
pub struct Setting {
    /// The settings file it's in: `main` or `map`.
    pub file: &'static str,
    pub key: &'static str,
    pub group: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    #[serde(flatten)]
    pub kind: Kind,
    /// LSB's default (`settings/default/<file>.lua`).
    pub default: f64,
}

const RATES: &str = "Rates";
const NEW: &str = "New characters";
const WORLD: &str = "World";

const fn rate(file: &'static str, key: &'static str, label: &'static str, hint: &'static str) -> Setting {
    Setting { file, key, group: RATES, label, hint, kind: Kind::Number { min: 0.0, max: 100.0, whole: false }, default: 1.0 }
}

pub const SETTINGS: &[Setting] = &[
    rate("map", "EXP_RATE", "Experience from monsters", "2 doubles it."),
    rate("main", "EXP_RATE", "Experience from quests", ""),
    rate("map", "CAPACITY_RATE", "Capacity points", ""),
    rate("map", "EXP_LOSS_RATE", "Experience lost on death", "0 loses none."),
    rate("map", "DROP_RATE_MULTIPLIER", "Item drop rate", ""),
    rate("map", "MOB_GIL_MULTIPLIER", "Gil from monsters", ""),
    rate("main", "GIL_RATE", "Gil from quests", ""),
    rate("map", "SKILLUP_CHANCE_MULTIPLIER", "Combat skill-up chance", ""),
    rate("map", "CRAFT_CHANCE_MULTIPLIER", "Crafting skill-up chance", ""),
    rate("map", "CRAFT_HQ_CHANCE_MULTIPLIER", "Crafting high-quality chance", ""),
    Setting {
        file: "main",
        key: "INITIAL_LEVEL_CAP",
        group: NEW,
        label: "Starting level cap",
        hint: "Limit break quests raise it from here.",
        kind: Kind::Number { min: 1.0, max: 99.0, whole: true },
        default: 50.0,
    },
    Setting {
        file: "main",
        key: "MAX_LEVEL",
        group: NEW,
        label: "Highest level",
        hint: "",
        kind: Kind::Number { min: 1.0, max: 99.0, whole: true },
        default: 99.0,
    },
    Setting {
        file: "main",
        key: "START_GIL",
        group: NEW,
        label: "Starting gil",
        hint: "",
        kind: Kind::Number { min: 0.0, max: 999_999_999.0, whole: true },
        default: 10.0,
    },
    Setting {
        file: "main",
        key: "START_INVENTORY",
        group: NEW,
        label: "Starting inventory size",
        hint: "30 to 80.",
        kind: Kind::Number { min: 30.0, max: 80.0, whole: true },
        default: 30.0,
    },
    Setting {
        file: "main",
        key: "SUBJOB_QUEST_LEVEL",
        group: NEW,
        label: "Support job quest level",
        hint: "0 starts every character with support jobs unlocked.",
        kind: Kind::Number { min: 0.0, max: 99.0, whole: true },
        default: 18.0,
    },
    Setting {
        file: "main",
        key: "ALL_MAPS",
        group: NEW,
        label: "Start with every map",
        hint: "",
        kind: Kind::Switch,
        default: 0.0,
    },
    Setting {
        file: "main",
        key: "UNLOCK_OUTPOST_WARPS",
        group: NEW,
        label: "Outpost warps",
        hint: "",
        kind: Kind::Choice {
            options: &[(0, "Earned"), (1, "All"), (2, "All + Tu'Lia, Tavnazia")],
        },
        default: 0.0,
    },
    Setting {
        file: "map",
        key: "FISHING_ENABLE",
        group: WORLD,
        label: "Fishing",
        hint: "Off in LandSandBoat unless turned on.",
        kind: Kind::Bool,
        default: 0.0,
    },
    Setting {
        file: "main",
        key: "HOMEPOINT_TELEPORT",
        group: WORLD,
        label: "Home point teleports",
        hint: "",
        kind: Kind::Switch,
        default: 1.0,
    },
];

/// Values by `file.KEY`, as numbers (`1`/`0` for on and off).
pub type Values = BTreeMap<String, f64>;

fn id(s: &Setting) -> String {
    format!("{}.{}", s.file, s.key)
}

fn json_path(stack: &Path) -> PathBuf {
    stack.join("overrides").join("settings").join("app-settings.json")
}

pub fn lua_path(stack: &Path) -> PathBuf {
    stack.join("overrides").join("settings").join("zz_app.lua")
}

/// The saved values, with LSB's default for each one not set.
pub fn load(stack: &Path) -> Values {
    let saved: BTreeMap<String, Value> = fs::read(json_path(stack))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    SETTINGS
        .iter()
        .map(|s| {
            let value = saved.get(&id(s)).and_then(Value::as_f64).filter(|v| check(s, *v).is_ok());
            (id(s), value.unwrap_or(s.default))
        })
        .collect()
}

fn check(s: &Setting, v: f64) -> Result<(), String> {
    let ok = v.is_finite()
        && match s.kind {
            Kind::Number { min, max, whole } => v >= min && v <= max && (!whole || v.fract() == 0.0),
            Kind::Bool | Kind::Switch => v == 0.0 || v == 1.0,
            Kind::Choice { options } => options.iter().any(|(o, _)| *o as f64 == v),
        };
    if ok {
        return Ok(());
    }
    Err(match s.kind {
        Kind::Number { min, max, whole: true } => format!("{} must be a whole number from {min} to {max}.", s.label),
        Kind::Number { min, max, .. } => format!("{} must be from {min} to {max}.", s.label),
        _ => format!("{} has a value it can't take.", s.label),
    })
}

/// Saves `values` (missing ones keep their saved value) and writes the
/// Lua file. Returns whether anything changed.
pub fn save(stack: &Path, values: &Values) -> Result<bool, String> {
    let mut all = load(stack);
    for s in SETTINGS {
        if let Some(v) = values.get(&id(s)) {
            check(s, *v)?;
            all.insert(id(s), *v);
        }
    }
    if all.get("main.INITIAL_LEVEL_CAP") > all.get("main.MAX_LEVEL") {
        return Err("The starting level cap can't be above the highest level.".into());
    }
    let changed: Values = SETTINGS
        .iter()
        .filter(|s| all[&id(s)] != s.default)
        .map(|s| (id(s), all[&id(s)]))
        .collect();
    let lua = lua(&changed);
    let before = fs::read_to_string(lua_path(stack)).ok();
    let dir = json_path(stack).parent().unwrap().to_owned();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_vec_pretty(&changed).map_err(|e| e.to_string())?;
    fs::write(json_path(stack), json).map_err(|e| e.to_string())?;
    if changed.is_empty() {
        let _ = fs::remove_file(lua_path(stack));
    } else {
        fs::write(lua_path(stack), &lua).map_err(|e| e.to_string())?;
    }
    let after = (!changed.is_empty()).then_some(lua);
    Ok(before != after)
}

fn lua(changed: &Values) -> String {
    let mut out = String::from(
        "-- Written by the ffxi-wrapper desktop app from its Game settings, and\n\
         -- replaced when they're saved: change these there. LSB loads this file\n\
         -- after the other settings files here, so its values win.\n\
         xi = xi or {}\n\
         xi.settings = xi.settings or {}\n",
    );
    let mut files: Vec<&str> = SETTINGS.iter().map(|s| s.file).collect();
    files.dedup();
    files.sort();
    files.dedup();
    for file in files {
        let mine: Vec<&Setting> = SETTINGS.iter().filter(|s| s.file == file && changed.contains_key(&id(s))).collect();
        if mine.is_empty() {
            continue;
        }
        out.push_str(&format!("xi.settings.{file} = xi.settings.{file} or {{}}\n"));
        for s in mine {
            let v = changed[&id(s)];
            let value = match s.kind {
                Kind::Bool => (if v == 1.0 { "true" } else { "false" }).to_owned(),
                _ => v.to_string(),
            };
            out.push_str(&format!("xi.settings.{file}.{} = {value}\n", s.key));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("settings-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writes_only_what_differs_from_lsb() {
        let stack = temp("write");
        assert_eq!(load(&stack)["map.EXP_RATE"], 1.0);
        let mut v = Values::new();
        v.insert("map.EXP_RATE".into(), 2.5);
        v.insert("map.FISHING_ENABLE".into(), 1.0);
        v.insert("main.START_GIL".into(), 10.0);
        v.insert("main.ALL_MAPS".into(), 1.0);
        assert!(save(&stack, &v).unwrap());
        let lua = fs::read_to_string(lua_path(&stack)).unwrap();
        assert!(lua.contains("xi.settings.map.EXP_RATE = 2.5\n"));
        assert!(lua.contains("xi.settings.map.FISHING_ENABLE = true\n"));
        assert!(lua.contains("xi.settings.main.ALL_MAPS = 1\n"));
        assert!(!lua.contains("START_GIL"));
        assert!(lua.find("xi.settings.main =").unwrap() < lua.find("xi.settings.map =").unwrap());
        assert_eq!(load(&stack)["map.EXP_RATE"], 2.5);
        // Saving the same again changes nothing.
        assert!(!save(&stack, &v).unwrap());

        // Back to the defaults: the file goes.
        let mut back = Values::new();
        back.insert("map.EXP_RATE".into(), 1.0);
        back.insert("map.FISHING_ENABLE".into(), 0.0);
        back.insert("main.ALL_MAPS".into(), 0.0);
        assert!(save(&stack, &back).unwrap());
        assert!(!lua_path(&stack).exists());
        fs::remove_dir_all(stack).unwrap();
    }

    #[test]
    fn refuses_values_out_of_range() {
        let stack = temp("range");
        let one = |k: &str, v: f64| Values::from([(k.to_owned(), v)]);
        assert!(save(&stack, &one("main.START_INVENTORY", 81.0)).is_err());
        assert!(save(&stack, &one("main.MAX_LEVEL", 75.5)).is_err());
        assert!(save(&stack, &one("map.EXP_RATE", -1.0)).is_err());
        assert!(save(&stack, &one("main.UNLOCK_OUTPOST_WARPS", 3.0)).is_err());
        assert!(save(&stack, &one("map.FISHING_ENABLE", 2.0)).is_err());
        assert!(save(&stack, &one("main.MAX_LEVEL", 40.0)).unwrap_err().contains("level cap"));
        assert!(save(&stack, &one("main.UNLOCK_OUTPOST_WARPS", 2.0)).is_ok());
        fs::remove_dir_all(stack).unwrap();
    }

    #[test]
    fn every_setting_has_a_valid_default() {
        for s in SETTINGS {
            assert!(check(s, s.default).is_ok(), "{}", id(s));
        }
    }
}
