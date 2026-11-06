//! The Docker Compose stack: where its files live and how to run `docker compose` on it.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::host;

/// Fixed so the containers and the `db-data` volume keep their names
/// wherever the stack files live.
pub const PROJECT: &str = "ffxi-wrapper";

/// The settings template, compiled in: Tauri's resource bundling skips dotfiles.
pub const ENV_EXAMPLE: &str = include_str!("../../../.env.example");

/// Every `LSB_COMMIT` an app version has pinned. Setup used to copy the pin
/// into `.env`, where it then stayed through app updates; a `.env` still
/// holding one of these follows the app's pin instead. Add the old pin here
/// whenever it moves. A commit someone typed in themselves is left alone.
const APP_LSB_PINS: [&str; 7] = [
    "6cc5715c79331addb38e2bd20445d1b3d3e48504",
    "226b3b7b699fee9daa8ec70d88b7b0834ef0f7ef",
    "6c02b556f1ce1a8d0acf38e14cb7be6ddadb9988",
    "e87cb4905e471dc44a7dd5dbf99742617d3fa2b4",
    "25f50df48a0fc5d041be184394f4fb8755dc3827",
    "ebc266659120c32b48470694d587d7df7dbcaaf2",
    "ab97ceef8ba7add609ef900b504b2cd104ccd388",
];

/// Empties a `LSB_COMMIT` in `env` that an older setup copied from the app's
/// pin, so compose.yaml's default (this app version's pin) applies. Returns
/// whether it changed anything.
pub fn follow_lsb_pin(env: &mut crate::envfile::EnvFile) -> bool {
    let pinned = env.get("LSB_COMMIT").is_some_and(|c| {
        APP_LSB_PINS.contains(&c.as_str())
            || crate::envfile::EnvFile::parse(ENV_EXAMPLE).get("LSB_COMMIT").as_deref() == Some(c.as_str())
    });
    if pinned {
        env.set("LSB_COMMIT", "");
    }
    pinned
}

/// The LSB services, which read `overrides/settings/`.
pub const GAME_SERVERS: [&str; 4] = ["connect", "search", "world", "map"];

/// Copies the stack files bundled with the app (compose.yaml, docker/, the
/// overrides README) and `.env.example` into `dest`, replacing older copies.
/// `.env`, the user's override files and `data/` are never bundled, so they
/// survive app updates.
pub fn install(bundled: &Path, dest: &Path) -> std::io::Result<()> {
    // Replaced whole, so a file an older version had doesn't linger.
    match fs::remove_dir_all(dest.join("docker")) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    copy_dir(bundled, dest)?;
    fs::create_dir_all(dest.join("overrides").join("settings"))?;
    fs::create_dir_all(dest.join("data"))?;
    fs::write(dest.join(".env.example"), ENV_EXAMPLE)
}

fn copy_dir(bundled: &Path, dest: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(bundled)? {
        let entry = entry?;
        let target = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

pub fn compose(dir: &Path) -> Command {
    let mut cmd = host::command("docker");
    cmd.current_dir(dir)
        .args(["compose", "--ansi", "never", "--project-name", PROJECT, "--project-directory"])
        .arg(dir)
        .arg("--file")
        .arg(dir.join("compose.yaml"))
        .stdin(Stdio::null());
    cmd
}

/// How long a quick Docker call may take before it's given up on, so a
/// hung Docker daemon can't hang the app.
pub const QUICK: Duration = Duration::from_secs(60);

/// Runs a command to completion and returns stdout, or stderr as the error.
pub fn output(cmd: Command) -> Result<String, String> {
    output_within(cmd, QUICK)
}

/// [`output`] for a command that may take up to `limit`.
pub fn output_within(mut cmd: Command, limit: Duration) -> Result<String, String> {
    let what = std::iter::once(cmd.get_program())
        .chain(cmd.get_args().take(if cmd.get_program() == "docker" { 2 } else { 1 }))
        .map(|a| a.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "Docker isn't installed (no `docker` command found).".to_owned(),
            _ => e.to_string(),
        })?;
    // Read from threads, so a full pipe can't stall the command.
    let read = |stream: Option<Box<dyn Read + Send>>| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut stream) = stream {
                let _ = stream.read_to_end(&mut buf);
            }
            buf
        })
    };
    let stdout = read(child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let stderr = read(child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("`{what}` didn't answer within {} seconds. Is Docker stuck?", limit.as_secs()));
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    if status.success() {
        Ok(String::from_utf8_lossy(&stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&stderr).trim().to_owned())
    }
}

