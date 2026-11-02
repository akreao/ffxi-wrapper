//! The game client, vana-client (a native FFXI client for LandSandBoat
//! servers): everything it has lives in `client/` in the server folder: the
//! program, its settings (`config.json`, which `FFXI_CONFIG` points it at)
//! and the game data under `game/`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;
use serde_json::{json, Value};

use crate::envfile::EnvFile;
use crate::host;

/// Where the client builds come from: this repository's releases, one per
/// fork commit, tagged `client-<first 12 characters>` (the Game client
/// workflow publishes them).
pub const DOWNLOADS: &str = "https://github.com/akreao/ffxi-wrapper/releases/download";

/// The download address of the build of `commit` for this platform.
pub fn download_url(commit: &str) -> String {
    format!("{DOWNLOADS}/client-{}/{}", &commit[..commit.len().min(12)], asset())
}

/// The release file for this platform.
pub fn asset() -> &'static str {
    if cfg!(windows) {
        "vana-client-windows-x86_64.exe"
    } else {
        "vana-client-linux-x86_64"
    }
}

pub fn dir(stack: &Path) -> PathBuf {
    stack.join("client")
}

pub fn program(stack: &Path) -> PathBuf {
    dir(stack).join(if cfg!(windows) { "vana-client.exe" } else { "vana-client" })
}

pub fn config_path(stack: &Path) -> PathBuf {
    dir(stack).join("config.json")
}

/// Where the app puts the game data: `client/game/`.
fn default_game_dir(stack: &Path) -> PathBuf {
    dir(stack).join("game")
}

/// Where the game data is: the folder the client's config names (its first
/// start can put it in another folder, or use one already on disk), else
/// the app's default.
fn game_dir(stack: &Path) -> PathBuf {
    read_config(stack)["game_dir"]
        .as_str()
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| default_game_dir(stack))
}

/// Records which fork commit the downloaded vana-client was built from.
fn version_path(stack: &Path) -> PathBuf {
    dir(stack).join("version.txt")
}

#[derive(Serialize)]
pub struct Status {
    pub dir: String,
    /// vana-client is downloaded.
    pub program: bool,
    /// The downloaded vana-client is from another commit than the one `.env` pins.
    pub outdated: bool,
    /// The game data is installed.
    pub installed: bool,
    /// The official installer's first part, if one is in the server folder.
    pub installer: Option<String>,
}

pub fn status(stack: &Path, pinned: &str) -> Status {
    let config = read_config(stack);
    let built = fs::read_to_string(version_path(stack)).unwrap_or_default();
    let installed = config["installed"].as_bool() == Some(true)
        && game_dir(stack).join("FINAL FANTASY XI").join("VTABLE.DAT").is_file();
    Status {
        dir: dir(stack).display().to_string(),
        program: program(stack).is_file(),
        outdated: program(stack).is_file() && built.trim() != pinned,
        installed,
        installer: find_installer(stack).map(|p| p.display().to_string()),
    }
}

/// Looks for the first part of Square Enix's installer
/// (`FFXIFullSetup_US.part1.exe`) in the server folder, its `client/`
/// folder and the folders right inside the server folder.
pub fn find_installer(stack: &Path) -> Option<PathBuf> {
    let is_installer = |p: &Path| {
        p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
            let n = n.to_ascii_lowercase();
            n.starts_with("ffxifullsetup") && n.ends_with(".part1.exe")
        })
    };
    let mut dirs = vec![stack.to_owned(), dir(stack)];
    if let Ok(entries) = fs::read_dir(stack) {
        let mut subdirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && !p.ends_with("data") && !p.ends_with("client"))
            .collect();
        subdirs.sort();
        dirs.extend(subdirs);
    }
    dirs.iter().find_map(|d| {
        let mut found: Vec<PathBuf> = fs::read_dir(d).ok()?.flatten().map(|e| e.path()).filter(|p| is_installer(p)).collect();
        found.sort();
        found.into_iter().next()
    })
}

