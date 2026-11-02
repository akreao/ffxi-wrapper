//! Database backups: gzipped `mariadb-dump`s of LSB's `xidb` database in
//! `backups/` in the server folder, the newest few of each kind kept, and
//! loading one back.

use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::Serialize;

use crate::envfile::EnvFile;
use crate::stack;
use crate::task::{self, Ctx};

/// How many backups made by hand, and how many made automatically, are
/// kept; each new one deletes the oldest of its kind beyond this.
pub const KEEP: usize = 10;

const PREFIX: &str = "xidb-";
const SUFFIX: &str = ".sql.gz";

/// LSB's tables are a mix of InnoDB, Aria and MyISAM, so a consistent dump
/// of a running server takes a global read lock rather than
/// `--single-transaction`; writes pause for the few seconds it takes.
/// Tables an unclean shutdown left marked crashed are repaired first, since
/// MariaDB won't repair them under the dump's lock. `--hex-blob` keeps the
/// characters' binary columns plain text, as LSB's dbtool does.
const DUMP: &str = r#"export MYSQL_PWD="$MARIADB_ROOT_PASSWORD"
mariadb-check --user=root --fast --auto-repair --silent --databases xidb >&2 || exit
exec mariadb-dump --user=root --hex-blob --lock-all-tables --routines --events --triggers --databases xidb"#;
const LOAD: &str = r#"MYSQL_PWD="$MARIADB_ROOT_PASSWORD" exec mariadb --user=root"#;

/// mariadb-dump's last line; a dump without it was cut short.
const COMPLETE: &[u8] = b"-- Dump completed";

/// How `mariadb-dump --databases xidb` starts the database.
const CREATES_XIDB: &[u8] = b"CREATE DATABASE /*!32312 IF NOT EXISTS*/ `xidb`";

/// Left in a restored database for `db-init` to find: dbtool's record of
/// the database's version, outside the database, describes the one before
/// the restore, so `db-init` brings the restored one up to date in full and
/// drops this.
const RESTORED_MARKER: &str = "ffxi_wrapper_restored";

/// Why a backup was made. Part of its file name.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Reason {
    Manual,
    /// On Stop, when there's no backup from the last day.
    Daily,
    BeforeRestore,
}

impl Reason {
    fn slug(self) -> &'static str {
        match self {
            Reason::Manual => "manual",
            Reason::Daily => "daily",
            Reason::BeforeRestore => "before-restore",
        }
    }
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Backup {
    pub file: String,
    /// Local time, "YYYY-MM-DD HH:MM:SS".
    pub created: String,
    /// manual, daily or before-restore; anything else a later version wrote.
    pub reason: String,
    pub size: u64,
}

pub fn folder(stack: &Path) -> PathBuf {
    stack.join("backups")
}

/// `xidb-20261102-071500-manual.sql.gz`: sorts by time, oldest first.
fn file_name(stamp: &str, reason: Reason) -> String {
    format!("{PREFIX}{stamp}-{}{SUFFIX}", reason.slug())
}

/// Makes the backups folder. On Linux only its owner can read it, as the
/// dumps hold every account's password hash.
fn ensure_folder(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn create_private(path: &Path) -> io::Result<File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// The time and reason in a backup's file name, or `None` for any other file.
fn parse(name: &str) -> Option<(String, String)> {
    let rest = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    let (date, rest) = rest.split_once('-')?;
    let (time, reason) = rest.split_once('-')?;
    let digits = |s: &str, n| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    // The reason is a slug, so a name can't reach outside the backups folder.
    let slug = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !digits(date, 8) || !digits(time, 6) || !slug(reason) {
        return None;
    }
    let created = format!(
        "{}-{}-{} {}:{}:{}",
        &date[..4],
        &date[4..6],
        &date[6..],
        &time[..2],
        &time[2..4],
        &time[4..]
    );
    Some((created, reason.to_owned()))
}

/// The backups in `dir`, newest first.
pub fn list(dir: &Path) -> Vec<Backup> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut backups: Vec<Backup> = entries
        .filter_map(|e| {
            let e = e.ok()?;
            let file = e.file_name().into_string().ok()?;
            let (created, reason) = parse(&file)?;
            let size = e.metadata().ok().filter(|m| m.is_file())?.len();
            Some(Backup { file, created, reason, size })
        })
        .collect();
    backups.sort_by(|a, b| b.file.cmp(&a.file));
    backups
}