/// Checks that Docker and its Compose plugin are usable by this user, and
/// returns Compose's version.
pub fn check_docker() -> Result<String, String> {
    let mut v = host::command("docker");
    v.args(["compose", "version", "--short"]);
    let version = output(v).map_err(|e| format!("Docker Compose isn't available: {e}"))?;
    if too_old(&version) {
        return Err(format!(
            "Docker Compose {} is too old: the server needs 2.24 or newer. Update Docker.",
            version.trim()
        ));
    }
    let mut info = host::command("docker");
    info.args(["info", "--format", "{{.ServerVersion}}"]);
    output(info).map_err(|e| {
        if e.contains("permission denied") {
            "This user can't talk to Docker. Add it to the docker group \
             (`sudo usermod -aG docker $USER`) and log in again."
                .to_owned()
        } else if cfg!(windows) {
            format!("Docker isn't running. Start Docker Desktop, then try again. ({e})")
        } else {
            format!("The Docker daemon isn't reachable: {e}")
        }
    })?;
    Ok(version.trim().to_owned())
}

/// Whether `docker compose version --short` names a version before 2.24,
/// which can't read `compose.yaml`'s `!reset`. A version it can't read is
/// given the benefit of the doubt.
fn too_old(version: &str) -> bool {
    let mut parts = version.trim().trim_start_matches('v').split('.');
    let mut number = || -> Option<u32> {
        let part = parts.next()?;
        let digits = part.find(|c: char| !c.is_ascii_digit()).map_or(part, |end| &part[..end]);
        digits.parse().ok()
    };
    match (number(), number()) {
        (Some(major), Some(minor)) => (major, minor) < (2, 24),
        _ => false,
    }
}

/// Where the database should live for a stack in `dir`: a folder beside
/// it when `dir` is on a Linux filesystem, which MariaDB needs, or else the
/// `db-data` Docker volume. On Windows it's always the volume: a Windows
/// folder mounted into a container lacks the file ownership and locking
/// MariaDB relies on.
pub fn db_location(dir: &Path) -> &'static str {
    if linux_filesystem(dir) {
        "./data/db"
    } else {
        "db-data"
    }
}

/// Whether the LSB server image the stack runs is already on this computer.
/// When it isn't, the next start downloads or builds it.
pub fn image_ready(dir: &Path) -> bool {
    let mut config = compose(dir);
    config.args(["config", "--format", "json"]);
    let Some(image) = output(config)
        .ok()
        .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
        .and_then(|c| c["services"]["db-init"]["image"].as_str().map(str::to_owned))
    else {
        return false;
    };
    let mut inspect = host::command("docker");
    inspect.args(["image", "inspect", "--format", "{{.Id}}", &image]);
    output(inspect).is_ok()
}

/// Whether Docker has this stack's volume `name`.
pub fn volume_exists(name: &str) -> bool {
    let mut inspect = host::command("docker");
    inspect.args(["volume", "inspect", &format!("{PROJECT}_{name}")]);
    output(inspect).is_ok()
}

#[cfg(target_os = "linux")]
fn linux_filesystem(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    // ext2/3/4, btrfs, xfs, f2fs, zfs, bcachefs, tmpfs.
    const LINUX: [i64; 7] = [0xEF53, 0x9123_683E, 0x5846_5342, 0xF2F5_2010, 0x2FC1_2FC1, 0xCA45_1A4E, 0x0102_1994];
    let Ok(path) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(path.as_ptr(), &mut stat) } != 0 {
        return false;
    }
    #[allow(clippy::unnecessary_cast)]
    LINUX.contains(&(stat.f_type as i64))
}

#[cfg(not(target_os = "linux"))]
fn linux_filesystem(_dir: &Path) -> bool {
    false
}