fn read_config(stack: &Path) -> Value {
    fs::read(config_path(stack))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| json!({}))
}

/// Points the client at this server, keeping every other setting the client
/// saved (its logins, graphics, key bindings). A new config gets the app's
/// game folder; one the client already has stays, since its first start may
/// have picked another. The server's login certificate is pinned too.
pub fn write_config(stack: &Path, server: &str) -> Result<(), String> {
    pin_login_cert(stack, server)?;
    let mut config = read_config(stack);
    if !config.is_object() {
        config = json!({});
    }
    let set = |config: &mut Value, key: &str, value: Value| {
        config.as_object_mut().unwrap().insert(key.to_owned(), value);
    };
    let game = game_dir(stack);
    set(&mut config, "game_dir", json!(game));
    set(&mut config, "server", json!(server));
    set(&mut config, "auth_port", json!(AUTH_PORT));
    set(&mut config, "data_port", json!(54230));
    set(&mut config, "view_port", json!(54001));
    fs::create_dir_all(dir(stack)).map_err(|e| e.to_string())?;
    let text = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
    fs::write(config_path(stack), text).map_err(|e| e.to_string())
}

/// The login server's certificate: xi_connect makes it on its first start, and
/// the stack keeps it in `DATA_DIR/tls/` (run.sh links it into the container).
pub fn login_cert(stack: &Path) -> PathBuf {
    let data = EnvFile::load(&stack.join(".env"))
        .ok()
        .and_then(|e| e.get("DATA_DIR"))
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "./data".into());
    stack.join(data).join("tls").join("login.cert")
}

/// The SHA-256 of a PEM file's first certificate (its DER), as lowercase hex:
/// the fingerprint vana-client pins.
fn cert_fingerprint(pem: &str) -> Option<String> {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let body = pem.split("-----BEGIN CERTIFICATE-----").nth(1)?.split("-----END CERTIFICATE-----").next()?;
    let b64: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    let der = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    Some(Sha256::digest(der).iter().map(|b| format!("{b:02x}")).collect())
}

/// vana-client's list of server certificates (`known_servers.json`, beside
/// the config `FFXI_CONFIG` names).
fn known_servers_path(stack: &Path) -> PathBuf {
    dir(stack).join("known_servers.json")
}

/// Pins this server's login certificate in vana-client's `known_servers.json`,
/// so the client accepts it from the first login and refuses any other. It's
/// marked as set by hand (`fixed`), like `vana-client trust-server
/// --fingerprint`. Other servers' entries stay. Without a certificate yet
/// (the server hasn't started), nothing changes. Clients from before
/// certificate pinning ignore the file.
fn pin_login_cert(stack: &Path, server: &str) -> Result<(), String> {
    let Some(sha256) = fs::read_to_string(login_cert(stack)).ok().as_deref().and_then(cert_fingerprint) else {
        return Ok(());
    };
    let path = known_servers_path(stack);
    let mut known = fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let key = format!("{}:{}", server.to_ascii_lowercase(), AUTH_PORT);
    let pin = json!({ "sha256": sha256, "fixed": true });
    if known[&key] == pin {
        return Ok(());
    }
    known.as_object_mut().unwrap().insert(key, pin);
    fs::create_dir_all(dir(stack)).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_vec_pretty(&known).map_err(|e| e.to_string())?;
    fs::write(&tmp, text).and_then(|_| fs::rename(&tmp, &path)).map_err(|e| format!("Couldn't save {}: {e}", path.display()))
}

/// The login server's port, where the client's TLS connection goes.
const AUTH_PORT: u16 = 54231;