/// Deletes all but the newest `KEEP` backups made by hand and the newest
/// `KEEP` made automatically, sparing `spare`. Automatic backups never push
/// out one made by hand.
fn prune(dir: &Path, spare: Option<&str>) {
    let (manual, automatic): (Vec<_>, Vec<_>) = list(dir).into_iter().partition(|b| b.reason == Reason::Manual.slug());
    for old in [manual, automatic].into_iter().flat_map(|kind| kind.into_iter().skip(KEEP)) {
        if Some(old.file.as_str()) != spare {
            let _ = fs::remove_file(dir.join(&old.file));
        }
    }
}

/// Whether the stack has a database yet: the `db-data` volume, or a
/// non-empty folder when `DB_DATA` names one.
fn database_exists(stack: &Path) -> bool {
    let db_data = EnvFile::load(&stack.join(".env"))
        .ok()
        .and_then(|e| e.get("DB_DATA"))
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "db-data".into());
    if db_data.contains('/') || db_data.contains('\\') {
        // MariaDB may keep the folder from being listed; then it has a database.
        match fs::read_dir(stack.join(&db_data)) {
            Ok(mut entries) => entries.next().is_some(),
            Err(e) => e.kind() == io::ErrorKind::PermissionDenied,
        }
    } else {
        stack::volume_exists(&db_data)
    }
}

fn db_running(stack: &Path) -> bool {
    stack::status(stack)
        .unwrap_or_default()
        .iter()
        .any(|s| s.service == "db" && s.state == "running")
}

/// Starts the database if it isn't running and waits until it's healthy.
/// Returns whether it was already running.
fn ensure_db(ctx: &Ctx) -> Result<bool, String> {
    let was_running = db_running(&ctx.dir);
    let mut up = stack::compose(&ctx.dir);
    up.args(["--progress", "plain", "up", "--detach", "--wait", "db"]);
    ctx.run(up)?;
    Ok(was_running)
}

fn stop_db(ctx: &Ctx) {
    let mut stop = stack::compose(&ctx.dir);
    stop.args(["stop", "db"]);
    let _ = ctx.run(stop);
}

/// Backs up the running database when there's no backup of any kind from
/// the last 24 hours. Used on Stop; a failure is reported but doesn't stop
/// the server from going down.
pub fn daily(ctx: &Ctx) {
    let now = chrono::Local::now().naive_local();
    if !db_running(&ctx.dir) || !due(newest(&folder(&ctx.dir)), now, 24) {
        return;
    }
    if let Err(e) = create(ctx, Reason::Daily, None) {
        ctx.line(e);
    }
}

fn newest(dir: &Path) -> Option<chrono::NaiveDateTime> {
    list(dir)
        .into_iter()
        .find_map(|b| chrono::NaiveDateTime::parse_from_str(&b.created, "%Y-%m-%d %H:%M:%S").ok())
}

fn due(newest: Option<chrono::NaiveDateTime>, now: chrono::NaiveDateTime, hours: i64) -> bool {
    newest.is_none_or(|t| now - t >= chrono::TimeDelta::hours(hours))
}

/// Dumps the database into a new backup, starting MariaDB for it if the
/// server is stopped, then deletes the oldest beyond `KEEP`.
pub fn create(ctx: &Ctx, reason: Reason, spare: Option<&str>) -> Result<(), String> {
    if !database_exists(&ctx.dir) {
        ctx.line("There's no database yet, so there's nothing to back up.");
        return Ok(());
    }
    ctx.line("Backing up the database…");
    let was_running = ensure_db(ctx)?;
    let result = dump(ctx, reason);
    if !was_running {
        stop_db(ctx);
    }
    result?;
    prune(&folder(&ctx.dir), spare);
    Ok(())
}

