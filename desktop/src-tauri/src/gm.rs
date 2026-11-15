//! Game masters: the characters with a GM level, read-only. LSB keeps the
//! level per character (`chars.gmlevel`, 0 to 5); GM commands such as
//! `!promote` and `!goto` check it. Levels are changed on the website, which
//! checks who may change them and records each change in its audit log.

use std::path::Path;

use serde::Serialize;

use crate::stack;

/// How many game masters the screen lists; the website has the full list.
pub const SHOWN: usize = 50;

/// Runs one statement in `xidb`; the statement comes in as `$0`, so the
/// shell never reads it.
const QUERY: &str = r#"MYSQL_PWD="$MARIADB_ROOT_PASSWORD" exec mariadb --user=root --batch --skip-column-names xidb -e "$0""#;

/// Deleted characters keep their row with `accid` 0, so the join leaves them
/// out. One row more than shown tells the screen there are more.
const LIST: &str = "SELECT a.login, c.charname, c.gmlevel \
     FROM chars c JOIN accounts a ON a.id = c.accid \
     WHERE c.accid <> 0 AND c.gmlevel > 0 \
     ORDER BY c.gmlevel DESC, c.charname LIMIT 51";

#[derive(Serialize, Debug, PartialEq)]
pub struct GameMasters {
    pub characters: Vec<Character>,
    /// More than [`SHOWN`] characters have a GM level.
    pub more: bool,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Character {
    pub name: String,
    pub login: String,
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

pub fn list(dir: &Path) -> Result<GameMasters, String> {
    Ok(parse(&query(dir, LIST)?))
}

/// Reads `mariadb --batch` rows: tab-separated, with tabs, newlines and
/// backslashes escaped.
fn parse(out: &str) -> GameMasters {
    let mut characters: Vec<Character> = out
        .lines()
        .filter_map(|line| {
            let cols: Vec<String> = line.split('\t').map(unescape).collect();
            let [login, name, level] = &cols[..] else { return None };
            Some(Character { name: name.clone(), login: login.clone(), gm_level: level.parse().ok()? })
        })
        .collect();
    let more = characters.len() > SHOWN;
    characters.truncate(SHOWN);
    GameMasters { characters, more }
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
    fn reads_rows_and_unescapes_names() {
        let out = "steve\tAlpha\t5\nnew\\tbie\tBeta\t1\n";
        assert_eq!(
            parse(out),
            GameMasters {
                characters: vec![
                    Character { name: "Alpha".into(), login: "steve".into(), gm_level: 5 },
                    Character { name: "Beta".into(), login: "new\tbie".into(), gm_level: 1 },
                ],
                more: false,
            }
        );
    }

    #[test]
    fn the_extra_row_means_more() {
        let out: String = (0..=SHOWN).map(|i| format!("a\tC{i}\t1\n")).collect();
        let gms = parse(&out);
        assert_eq!(gms.characters.len(), SHOWN);
        assert!(gms.more);
    }
}
