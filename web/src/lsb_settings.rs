//! Whether LandSandBoat's login server lets people create accounts (`login.ACCOUNT_CREATION`),
//! worked out from the settings files the stack mounts read-only (`overrides/settings/*.lua`),
//! the same files LSB loads on top of its own defaults.
//!
//! The world server's HTTP API could say what is running now, but it is off by default and
//! bound to localhost, so using it would mean opening an unauthenticated API inside the
//! compose network and recreating `world`. The files need neither.
//!
//! Anything that isn't a plain `true` or `false` after the key (a variable, `os.getenv`, an
//! expression) is "unknown", and unknown means closed.

use std::path::Path;

/// LSB's own default at the pinned commit (`settings/default/login.lua`).
pub const LSB_DEFAULT: bool = true;

#[derive(Debug, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Str(String),
    Punct(char),
}

/// Cuts Lua source into identifiers, strings and punctuation, dropping comments, numbers and spaces.
fn lex(src: &str) -> Vec<Tok> {
    let s: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    // `[[` or `[==[` at i: the level and the index after the opener.
    let long_open = |i: usize| -> Option<(usize, usize)> {
        if s.get(i) != Some(&'[') {
            return None;
        }
        let mut j = i + 1;
        while s.get(j) == Some(&'=') {
            j += 1;
        }
        (s.get(j) == Some(&'[')).then_some((j - i - 1, j + 1))
    };
    let long_end = |from: usize, level: usize| -> usize {
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
    };
    while i < s.len() {
        let c = s[i];
        if c == '-' && s.get(i + 1) == Some(&'-') {
            i += 2;
            if let Some((level, after)) = long_open(i) {
                i = long_end(after, level);
            } else {
                while i < s.len() && s[i] != '\n' {
                    i += 1;
                }
            }
        } else if c == '"' || c == '\'' {
            let mut j = i + 1;
            let mut text = String::new();
            while j < s.len() && s[j] != c && s[j] != '\n' {
                if s[j] == '\\' && j + 1 < s.len() {
                    j += 1;
                }
                text.push(s[j]);
                j += 1;
            }
            out.push(Tok::Str(text));
            i = j + 1;
        } else if let Some((level, after)) = long_open(i) {
            let end = long_end(after, level);
            out.push(Tok::Str(s[after..end.saturating_sub(level + 2).max(after)].iter().collect()));
            i = end;
        } else if c.is_ascii_alphabetic() || c == '_' {
            let mut j = i;
            while j < s.len() && (s[j].is_ascii_alphanumeric() || s[j] == '_') {
                j += 1;
            }
            out.push(Tok::Ident(s[i..j].iter().collect()));
            i = j;
        } else if c.is_ascii_digit() {
            while i < s.len() && (s[i].is_ascii_alphanumeric() || s[i] == '.') {
                i += 1;
            }
        } else {
            if !c.is_whitespace() {
                out.push(Tok::Punct(c));
            }
            i += 1;
        }
    }
    out
}

/// What one settings file says about the key: `Ok(None)` when it doesn't mention it,
/// `Err(())` when it does in a way this can't read for certain.
#[allow(clippy::result_unit_err)]
pub fn scan(src: &str) -> Result<Option<bool>, ()> {
    let toks = lex(src);
    let mut found = None;
    for (n, t) in toks.iter().enumerate() {
        match t {
            Tok::Str(s) if s == "ACCOUNT_CREATION" => return Err(()),
            Tok::Ident(id) if id == "ACCOUNT_CREATION" => {
                // `= true` / `= false`, a lone `=` (not `==`), then the end of the value.
                let value = match (toks.get(n + 1), toks.get(n + 2)) {
                    (Some(Tok::Punct('=')), Some(Tok::Ident(v))) if v == "true" => true,
                    (Some(Tok::Punct('=')), Some(Tok::Ident(v))) if v == "false" => false,
                    _ => return Err(()),
                };
                match toks.get(n + 3) {
                    None | Some(Tok::Punct(',' | '}' | ';')) => {}
                    Some(Tok::Ident(w)) if w != "and" && w != "or" => {}
                    _ => return Err(()),
                }
                found = Some(value);
            }
            _ => {}
        }
    }
    // An assignment that may not run (inside an `if`, after a `return`) can't be taken as given.
    const FLOW: [&str; 11] = ["if", "elseif", "else", "return", "function", "while", "for", "repeat", "goto", "do", "break"];
    if found.is_some() && toks.iter().any(|t| matches!(t, Tok::Ident(w) if FLOW.contains(&w.as_str()))) {
        return Err(());
    }
    Ok(found)
}

