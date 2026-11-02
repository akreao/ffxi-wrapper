//! The game client's settings changed from the app: a few of vana-client's
//! options, edited in place in its `config.json` (the one `FFXI_CONFIG`
//! points it at, in `client/` in the server folder). Every other key the
//! client keeps there (logins, window places, key bindings) is left alone.
//!
//! Each setting's `file` says where it lives: `cfg` is a key of the config
//! itself, `tool` one of the "FINAL FANTASY XI Config" tool's values under
//! `tool` (a number, 0 or 1 for a switch). Defaults are vana-client's
//! (`config.rs`: `Config::default` and `TOOL_DEFAULTS`).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use crate::settings::{Kind, Setting};

const DISPLAY: &str = "Display";
const SOUND: &str = "Sound";
const INTERFACE: &str = "Interface";
const EXTRAS: &str = "Extras";
const GAMEPAD: &str = "Gamepad";

const fn cfg_bool(key: &'static str, group: &'static str, label: &'static str, hint: &'static str, default: bool) -> Setting {
    Setting { file: "cfg", key, group, label, hint, kind: Kind::Bool, default: if default { 1.0 } else { 0.0 } }
}

const fn tool_switch(key: &'static str, group: &'static str, label: &'static str, hint: &'static str, default: bool) -> Setting {
    Setting { file: "tool", key, group, label, hint, kind: Kind::Switch, default: if default { 1.0 } else { 0.0 } }
}

pub const SETTINGS: &[Setting] = &[
    Setting {
        file: "tool",
        key: "0034",
        group: DISPLAY,
        label: "Window mode",
        hint: "",
        kind: Kind::Choice {
            options: &[(1, "Window"), (0, "Full screen"), (2, "Borderless, window size"), (3, "Borderless, whole screen")],
        },
        default: 1.0,
    },
    Setting {
        file: "tool",
        key: "0001",
        group: DISPLAY,
        label: "Window width",
        hint: "In pixels.",
        kind: Kind::Number { min: 320.0, max: 16384.0, whole: true },
        default: 1280.0,
    },
    Setting {
        file: "tool",
        key: "0002",
        group: DISPLAY,
        label: "Window height",
        hint: "",
        kind: Kind::Number { min: 320.0, max: 16384.0, whole: true },
        default: 800.0,
    },
    Setting {
        file: "cfg",
        key: "frame_limit",
        group: DISPLAY,
        label: "Frame limit",
        hint: "Game time runs at its own pace whatever this is.",
        kind: Kind::Choice {
            options: &[
                (30, "30 fps"),
                (60, "60 fps"),
                (75, "75 fps"),
                (90, "90 fps"),
                (100, "100 fps"),
                (120, "120 fps"),
                (144, "144 fps"),
                (165, "165 fps"),
                (180, "180 fps"),
                (200, "200 fps"),
                (240, "240 fps"),
                (300, "300 fps"),
                (360, "360 fps"),
                (0, "No limit"),
            ],
        },
        default: 300.0,
    },
    Setting {
        file: "cfg",
        key: "draw_distance",
        group: DISPLAY,
        label: "Draw distance",
        hint: "In percent of retail's; 0 draws as far as the zone allows.",
        kind: Kind::Number { min: 0.0, max: 1000.0, whole: true },
        default: 0.0,
    },
    Setting {
        file: "cfg",
        key: "anisotropy",
        group: DISPLAY,
        label: "Anisotropic filtering",
        hint: "Sharper textures at a slant.",
        kind: Kind::Choice { options: &[(1, "Off"), (2, "2×"), (4, "4×"), (8, "8×"), (16, "16×")] },
        default: 16.0,
    },
    cfg_bool("lamp_lights", DISPLAY, "Lamp lights", "Real lights at the zones' lamps in place of their painted-on light.", true),
    tool_switch("0007", SOUND, "Sound", "", true),
    tool_switch("0035", SOUND, "Sound in the background", "Keep playing sound while the game window isn't in front.", true),
    tool_switch("0022", INTERFACE, "Opening movie", "", true),
    cfg_bool("modern_ui", INTERFACE, "Modern interface", "vana-client's modern windows in place of the retail ones they replace.", false),
    Setting {
        file: "cfg",
        key: "modern_scale",
        group: INTERFACE,
        label: "Modern interface size",
        hint: "In percent.",
        kind: Kind::Number { min: 30.0, max: 200.0, whole: true },
        default: 60.0,
    },
    cfg_bool("party_tp", EXTRAS, "Party TP", "Show party members' TP.", true),
    cfg_bool("minimap", EXTRAS, "Minimap", "", false),
    cfg_bool("timers", EXTRAS, "Timers window", "Buff and recast timers.", false),
    cfg_bool("skillchain_helper", EXTRAS, "Skillchain helper", "In a party, the skillchains it can make.", false),
    cfg_bool("target_distance", EXTRAS, "Target distance", "", false),
    cfg_bool("target_hpp", EXTRAS, "Target HP percent", "", false),
    cfg_bool("earth_clock", EXTRAS, "Earth clock", "Earth time, the moon and the next day beside the Vana'diel clock.", false),
    cfg_bool("short_battle", EXTRAS, "Short battle messages", "", false),
    cfg_bool("battle_name_colours", EXTRAS, "Coloured names in battle messages", "", false),
    cfg_bool("hide_trust_speech", EXTRAS, "Hide trusts' chatter", "", false),
    Setting {
        file: "cfg",
        key: "auto_advance",
        group: EXTRAS,
        label: "Auto-advance cutscene text",
        hint: "Seconds before waiting text goes on by itself; 0 waits for a key.",
        kind: Kind::Number { min: 0.0, max: 60.0, whole: true },
        default: 0.0,
    },
    cfg_bool("gamepad", GAMEPAD, "Gamepad", "Play with a game controller: the left stick moves, the right stick turns the camera, the buttons press keys.", true),
    Setting {
        file: "cfg",
        key: "gamepad_deadzone",
        group: GAMEPAD,
        label: "Stick dead zone",
        hint: "How far a stick moves before it counts, from 0 to 0.9. The original game uses 0.33.",
        kind: Kind::Number { min: 0.0, max: 0.9, whole: false },
        default: 0.33,
    },
    cfg_bool("gamepad_invert_y", GAMEPAD, "Invert camera up and down", "", false),
];