/// `curl` downloading the latest release into `client/` under a temporary
/// name. curl comes with Linux distributions and with Windows 10 and later.
pub fn download(stack: &Path, commit: &str) -> Result<Command, String> {
    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("CLIENT_COMMIT in .env isn't a full commit SHA: \"{commit}\"."));
    }
    fs::create_dir_all(dir(stack)).map_err(|e| e.to_string())?;
    let mut cmd = host::command("curl");
    cmd.args(["--fail", "--location", "--retry", "2", "--progress-bar", "--output"])
        .arg(partial(stack))
        .arg(download_url(commit))
        .stdin(Stdio::null());
    Ok(cmd)
}

fn partial(stack: &Path) -> PathBuf {
    dir(stack).join("vana-client.download")
}

/// Puts a finished download of `commit` in place, executable.
pub fn finish_download(stack: &Path, commit: &str) -> Result<(), String> {
    let from = partial(stack);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&from, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    fs::rename(&from, program(stack)).map_err(|e| format!("Couldn't put vana-client in place: {e}"))?;
    fs::write(version_path(stack), commit).map_err(|e| e.to_string())
}

/// A vana-client command run with this server folder's settings. Its own
/// Update button would replace the fork's build with upstream's, so it's
/// pointed at the fork's releases instead, which the app doesn't use: the
/// app updates the client when `CLIENT_COMMIT` moves.
fn client(stack: &Path) -> Command {
    let mut cmd = host::command(&program(stack).to_string_lossy());
    cmd.current_dir(dir(stack))
        .env("FFXI_CONFIG", config_path(stack))
        .env("VANA_RELEASE_API", "https://api.github.com/repos/akreao/vana-client/releases/latest")
        .stdin(Stdio::null());
    cmd
}

/// Installs the game data from the official installer into `client/game/`.
/// It unpacks into `client/game/.staging` first, so it needs the room for
/// both on that drive for a while.
pub fn install(stack: &Path, installer: &Path) -> Command {
    let mut cmd = client(stack);
    cmd.arg("install").arg(installer).arg("--game-dir").arg(game_dir(stack));
    cmd
}

/// Brings the game data up to the current retail version from Square
/// Enix's patch server.
pub fn update(stack: &Path) -> Command {
    let mut cmd = client(stack);
    cmd.arg("update");
    cmd
}

