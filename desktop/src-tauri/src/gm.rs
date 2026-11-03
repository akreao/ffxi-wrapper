//! Game masters: the accounts and characters in LSB's database, and setting
//! a character's GM level. LSB keeps the level per character (`chars.gmlevel`,
//! 0 to 5); GM commands such as `!promote` and `!goto` check it.

use std::path::Path;

use serde::Serialize;

use crate::stack;

/// The highest level LSB's commands ask for.
pub const MAX_LEVEL: u8 = 5;

/// Runs one statement in `xidb`; the statement comes in as `$0`, so the
/// shell never reads it.
const QUERY: &str = r#"MYSQL_PWD="$MARIADB_ROOT_PASSWORD" exec mariadb --user=root --batch --skip-column-names xidb -e "$0""#;

/// Deleted characters keep their row with `accid` 0, so the join leaves them out.
const LIST: &str = "SELECT a.id, a.login, c.charid, c.charname, c.gmlevel \
     FROM accounts a LEFT JOIN chars c ON c.accid = a.id \
     ORDER BY a.login, c.charname";

#[derive(Serialize, Debug, PartialEq)]
pub struct Account {
    pub id: u32,
    pub login: String,
    pub characters: Vec<Character>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Character {
    pub id: u32,
    pub name: String,
    pub gm_level: u8,
}

fn query(dir: &Path, sql: &str) -> Result<String, String> {
    let mut cmd = stack::compose(dir);
    cmd.args(["exec", "-T", "db", "sh", "-c", QUERY, sql]);
    stack::output(cmd).map_err(|e| {
        if e.contains("not running") || e.contains("no such service") {
            "Start the server first: the database isn't running.".to_owned()
        } else {
            format!("The database didn't answer: {e}")
        }
    })
}

pub fn accounts(dir: &Path) -> Result<Vec<Account>, String> {
    Ok(parse(&query(dir, LIST)?))
}

pub fn set_level(dir: &Path, character: u32, level: u8) -> Result<(), String> {
    if level > MAX_LEVEL {
        return Err(format!("GM levels go from 0 to {MAX_LEVEL}."));
    }
    // Both values are numbers, so they can go straight into the statement.
    query(dir, &format!("UPDATE chars SET gmlevel = {level} WHERE charid = {character} AND accid <> 0"))?;
    Ok(())
}

/// Reads `mariadb --batch` rows: tab-separated, NULL for an account with
/// no characters, and tabs, newlines and backslashes escaped.
fn parse(out: &str) -> Vec<Account> {
    let mut accounts: Vec<Account> = Vec::new();
    for line in out.lines() {
        let cols: Vec<String> = line.split('\t').map(unescape).collect();
        let [id, login, char_id, name, level] = &cols[..] else { continue };
        let Ok(id) = id.parse() else { continue };
        if accounts.last().is_none_or(|a| a.id != id) {
            accounts.push(Account { id, login: login.clone(), characters: Vec::new() });
        }
        if let (Ok(char_id), Ok(gm_level)) = (char_id.parse(), level.parse()) {
            accounts.last_mut().unwrap().characters.push(Character { id: char_id, name: name.clone(), gm_level });
        }
    }
    accounts
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('0') => out.push('\0'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_characters_by_account() {
        let out = "1000\tsteve\t21828\tAlpha\t5\n1000\tsteve\t21829\tBeta\t0\n1001\tnew\\tbie\tNULL\tNULL\tNULL\n";
        assert_eq!(
            parse(out),
            vec![
                Account {
                    id: 1000,
                    login: "steve".into(),
                    characters: vec![
                        Character { id: 21828, name: "Alpha".into(), gm_level: 5 },
                        Character { id: 21829, name: "Beta".into(), gm_level: 0 },
                    ],
                },
                Account { id: 1001, login: "new\tbie".into(), characters: vec![] },
            ]
        );
    }

    #[test]
    fn refuses_levels_above_five() {
        assert!(set_level(Path::new("/nonexistent"), 1, 6).unwrap_err().contains("0 to 5"));
    }
}
