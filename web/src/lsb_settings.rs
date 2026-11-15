//! The server's `MAX_LEVEL`, read from LSB's settings overrides (`overrides/settings/*.lua`,
//! mounted read-only) the way LSB loads them: in name order (the desktop app's `zz_app.lua`
//! last), the last plain number set for it winning, LSB's default when none is.
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

/// What the folder looked like (each file's name, size and modified time) and the level read then.
type Seen = (PathBuf, Vec<(PathBuf, u64, Option<SystemTime>)>, i64);
static CACHE: Mutex<Option<Seen>> = Mutex::new(None);

/// [`max_level`] off the async runtime: the file calls block.
pub async fn max_level_async(dir: Option<PathBuf>) -> i64 {
    tokio::task::spawn_blocking(move || max_level(dir.as_deref())).await.unwrap_or(LSB_MAX_LEVEL)
}

/// The level, re-reading the files only when one of them was added, removed or changed.
pub fn max_level(dir: Option<&Path>) -> i64 {
    let Some(dir) = dir else { return LSB_MAX_LEVEL };
    let Ok(rd) = std::fs::read_dir(dir) else { return LSB_MAX_LEVEL };
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
    if let Some((d, f, level)) = cache.as_ref() {
        if d == dir && *f == files {
            return *level;
        }
    }
    let level = files.iter().fold(LSB_MAX_LEVEL, |level, (p, _, _)| {
        std::fs::read_to_string(p).ok().and_then(|src| last_max_level(&src)).unwrap_or(level)
    });
    *cache = Some((dir.to_path_buf(), files, level));
    level
}

/// The last plain-number `MAX_LEVEL = n` in one file's code (`xi.settings.main.MAX_LEVEL = 75`
/// or a `MAX_LEVEL = 75,` table field), clamped to 1-99.
pub fn last_max_level(src: &str) -> Option<i64> {
    let code = strip_comments_and_strings(src);
    let mut found = None;
    for (at, _) in code.match_indices("MAX_LEVEL") {
        if code[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let Some(rest) = code[at + "MAX_LEVEL".len()..].trim_start().strip_prefix('=') else { continue };
        if rest.starts_with('=') {
            continue; // `==` is a comparison
        }
        let rest = rest.trim_start();
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        // Only a whole plain number: `75,` `75}` or a line end, not `75 + x` or `7.5`.
        let after = rest[digits.len()..].trim_start();
        if digits.is_empty() || !(after.is_empty() || after.starts_with([',', ';', '}']) || after.starts_with(|c: char| c.is_alphabetic())) {
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