/// Opens the game. It runs on its own: closing the app leaves it open.
/// Without game data, the client's first start offers to download and
/// install it from Square Enix, or to use game files already on disk.
pub fn play(stack: &Path) -> Result<(), String> {
    let mut cmd = client(stack);
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = cmd.spawn().map_err(|e| format!("Couldn't start vana-client: {e}"))?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("client-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_the_installer_in_the_server_folder_or_one_below() {
        let stack = temp("installer");
        assert!(find_installer(&stack).is_none());
        fs::create_dir_all(stack.join("downloads")).unwrap();
        fs::write(stack.join("downloads/FFXIFullSetup_US.part1.exe"), "").unwrap();
        fs::write(stack.join("downloads/FFXIFullSetup_US.part2.rar"), "").unwrap();
        assert_eq!(find_installer(&stack).unwrap(), stack.join("downloads/FFXIFullSetup_US.part1.exe"));
        // One right in the server folder comes first.
        fs::write(stack.join("ffxifullsetup_us.part1.exe"), "").unwrap();
        assert_eq!(find_installer(&stack).unwrap(), stack.join("ffxifullsetup_us.part1.exe"));
        fs::remove_dir_all(stack).unwrap();
    }

    #[test]
    fn config_keeps_the_client_s_own_settings() {
        let stack = temp("config");
        fs::create_dir_all(dir(&stack)).unwrap();
        fs::write(config_path(&stack), r#"{"server":"old","frame_limit":144,"installed":true,"logins":[{"username":"me"}]}"#).unwrap();
        write_config(&stack, "127.0.0.1").unwrap();
        let c = read_config(&stack);
        assert_eq!(c["server"], "127.0.0.1");
        assert_eq!(c["auth_port"], 54231);
        assert_eq!(c["game_dir"], json!(default_game_dir(&stack)));
        assert_eq!(c["frame_limit"], 144);
        assert_eq!(c["logins"][0]["username"], "me");

        // Not installed until the game files are there too.
        assert!(!status(&stack, "a").installed);
        fs::create_dir_all(game_dir(&stack).join("FINAL FANTASY XI")).unwrap();
        fs::write(game_dir(&stack).join("FINAL FANTASY XI/VTABLE.DAT"), "").unwrap();
        assert!(status(&stack, "a").installed);
        fs::remove_dir_all(stack).unwrap();
    }

    #[test]
    fn downloads_the_pinned_build_and_notices_a_new_pin() {
        let stack = temp("download");
        let commit = "8d7dd1ffb8d70924f5c40a82c376db4d6c0a9038";
        assert!(download_url(commit).starts_with(&format!("{DOWNLOADS}/client-8d7dd1ffb8d7/vana-client-")));
        assert!(download(&stack, "8d7dd1f").is_err());
        assert!(download(&stack, commit).is_ok());
        fs::write(partial(&stack), "binary").unwrap();
        finish_download(&stack, commit).unwrap();
        assert!(status(&stack, commit).program);
        assert!(!status(&stack, commit).outdated);
        assert!(status(&stack, "0123456789012345678901234567890123456789").outdated);
        fs::remove_dir_all(stack).unwrap();
    }

    // A self-signed certificate; its fingerprint is from
    // `openssl x509 -noout -fingerprint -sha256`.
    const CERT: &str = include_str!("../testdata/login.cert");
    const CERT_SHA256: &str = include_str!("../testdata/login.cert.sha256");

    #[test]
    fn pins_the_login_certificate() {
        let stack = temp("pin");
        // No certificate yet: nothing is written.
        write_config(&stack, "127.0.0.1").unwrap();
        assert!(!known_servers_path(&stack).exists());

        assert_eq!(cert_fingerprint(CERT).unwrap(), CERT_SHA256.trim());
        fs::write(stack.join(".env"), "DATA_DIR=./server-data\n").unwrap();
        fs::create_dir_all(stack.join("server-data/tls")).unwrap();
        fs::write(stack.join("server-data/tls/login.cert"), CERT).unwrap();
        fs::write(known_servers_path(&stack), r#"{"127.0.0.1:54231":{"sha256":"00","fixed":false},"other:54231":{"sha256":"11"}}"#).unwrap();
        write_config(&stack, "127.0.0.1").unwrap();
        let known: Value = serde_json::from_slice(&fs::read(known_servers_path(&stack)).unwrap()).unwrap();
        assert_eq!(known["127.0.0.1:54231"], json!({ "sha256": CERT_SHA256.trim(), "fixed": true }));
        assert_eq!(known["other:54231"]["sha256"], "11");

        assert!(cert_fingerprint("not a certificate").is_none());
        fs::remove_dir_all(stack).unwrap();
    }

    #[test]
    fn keeps_the_game_folder_the_client_picked() {
        let stack = temp("picked");
        fs::create_dir_all(dir(&stack)).unwrap();
        fs::write(config_path(&stack), r#"{"game_dir":"/games/ffxi","installed":true}"#).unwrap();
        write_config(&stack, "127.0.0.1").unwrap();
        assert_eq!(read_config(&stack)["game_dir"], "/games/ffxi");
        assert_eq!(game_dir(&stack), PathBuf::from("/games/ffxi"));
        fs::remove_dir_all(stack).unwrap();
    }

    #[test]
    fn a_broken_config_starts_over() {
        let stack = temp("broken");
        fs::create_dir_all(dir(&stack)).unwrap();
        fs::write(config_path(&stack), "not json").unwrap();
        write_config(&stack, "192.168.1.20").unwrap();
        assert_eq!(read_config(&stack)["server"], "192.168.1.20");
        fs::remove_dir_all(stack).unwrap();
    }
}