#[derive(Serialize, Default, Clone)]
pub struct ServiceStatus {
    pub service: String,
    /// created, running, restarting, exited, ... or "absent" when there is no container.
    pub state: String,
    pub health: String,
    pub exit_code: i64,
    pub status: String,
    pub ports: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PsEntry {
    service: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    health: String,
    #[serde(default)]
    exit_code: i64,
    #[serde(default)]
    status: String,
    #[serde(default)]
    publishers: Option<Vec<Publisher>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Publisher {
    #[serde(default)]
    published_port: u16,
    #[serde(default)]
    protocol: String,
}

/// The order services start in, which the app lists them in.
const ORDER: [&str; 8] = ["db", "db-init", "meshes", "world", "connect", "search", "map", "ahbot"];

/// Every service in compose.yaml with its container's state.
pub fn status(dir: &Path) -> Result<Vec<ServiceStatus>, String> {
    let mut services = compose(dir);
    services.args(["config", "--services"]);
    let names = output(services)?;

    let mut ps = compose(dir);
    ps.args(["ps", "--all", "--format", "json"]);
    let entries = parse_ps(&output(ps)?);

    let mut names: Vec<&str> = names.lines().filter(|n| !n.is_empty()).collect();
    // `config --services` lists them in no fixed order; show them in the
    // order they start.
    names.sort_by_key(|n| (ORDER.iter().position(|o| o == n).unwrap_or(ORDER.len()), *n));
    Ok(names
        .into_iter()
        .map(|name| match entries.iter().find(|e| e.service == name) {
            Some(e) => {
                let mut ports: Vec<String> = e
                    .publishers
                    .iter()
                    .flatten()
                    .filter(|p| p.published_port != 0)
                    .map(|p| match p.protocol.as_str() {
                        "udp" => format!("{}/udp", p.published_port),
                        _ => p.published_port.to_string(),
                    })
                    .collect();
                ports.sort();
                ports.dedup();
                ServiceStatus {
                    service: name.to_owned(),
                    state: e.state.clone(),
                    health: e.health.clone(),
                    exit_code: e.exit_code,
                    status: e.status.clone(),
                    ports,
                }
            }
            None => ServiceStatus { service: name.to_owned(), state: "absent".to_owned(), ..Default::default() },
        })
        .collect())
}

/// `docker compose ps --format json` prints one object per line in current
/// versions and a single array in older ones.
fn parse_ps(text: &str) -> Vec<PsEntry> {
    let text = text.trim();
    if text.starts_with('[') {
        return serde_json::from_str(text).unwrap_or_default();
    }
    text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// The TCP ports the stack publishes, from its compose config.
fn published_tcp_ports(dir: &Path) -> Result<Vec<u16>, String> {
    let mut config = compose(dir);
    config.args(["config", "--format", "json"]);
    Ok(parse_published(&output(config)?))
}

fn parse_published(json: &str) -> Vec<u16> {
    let Ok(config) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let mut ports: Vec<u16> = config["services"]
        .as_object()
        .into_iter()
        .flatten()
        .flat_map(|(_, def)| def["ports"].as_array().into_iter().flatten())
        .filter(|p| p["protocol"].as_str().unwrap_or("tcp") == "tcp")
        .filter_map(|p| match &p["published"] {
            serde_json::Value::String(s) => s.parse().ok(),
            v => v.as_u64().and_then(|n| u16::try_from(n).ok()),
        })
        .collect();
    ports.sort();
    ports.dedup();
    ports
}

/// Before a start: finds a TCP port the stack publishes that another
/// program holds, and says so, instead of Docker's "address already in
/// use" halfway through the start. This stack's own containers are fine.
pub fn check_ports(dir: &Path) -> Result<(), String> {
    let Ok(wanted) = published_tcp_ports(dir) else {
        return Ok(());
    };
    let mut ps = compose(dir);
    ps.args(["ps", "--format", "json"]);
    let ours: Vec<u16> = parse_ps(&output(ps).unwrap_or_default())
        .iter()
        .flat_map(|e| e.publishers.iter().flatten().map(|p| p.published_port))
        .collect();
    for port in wanted.into_iter().filter(|p| !ours.contains(p)) {
        if let Err(e) = std::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port)) {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                return Err(format!(
                    "Port {port} is in use by another program on this computer (another FFXI server?). \
                     Stop it, then start again."
                ));
            }
        }
    }
    Ok(())
}

/// Starts `cmd` with stdout and stderr piped, calling `on_line` for each
/// line of either from reader threads. Returns the child and a handle that
/// finishes once both streams close.
pub fn spawn_lines<F>(mut cmd: Command, on_line: F) -> std::io::Result<(Child, thread::JoinHandle<()>)>
where
    F: Fn(String) + Send + Sync + 'static,
{
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let on_line = std::sync::Arc::new(on_line);
    let readers: Vec<_> = [
        Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>,
        Box::new(child.stderr.take().unwrap()),
    ]
    .into_iter()
    .map(|stream| {
        let on_line = on_line.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut buf = Vec::new();
            while matches!(reader.read_until(b'\n', &mut buf), Ok(n) if n > 0) {
                let line = String::from_utf8_lossy(&buf);
                // Progress output redraws with \r; keep the last state.
                let line = line.trim_end().rsplit('\r').next().unwrap_or_default();
                on_line(line.to_owned());
                buf.clear();
            }
        })
    })
    .collect();
    let done = thread::spawn(move || {
        for r in readers {
            let _ = r.join();
        }
    });
    Ok((child, done))
}

