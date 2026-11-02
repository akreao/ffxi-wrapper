//! Reads and edits the stack's `.env`, keeping its comments and order.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Held across every load, change and save of `.env`, so a UI command and
/// a running task can't each save over the other's change.
static EDITING: Mutex<()> = Mutex::new(());

/// Loads the `.env` at `path`, lets `change` edit it and saves it, with no
/// other edit in between. Nothing is saved when `change` fails.
pub fn edit<T>(path: &Path, change: impl FnOnce(&mut EnvFile) -> Result<T, String>) -> Result<T, String> {
    let _held = EDITING.lock().unwrap_or_else(|e| e.into_inner());
    let mut env = EnvFile::load(path).map_err(|e| e.to_string())?;
    let out = change(&mut env)?;
    env.save().map_err(|e| e.to_string())?;
    Ok(out)
}

pub struct EnvFile {
    path: PathBuf,
    lines: Vec<String>,
}

impl EnvFile {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let text = fs::read_to_string(path)?;
        Ok(Self::parse(&text).at(path))
    }

    /// Reads settings from text, such as the bundled `.env.example`.
    pub fn parse(text: &str) -> Self {
        Self { path: PathBuf::new(), lines: text.lines().map(str::to_owned).collect() }
    }

    /// The same settings, saved to `path`.
    pub fn at(self, path: &Path) -> Self {
        Self { path: path.to_owned(), ..self }
    }

    /// The key's value as Compose reads it: from the last line that sets
    /// it, without quotes or a ` #` comment after an unquoted value.
    pub fn get(&self, key: &str) -> Option<String> {
        self.lines.iter().rev().find_map(|line| {
            let (k, v) = line.split_once('=')?;
            (k.trim() == key).then(|| value(v))
        })
    }

    /// Replaces the key's value, or appends `KEY=value` if it isn't there.
    pub fn set(&mut self, key: &str, value: &str) {
        let entry = format!("{key}={value}");
        match self
            .lines
            .iter_mut()
            .rev()
            .find(|line| matches!(line.split_once('='), Some((k, _)) if k.trim() == key))
        {
            Some(line) => *line = entry,
            None => self.lines.push(entry),
        }
    }

    /// Writes the file, readable by its owner only on Linux, as it holds passwords.
    pub fn save(&self) -> std::io::Result<()> {
        let mut text = self.lines.join("\n");
        text.push('\n');
        write_private(&self.path, &text)
    }
}

/// Writes `text` to `path` through a temporary file that replaces it once
/// it's on disk, so a full disk or a crash can't leave it half written.
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = path.with_file_name(format!("{name}.tmp"));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options
        .open(&tmp)
        .and_then(|mut file| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(0o600))?;
            }
            file.write_all(text.as_bytes())?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&tmp, path));
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

/// A value as Compose reads it: quotes removed, or for an unquoted value,
/// a comment after ` #` and the spaces around it.
fn value(raw: &str) -> String {
    let v = raw.trim();
    for q in ['"', '\''] {
        if let Some((inner, _)) = v.strip_prefix(q).and_then(|s| s.split_once(q)) {
            return inner.to_owned();
        }
    }
    let v = match v.find(" #").or_else(|| v.find("\t#")) {
        Some(i) => &v[..i],
        None => v,
    };
    v.trim().to_owned()
}

/// A random password of letters and digits, safe in `.env` and SQL as-is.
pub fn random_password(len: usize) -> Result<String, String> {
    const CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|b| CHARS[*b as usize % CHARS.len()] as char).collect())
}

/// Checks a server name for `.env`: one line, no quotes, which Compose and
/// LSB's settings would read differently.
pub fn check_server_name(name: &str) -> Result<(), String> {
    if name.len() > 30 {
        return Err("The server name can be at most 30 characters.".into());
    }
    if name.chars().any(|c| c.is_control() || c == '"' || c == '\'' || c == '#' || c == '$') {
        return Err("The server name can't contain quotes, #, $ or line breaks.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_replaces_and_appends() {
        let dir = std::env::temp_dir().join(format!("envfile-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env");
        fs::write(&path, "# comment\nA=1\nB=\"two\"\n").unwrap();

        let mut env = EnvFile::load(&path).unwrap();
        assert_eq!(env.get("B").as_deref(), Some("two"));
        env.set("A", "9");
        env.set("C", "3");
        env.save().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "# comment\nA=9\nB=\"two\"\nC=3\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(!dir.join(".env.tmp").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reads_values_the_way_compose_does() {
        let env = EnvFile::parse("A=1\nB=no  # comment\nA=2\nQ=\"x # y\"\nH=a#b\nE=\n");
        assert_eq!(env.get("A").as_deref(), Some("2"));
        assert_eq!(env.get("B").as_deref(), Some("no"));
        assert_eq!(env.get("Q").as_deref(), Some("x # y"));
        assert_eq!(env.get("H").as_deref(), Some("a#b"));
        assert_eq!(env.get("E").as_deref(), Some(""));
        assert_eq!(env.get("Z"), None);
    }

    #[test]
    fn edits_save_only_when_the_change_works() {
        let dir = std::env::temp_dir().join(format!("envfile-edit-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env");
        fs::write(&path, "A=1\n").unwrap();
        edit(&path, |env| {
            env.set("A", "2");
            Ok(())
        })
        .unwrap();
        let failed: Result<(), String> = edit(&path, |env| {
            env.set("A", "3");
            Err("no".into())
        });
        assert!(failed.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "A=2\n");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn passwords_and_names() {
        let p = random_password(24).unwrap();
        assert_eq!(p.len(), 24);
        assert!(p.chars().all(|c| c.is_ascii_alphanumeric()));
        assert!(check_server_name("Vana'diel").is_err());
        assert!(check_server_name("Home Server").is_ok());
        assert!(check_server_name("").is_ok());
        assert!(check_server_name("a\nb").is_err());
    }
}