fn dump(ctx: &Ctx, reason: Reason) -> Result<String, String> {
    ctx.check_cancelled()?;
    let dir = folder(&ctx.dir);
    ensure_folder(&dir).map_err(|e| format!("Couldn't create the backups folder: {e}"))?;
    let stamp = || chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let mut file = file_name(&stamp(), reason);
    while dir.join(&file).exists() {
        // Two backups in the same second; the second waits for its own name.
        std::thread::sleep(std::time::Duration::from_millis(250));
        file = file_name(&stamp(), reason);
    }
    let path = dir.join(&file);
    let partial = dir.join(format!(".{file}.partial"));
    match dump_to(ctx, &partial).and_then(|size| fs::rename(&partial, &path).map(|()| size).map_err(|e| e.to_string())) {
        Ok(size) => {
            ctx.line(format!("Saved {file} ({}).", human_size(size)));
            Ok(file)
        }
        Err(e) => {
            let _ = fs::remove_file(&partial);
            Err(format!("The backup failed: {e}"))
        }
    }
}

/// Streams `mariadb-dump` through gzip into `dest`. Returns the file's size.
fn dump_to(ctx: &Ctx, dest: &Path) -> Result<u64, String> {
    let mut cmd = stack::compose(&ctx.dir);
    cmd.args(["exec", "-T", "db", "sh", "-c", DUMP])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let mut stdout = child.stdout.take().unwrap();
    let errors = child.stderr.take().map(|stderr| {
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                ctx.line(line);
            }
        })
    });
    let child = Arc::new(Mutex::new(child));
    ctx.track(Some(child.clone()));

    let copied = (|| -> io::Result<bool> {
        let mut gz = GzEncoder::new(BufWriter::new(create_private(dest)?), Compression::default());
        let mut tail = Tail::default();
        let mut buf = vec![0; 64 * 1024];
        loop {
            let n = stdout.read(&mut buf)?;
            if n == 0 {
                break;
            }
            tail.push(&buf[..n]);
            gz.write_all(&buf[..n])?;
        }
        gz.finish()?.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        Ok(tail.complete())
    })();

    let status = task::wait(&child);
    ctx.track(None);
    if let Some(t) = errors {
        let _ = t.join();
    }
    ctx.check_cancelled()?;
    if !status?.success() {
        return Err("mariadb-dump stopped with an error.".into());
    }
    match copied {
        Ok(true) => fs::metadata(dest).map(|m| m.len()).map_err(|e| e.to_string()),
        Ok(false) => Err("the dump ended early.".into()),
        Err(e) => Err(format!("couldn't write the file: {e}")),
    }
}

/// Keeps the last bytes seen, to check a dump's final line.
#[derive(Default)]
struct Tail(Vec<u8>);

impl Tail {
    const LEN: usize = 256;

    fn push(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(&bytes[bytes.len().saturating_sub(Self::LEN)..]);
        let excess = self.0.len().saturating_sub(Self::LEN);
        self.0.drain(..excess);
    }

    fn complete(&self) -> bool {
        let text = self.0.trim_ascii_end();
        let last = text.rsplit(|&b| b == b'\n').next().unwrap_or_default();
        last.starts_with(COMPLETE)
    }
}

/// Reads a backup through, checking it's a whole dump of `xidb`.
fn check(path: &Path) -> Result<(), String> {
    let gz = GzDecoder::new(BufReader::new(File::open(path).map_err(|e| e.to_string())?));
    let mut reader = BufReader::with_capacity(64 * 1024, gz);
    let mut creates = false;
    let mut tail = Tail::default();
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line).map_err(|e| format!("the file is damaged ({e})."))?;
        if n == 0 {
            break;
        }
        tail.push(&line);
        creates |= line.starts_with(CREATES_XIDB);
    }
    if !creates {
        Err("the file isn't a backup of LSB's database.".into())
    } else if !tail.complete() {
        Err("the file doesn't hold a whole database dump.".into())
    } else {
        Ok(())
    }
}