/// Asks a process to stop the way Ctrl+C would, so `docker compose` can
/// cancel a build cleanly. Windows has no such signal, so there it's killed.
pub fn interrupt(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc_kill(child.id() as i32, 2);
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
}

#[cfg(unix)]
extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_before_2_24_is_too_old() {
        for old in ["2.20.2", "v2.23.3", "1.29.2\n"] {
            assert!(too_old(old), "{old}");
        }
        for new in ["2.24.0", "v2.40.3-desktop.1", "5.6.1\n", "", "unknown"] {
            assert!(!too_old(new), "{new}");
        }
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn install_replaces_the_stack_files_and_keeps_the_user_s() {
        let root = std::env::temp_dir().join(format!("stack-install-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (bundled, dest) = (root.join("bundled"), root.join("stack"));
        write(&bundled.join("compose.yaml"), "new compose");
        write(&bundled.join("docker/lsb/run.sh"), "new run");
        write(&bundled.join("overrides/settings/README.md"), "new readme");

        write(&dest.join("compose.yaml"), "old compose");
        write(&dest.join("docker/lsb/removed.sh"), "from an older version");
        write(&dest.join(".env"), "DB_PASSWORD=mine");
        write(&dest.join("overrides/settings/map.lua"), "mine");
        write(&dest.join("data/dbtool/config.yaml"), "state");

        install(&bundled, &dest).unwrap();

        let read = |rel: &str| fs::read_to_string(dest.join(rel)).unwrap();
        assert_eq!(read("compose.yaml"), "new compose");
        assert_eq!(read("docker/lsb/run.sh"), "new run");
        assert_eq!(read("overrides/settings/README.md"), "new readme");
        assert_eq!(read(".env.example"), ENV_EXAMPLE);
        assert!(!dest.join("docker/lsb/removed.sh").exists());
        assert_eq!(read(".env"), "DB_PASSWORD=mine");
        assert_eq!(read("overrides/settings/map.lua"), "mine");
        assert_eq!(read("data/dbtool/config.yaml"), "state");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn old_app_pins_follow_the_current_one() {
        use crate::envfile::EnvFile;
        let current = EnvFile::parse(ENV_EXAMPLE).get("LSB_COMMIT").unwrap();
        assert!(APP_LSB_PINS.contains(&current.as_str()), "add the new LSB pin to APP_LSB_PINS");
        for pin in [APP_LSB_PINS[1], current.as_str()] {
            let mut env = EnvFile::parse(&format!("LSB_COMMIT={pin}"));
            assert!(follow_lsb_pin(&mut env));
            assert_eq!(env.get("LSB_COMMIT").as_deref(), Some(""));
        }
        for own in ["", "0123456789abcdef0123456789abcdef01234567"] {
            let mut env = EnvFile::parse(&format!("LSB_COMMIT={own}"));
            assert!(!follow_lsb_pin(&mut env));
            assert_eq!(env.get("LSB_COMMIT").as_deref(), Some(own));
        }
    }

    #[test]
    fn parses_both_ps_formats() {
        let lines = r#"{"Service":"db","State":"running","Health":"healthy","ExitCode":0,"Publishers":[]}
{"Service":"map","State":"running","ExitCode":0,"Publishers":[{"URL":"0.0.0.0","TargetPort":54230,"PublishedPort":54230,"Protocol":"udp"}]}"#;
        let v = parse_ps(lines);
        assert_eq!(v.len(), 2);
        assert_eq!(v[1].publishers.as_ref().unwrap()[0].protocol, "udp");
        let array = r#"[{"Service":"db-init","State":"exited","ExitCode":0,"Publishers":null}]"#;
        assert_eq!(parse_ps(array)[0].service, "db-init");
        assert!(parse_ps("").is_empty());
    }

    #[test]
    fn reads_published_tcp_ports() {
        let config = r#"{"services":{"connect":{"ports":[{"target":54231,"published":"54231","protocol":"tcp"},
            {"target":54001,"published":"54001"}]},"map":{"ports":[{"target":54230,"published":"54230","protocol":"udp"}]},"db":{}}}"#;
        assert_eq!(parse_published(config), vec![54001, 54231]);
        assert!(parse_published("not json").is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_database_stays_off_filesystems_it_can_t_use() {
        // /proc is procfs: not somewhere MariaDB can keep its files.
        assert_eq!(db_location(Path::new("/proc")), "db-data");
        assert_eq!(db_location(Path::new("/does/not/exist")), "db-data");
    }
}
