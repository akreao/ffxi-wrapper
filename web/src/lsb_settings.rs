//! The few LSB settings the site needs (`MAX_LEVEL`, `PLAYER_BOTS_ENABLED`), read from LSB's
//! settings overrides (`overrides/settings/*.lua`, mounted read-only) the way LSB loads them: in
//! name order (the desktop app's `zz_app.lua` last), the last plain value set for each winning,
//! LSB's default when none is.
//!
//! Comments and strings are removed first as Lua's lexer would (`--` line comments,
//! `--[[ ]]` / `--[==[ ]==]` block comments, quoted and long-bracket strings), so a
//! commented-out or quoted `MAX_LEVEL = 50` doesn't count. A value that isn't a plain
//! number (a variable, an expression) is skipped.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// LSB's default `MAX_LEVEL` (`settings/default/main.lua`).
pub const LSB_MAX_LEVEL: i64 = 99;

/// LSB's default `PLAYER_BOTS_ENABLED` (`settings/default/map.lua`): off.
pub const LSB_BOTS_ENABLED: bool = false;

/// What was read from the folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Read {
    pub max_level: i64,
    /// Player bots ("AI adventurers") are switched on.
    pub bots_enabled: bool,
}

const DEFAULTS: Read = Read { max_level: LSB_MAX_LEVEL, bots_enabled: LSB_BOTS_ENABLED };

/// What the folder looked like (each file's name, size and modified time) and what was read then.
type Seen = (PathBuf, Vec<(PathBuf, u64, Option<SystemTime>)>, Read);
static CACHE: Mutex<Option<Seen>> = Mutex::new(None);

/// [`max_level`] off the async runtime: the file calls block.
pub async fn max_level_async(dir: Option<PathBuf>) -> i64 {
    read_async(dir).await.max_level
}

/// [`read`] off the async runtime: the file calls block.
pub async fn read_async(dir: Option<PathBuf>) -> Read {
    tokio::task::spawn_blocking(move || read(dir.as_deref())).await.unwrap_or(DEFAULTS)
}

/// The level cap.
pub fn max_level(dir: Option<&Path>) -> i64 {
    read(dir).max_level
}

/// Whether player bots are on.
pub fn bots_enabled(dir: Option<&Path>) -> bool {
    read(dir).bots_enabled
}

/// The settings, re-reading the files only when one of them was added, removed or changed.
pub fn read(dir: Option<&Path>) -> Read {
    let Some(dir) = dir else { return DEFAULTS };
    let Ok(rd) = std::fs::read_dir(dir) else { return DEFAULTS };
    let mut files: Vec<(PathBuf, u64, Option<SystemTime>)> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "lua"))
        .map(|p| {
            // Follows symlinks, so a changed link target counts as a change.
            let m = std::fs::metadata(&p).ok();
            let len = m.as_ref().map_or(0, |m| m.len());
            (p, len, m.and_then(|m| m.modified().ok()))
        })
        .collect();
    files.sort();
    let mut cache = CACHE.lock().unwrap();
    if let Some((d, f, read)) = cache.as_ref() {
        if d == dir && *f == files {
            return *read;
        }
    }
    let read = files.iter().fold(DEFAULTS, |read, (p, _, _)| {
        let Ok(src) = std::fs::read_to_string(p) else { return read };
        Read {
            max_level: last_max_level(&src).unwrap_or(read.max_level),
            bots_enabled: last_flag(&src, "PLAYER_BOTS_ENABLED").unwrap_or(read.bots_enabled),
        }
    });
    *cache = Some((dir.to_path_buf(), files, read));
    read
}

/// What follows each `KEY =` (not `==`, not a longer name ending in `KEY`) in code with the
/// comments and strings already removed, start trimmed.
fn assigned<'a>(code: &'a str, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    code.match_indices(key).filter_map(move |(at, _)| {
        if code[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
            return None;
        }
        let rest = code[at + key.len()..].trim_start().strip_prefix('=')?;
        // `==` is a comparison.
        (!rest.starts_with('=')).then(|| rest.trim_start())
    })
}

/// Does a value end here: `,` `;` `}`, a line end or the next statement, not `x + 1`.
fn value_ends(after: &str) -> bool {
    let after = after.trim_start();
    after.is_empty() || after.starts_with([',', ';', '}']) || after.starts_with(|c: char| c.is_alphabetic())
}

/// The last plain `KEY = true` or `KEY = false` in one file's code.
pub fn last_flag(src: &str, key: &str) -> Option<bool> {
    let code = strip_comments_and_strings(src);
    let mut found = None;
    for rest in assigned(&code, key) {
        for (word, value) in [("true", true), ("false", false)] {
            if let Some(after) = rest.strip_prefix(word) {
                if !after.starts_with(|c: char| c.is_alphanumeric() || c == '_') && value_ends(after) {
                    found = Some(value);
                }
            }
        }
    }
    found
}

/// The last plain-number `MAX_LEVEL = n` in one file's code (`xi.settings.main.MAX_LEVEL = 75`
/// or a `MAX_LEVEL = 75,` table field), clamped to 1-99.
pub fn last_max_level(src: &str) -> Option<i64> {
    let code = strip_comments_and_strings(src);
    let mut found = None;
    for rest in assigned(&code, "MAX_LEVEL") {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        // Only a whole plain number: `75,` `75}` or a line end, not `75 + x` or `7.5`.
        if digits.is_empty() || !value_ends(&rest[digits.len()..]) {
            continue;
        }
        if let Ok(v) = digits.parse::<i64>() {
            found = Some(v.clamp(1, 99));
        }
    }
    found
}

/// `[[`, `[=[`, ... at `i`: the bracket level and where the content starts.
fn long_open(s: &[char], i: usize) -> Option<(usize, usize)> {
    if s.get(i) != Some(&'[') {
        return None;
    }
    let mut j = i + 1;
    while s.get(j) == Some(&'=') {
        j += 1;
    }
    (s.get(j) == Some(&'[')).then_some((j - i - 1, j + 1))
}

/// Where the long bracket of `level` opened before `from` closes (just past it), or the end.
fn long_close(s: &[char], from: usize, level: usize) -> usize {
    let mut j = from;
    while j < s.len() {
        if s[j] == ']' {
            let mut k = j + 1;
            while s.get(k) == Some(&'=') {
                k += 1;
            }
            if k - j - 1 == level && s.get(k) == Some(&']') {
                return k + 1;
            }
        }
        j += 1;
    }
    s.len()
}

/// The source with every comment and string replaced by one space (line breaks kept).
pub fn strip_comments_and_strings(src: &str) -> String {
    let s: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let blank = |out: &mut String, from: usize, to: usize| {
        out.push(' ');
        out.extend(s[from..to.min(s.len())].iter().filter(|c| **c == '\n'));
    };
    while i < s.len() {
        let c = s[i];
        if c == '-' && s.get(i + 1) == Some(&'-') {
            let end = match long_open(&s, i + 2) {
                Some((level, body)) => long_close(&s, body, level),
                None => s[i..].iter().position(|c| *c == '\n').map_or(s.len(), |n| i + n),
            };
            blank(&mut out, i, end);
            i = end;
        } else if c == '"' || c == '\'' {
            let mut j = i + 1;
            while j < s.len() && s[j] != c && s[j] != '\n' {
                j += if s[j] == '\\' { 2 } else { 1 };
            }
            let end = (j + 1).min(s.len());
            blank(&mut out, i, end);
            i = end;
        } else if let Some((level, body)) = long_open(&s, i) {
            let end = long_close(&s, body, level);
            blank(&mut out, i, end);
            i = end;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}