/// The effective value: the last file (in name order, as LSB loads them) that sets it wins,
/// and LSB's default stands when none does. `None`: the folder can't be read, or a file
/// sets it in a way that can't be read for certain.
pub fn account_creation(dir: &Path) -> Option<bool> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "lua"))
        .collect();
    files.sort();
    let mut value = LSB_DEFAULT;
    for f in files {
        let text = std::fs::read_to_string(&f).ok()?;
        if let Some(v) = scan(&text).ok()? {
            value = v;
        }
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_assignments_are_read() {
        assert_eq!(scan("xi.settings.login.ACCOUNT_CREATION = false"), Ok(Some(false)));
        assert_eq!(scan("xi.settings.login =\n{\n    ACCOUNT_CREATION = true,\n    X = 1,\n}"), Ok(Some(true)));
        assert_eq!(scan("xi.settings.login = { ACCOUNT_CREATION = false }"), Ok(Some(false)));
        assert_eq!(scan("xi.settings.map = { EXP_RATE = 2.0 }"), Ok(None));
    }

    #[test]
    fn the_last_assignment_in_a_file_wins() {
        assert_eq!(scan("xi.settings.login.ACCOUNT_CREATION = true\nxi.settings.login.ACCOUNT_CREATION = false"), Ok(Some(false)));
    }

    #[test]
    fn comments_and_other_strings_do_not_count() {
        assert_eq!(scan("-- ACCOUNT_CREATION = false\nx = 1"), Ok(None));
        assert_eq!(scan("--[[ ACCOUNT_CREATION = false ]] y = 2"), Ok(None));
        assert_eq!(scan("--[==[\nACCOUNT_CREATION = false\n]==]\nz = 1"), Ok(None));
        assert_eq!(scan("msg = 'ACCOUNT_CREATION = false'"), Ok(None), "text that merely mentions it");
        assert_eq!(scan("x = 'ACCOUNT_CREATION'"), Err(()), "the bare name as a string (a table key): not sure what it does");
    }

    #[test]
    fn anything_but_a_literal_is_unknown() {
        for src in [
            "if x then xi.settings.login.ACCOUNT_CREATION = false end",
            "return\nxi.settings.login.ACCOUNT_CREATION = false",
            "local function f() xi.settings.login.ACCOUNT_CREATION = false end",
            "xi.settings.login.ACCOUNT_CREATION = os.getenv('X') == '1'",
            "xi.settings.login.ACCOUNT_CREATION = enabled",
            "xi.settings.login.ACCOUNT_CREATION = 0",
            "xi.settings.login.ACCOUNT_CREATION = false or x",
            "xi.settings.login.ACCOUNT_CREATION == false",
            "xi.settings.login[\"ACCOUNT_CREATION\"] = false",
            "xi.settings.login.ACCOUNT_CREATION = true and y",
            "xi.settings.login.ACCOUNT_CREATION",
        ] {
            assert_eq!(scan(src), Err(()), "{src}");
        }
    }

    #[test]
    fn files_apply_in_name_order_over_the_default() {
        let dir = std::env::temp_dir().join(format!("lsbset-{}-{:?}", std::process::id(), std::thread::current().id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(account_creation(&dir), Some(true), "nothing sets it: LSB's default");
        std::fs::write(dir.join("login.lua"), "xi.settings.login = { ACCOUNT_CREATION = false }").unwrap();
        assert_eq!(account_creation(&dir), Some(false));
        std::fs::write(dir.join("zz_app.lua"), "xi.settings.login = { ACCOUNT_CREATION = true }").unwrap();
        assert_eq!(account_creation(&dir), Some(true), "zz_app.lua loads last");
        std::fs::write(dir.join("a.lua"), "xi.settings.login.ACCOUNT_CREATION = nope").unwrap();
        assert_eq!(account_creation(&dir), None, "an unreadable value in any file");
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(account_creation(&dir), None, "no folder");
    }
}