/// Replaces the database with a backup: checks the file, stops the game
/// servers, backs up the current database, loads the backup in its place,
/// and starts the server again if it was running. `db-init` brings the
/// restored database up to this LSB version's tables on that start.
pub fn restore(ctx: &Ctx, file: &str) -> Result<(), String> {
    if parse(file).is_none() {
        return Err(format!("{file} isn't a backup."));
    }
    let path = folder(&ctx.dir).join(file);
    ctx.line(format!("Checking {file}…"));
    check(&path).map_err(|e| format!("Can't restore {file}: {e}"))?;

    let services = stack::status(&ctx.dir)?;
    let was_serving = services
        .iter()
        .any(|s| stack::GAME_SERVERS.contains(&s.service.as_str()) && s.state == "running");
    ctx.line("Stopping the game servers…");
    let mut stop = stack::compose(&ctx.dir);
    stop.arg("stop").args(stack::GAME_SERVERS);
    ctx.run(stop)?;

    // Backed up after the servers stop, so it has their last saves.
    let had_db = database_exists(&ctx.dir);
    let db_was_running = ensure_db(ctx)?;
    if had_db {
        ctx.line("Backing up the current database first…");
        if let Err(e) = dump(ctx, Reason::BeforeRestore) {
            if was_serving {
                let _ = start(ctx);
            }
            return Err(format!("{e} Nothing was restored."));
        }
        prune(&folder(&ctx.dir), Some(file));
    }

    ctx.line(format!("Loading {file}…"));
    if let Err(e) = load(ctx, &path) {
        return Err(format!(
            "Loading failed: {e} The database may be incomplete, so the servers stay stopped. \
             Restore the \"before-restore\" backup to go back."
        ));
    }
    if was_serving {
        ctx.line("Restored. Starting the server again: it first brings the database up to date with this LSB version, which takes a few minutes.");
        start(ctx)?;
    } else {
        ctx.line("Restored. The next start brings it up to date with this LSB version, which takes a few minutes.");
        if !db_was_running {
            stop_db(ctx);
        }
    }
    Ok(())
}

fn start(ctx: &Ctx) -> Result<(), String> {
    stack::check_ports(&ctx.dir)?;
    let mut up = stack::compose(&ctx.dir);
    up.args(["--progress", "plain", "up", "--detach"]);
    ctx.run(up)
}

/// Feeds a backup to `mariadb`, into an emptied `xidb`, and marks it restored.
fn load(ctx: &Ctx, path: &Path) -> Result<(), String> {
    ctx.check_cancelled()?;
    let mut cmd = stack::compose(&ctx.dir);
    cmd.args(["exec", "-T", "db", "sh", "-c", LOAD]).stdin(Stdio::piped());
    let last = Arc::new(Mutex::new(String::new()));
    let lines = ctx.clone();
    let seen = last.clone();
    let (mut child, done) = stack::spawn_lines(cmd, move |line| {
        seen.lock().unwrap().clone_from(&line);
        lines.line(line);
    })
    .map_err(|e| e.to_string())?;
    let mut stdin = child.stdin.take().unwrap();
    let child = Arc::new(Mutex::new(child));
    ctx.track(Some(child.clone()));
    let fed = (|| -> io::Result<()> {
        // Tables made after the backup would otherwise survive the restore.
        stdin.write_all(b"DROP DATABASE IF EXISTS xidb;\n")?;
        io::copy(&mut GzDecoder::new(BufReader::new(File::open(path)?)), &mut stdin)?;
        stdin.write_all(format!("\nCREATE TABLE xidb.{RESTORED_MARKER} (restored DATETIME);\n").as_bytes())?;
        Ok(())
    })();
    drop(stdin);
    let _ = done.join();
    let status = task::wait(&child);
    ctx.track(None);
    ctx.check_cancelled()?;
    if !status?.success() {
        let last = last.lock().unwrap();
        return Err(if last.is_empty() {
            "MariaDB stopped with an error.".into()
        } else {
            format!("MariaDB stopped with an error: {last}")
        });
    }
    fed.map_err(|e| e.to_string())
}

