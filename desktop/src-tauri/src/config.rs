//! The app's own settings, kept outside the stack: which folder the server's
//! files live in. Everything else is in that folder's `.env`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const FILE: &str = "config.json";

#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    /// The folder with compose.yaml, `.env` and `data/`, chosen on first run.
    pub stack_dir: Option<PathBuf>,
}

pub fn load(config_dir: &Path) -> Config {
    fs::read_to_string(config_dir.join(FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(config_dir: &Path, config: &Config) -> Result<(), String> {
    fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(config_dir.join(FILE), text).map_err(|e| e.to_string())
}

/// Where the server's files go unless the user picks another folder: the
/// app's data folder.
pub fn default_stack_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("server")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_defaults() {
        let dir = std::env::temp_dir().join(format!("config-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert!(load(&dir).stack_dir.is_none());
        save(&dir, &Config { stack_dir: Some("/run/media/SSD/FFXI".into()) }).unwrap();
        assert_eq!(load(&dir).stack_dir.unwrap(), PathBuf::from("/run/media/SSD/FFXI"));
        fs::write(dir.join(FILE), "not json").unwrap();
        assert!(load(&dir).stack_dir.is_none());
        fs::remove_dir_all(dir).unwrap();
    }
}