pub type Values = BTreeMap<String, f64>;

fn id(s: &Setting) -> String {
    format!("{}.{}", s.file, s.key)
}

fn read(path: &Path) -> Value {
    fs::read(path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}))
}

fn number(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_bool().map(|b| if b { 1.0 } else { 0.0 }))
}

/// The settings' current values, defaults for the ones the config doesn't have.
pub fn load(path: &Path) -> Values {
    let config = read(path);
    SETTINGS
        .iter()
        .map(|s| {
            let v = match s.file {
                "tool" => &config["tool"][s.key],
                _ => &config[s.key],
            };
            (id(s), number(v).unwrap_or(s.default))
        })
        .collect()
}

/// Writes the values into the config, keeping everything else in it.
pub fn save(path: &Path, values: &Values) -> Result<(), String> {
    let mut config = read(path);
    for s in SETTINGS {
        let Some(&v) = values.get(&id(s)) else { continue };
        let value = match s.kind {
            Kind::Number { min, max, .. } if !(min..=max).contains(&v) => {
                return Err(format!("{} must be from {min} to {max}.", s.label));
            }
            Kind::Choice { options } if !options.iter().any(|(o, _)| *o as f64 == v) => {
                return Err(format!("{} can't be {v}.", s.label));
            }
            Kind::Bool => json!(v != 0.0),
            Kind::Number { whole: false, .. } => json!(v),
            _ => json!(v.round() as i64),
        };
        let obj = config.as_object_mut().unwrap();
        if s.file == "tool" {
            let tool = obj.entry("tool").or_insert_with(|| json!({}));
            if !tool.is_object() {
                *tool = json!({});
            }
            tool.as_object_mut().unwrap().insert(s.key.to_owned(), value);
        } else {
            obj.insert(s.key.to_owned(), value);
        }
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| format!("Couldn't save {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("clientcfg-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("config.json")
    }

    #[test]
    fn defaults_without_a_config() {
        let path = temp("defaults");
        let v = load(&path);
        assert_eq!(v["cfg.frame_limit"], 300.0);
        assert_eq!(v["tool.0034"], 1.0);
        assert_eq!(v["cfg.lamp_lights"], 1.0);
    }

    #[test]
    fn saves_in_place_and_keeps_the_rest() {
        let path = temp("save");
        fs::write(&path, r#"{"server":"127.0.0.1","logins":[{"username":"me"}],"tool":{"0003":512},"minimap":false}"#).unwrap();
        let mut values = load(&path);
        values.insert("cfg.minimap".into(), 1.0);
        values.insert("cfg.frame_limit".into(), 144.0);
        values.insert("tool.0034".into(), 3.0);
        values.insert("tool.0007".into(), 0.0);
        save(&path, &values).unwrap();
        let c = read(&path);
        assert_eq!(c["minimap"], true);
        assert_eq!(c["frame_limit"], 144);
        assert_eq!(c["tool"]["0034"], 3);
        assert_eq!(c["tool"]["0007"], 0);
        assert_eq!(c["tool"]["0003"], 512);
        assert_eq!(c["logins"][0]["username"], "me");
        assert_eq!(load(&path)["cfg.minimap"], 1.0);
    }

    #[test]
    fn refuses_values_out_of_range() {
        let path = temp("range");
        let mut values = load(&path);
        values.insert("cfg.frame_limit".into(), 55.0);
        assert!(save(&path, &values).is_err());
        values.insert("cfg.frame_limit".into(), 60.0);
        values.insert("tool.0001".into(), 10.0);
        assert!(save(&path, &values).is_err());
        assert!(!path.exists());
    }
}