fn human_size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.0} KB", b as f64 / (1 << 10) as f64),
        b => format!("{b} bytes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("backup-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn names_round_trip() {
        let name = file_name("20261102-075800", Reason::BeforeRestore);
        assert_eq!(name, "xidb-20261102-075800-before-restore.sql.gz");
        assert_eq!(parse(&name), Some(("2026-11-02 07:58:00".to_owned(), "before-restore".to_owned())));
        for other in [
            "xidb-20261102-0758-manual.sql.gz",
            "xidb-20261102-075800-.sql.gz",
            ".xidb-20261102-075800-manual.sql.gz.partial",
            "../xidb-20261102-075800-manual.sql.gz",
            "xidb-20260101-000000-x/../../../../tmp/evil.sql.gz",
            "xidb-20260101-000000-Manual.sql.gz",
            "notes.txt",
        ] {
            assert_eq!(parse(other), None, "{other}");
        }
    }

    #[test]
    fn keeps_the_newest_and_spares_one() {
        let dir = temp("prune");
        for day in 1..=12 {
            fs::write(dir.join(file_name(&format!("202610{day:02}-120000"), Reason::Manual)), b"x").unwrap();
        }
        for day in 1..=11 {
            fs::write(dir.join(file_name(&format!("202609{day:02}-120000"), Reason::Daily)), b"x").unwrap();
        }
        fs::write(dir.join("notes.txt"), b"mine").unwrap();
        let oldest = file_name("20261001-120000", Reason::Manual);

        prune(&dir, Some(&oldest));
        let left = list(&dir);
        assert_eq!(left.len(), 2 * KEEP + 1);
        assert!(!dir.join(file_name("20260901-120000", Reason::Daily)).exists());
        assert!(dir.join(file_name("20260902-120000", Reason::Daily)).exists());
        let manual: Vec<_> = left.into_iter().filter(|b| b.reason == "manual").collect();
        assert_eq!(manual[0].file, file_name("20261012-120000", Reason::Manual));
        assert_eq!(manual[KEEP].file, oldest);
        assert!(!dir.join(file_name("20261002-120000", Reason::Manual)).exists());
        assert!(dir.join("notes.txt").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn daily_is_due_after_a_day() {
        let at = |s| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap();
        let now = at("2026-11-02 12:00:00");
        assert!(due(None, now, 24));
        assert!(!due(Some(at("2026-11-01 12:00:01")), now, 24));
        assert!(due(Some(at("2026-11-01 12:00:00")), now, 24));
    }

    #[test]
    fn checks_a_dump_is_whole_and_of_xidb() {
        let dir = temp("check");
        let path = dir.join("b.sql.gz");
        let write = |text: &[u8]| {
            let mut gz = GzEncoder::new(File::create(&path).unwrap(), Compression::default());
            gz.write_all(text).unwrap();
            gz.finish().unwrap();
        };
        write(
            b"CREATE DATABASE /*!32312 IF NOT EXISTS*/ `xidb` /*!40100 DEFAULT CHARACTER SET utf8mb4 */;\n\
              USE `xidb`;\nINSERT INTO chars VALUES (1);\n-- Dump completed on 2026-11-02 12:07:58\n",
        );
        assert_eq!(check(&path), Ok(()));
        write(b"CREATE DATABASE /*!32312 IF NOT EXISTS*/ `xidb`;\nUSE `xidb`;\nINSERT INTO chars VALUES (1);\n");
        assert!(check(&path).unwrap_err().contains("whole"));
        write(b"CREATE DATABASE /*!32312 IF NOT EXISTS*/ `ragnarok`;\n-- Dump completed on 2026-11-02\n");
        assert!(check(&path).unwrap_err().contains("LSB"));
        fs::write(&path, b"not gzip").unwrap();
        assert!(check(&path).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn spots_a_cut_dump() {
        let mut tail = Tail::default();
        tail.push(b"INSERT INTO chars VALUES (1);\n");
        assert!(!tail.complete());
        tail.push(&[b'x'; 1000]);
        tail.push(b"\n-- Dump completed on 2026-11-02  7:58:00\n");
        assert!(tail.complete());
        tail.push(b"\n\n");
        assert!(tail.complete());
    }

    #[test]
    fn finds_a_database_folder() {
        let stack = temp("dbfolder");
        fs::write(stack.join(".env"), "DB_DATA=./data/db\n").unwrap();
        assert!(!database_exists(&stack));
        fs::create_dir_all(stack.join("data/db/mysql")).unwrap();
        assert!(database_exists(&stack));
        fs::remove_dir_all(&stack).unwrap();
    }
}
