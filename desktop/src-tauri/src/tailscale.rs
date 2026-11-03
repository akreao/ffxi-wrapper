//! Tailscale, for players on other networks: whether it's installed and
//! signed in on this computer, and its address there. Tailscale carries the
//! server's TCP and UDP ports as they are, so players who join this
//! computer's tailnet (or get it shared with them) reach it like a LAN.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::{host, net};

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// No `tailscale` command and no Tailscale address.
    Missing,
    /// Installed, but its service isn't running or the connection is off.
    Stopped,
    /// Installed and running, but not signed in.
    NeedsLogin,
    /// Connected; `ip` is set.
    Running,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Status {
    pub state: State,
    /// This computer's Tailscale IPv4 address.
    pub ip: Option<String>,
    /// Its MagicDNS name, such as `pc.tail1234.ts.net`.
    pub dns_name: Option<String>,
    /// The tailnet's name, as Tailscale shows it.
    pub tailnet: Option<String>,
}

/// Asks the `tailscale` command, or with no command, looks for an
/// interface with a Tailscale address.
pub fn status() -> Status {
    match run_status() {
        Some(Ok(json)) => parse_status(&json),
        // The command is there but its service isn't.
        Some(Err(())) => Status { state: State::Stopped, ip: None, dns_name: None, tailnet: None },
        None => match interface_ip() {
            Some(ip) => Status { state: State::Running, ip: Some(ip), dns_name: None, tailnet: None },
            None => Status { state: State::Missing, ip: None, dns_name: None, tailnet: None },
        },
    }
}

fn interface_ip() -> Option<String> {
    net::lan_addresses().into_iter().find(|a| a.vpn == Some("Tailscale")).map(|a| a.ip)
}

/// Where the command is: on the PATH, or where the Windows installer puts it.
fn program() -> PathBuf {
    #[cfg(windows)]
    {
        let base = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| r"C:\Program Files".into());
        let installed = base.join("Tailscale").join("tailscale.exe");
        if installed.exists() {
            return installed;
        }
    }
    PathBuf::from("tailscale")
}

/// `tailscale status --json`'s output; `None` without the command,
/// `Err` when it fails (its service isn't running). Gives up after 5 s.
fn run_status() -> Option<Result<Vec<u8>, ()>> {
    let mut child = host::command(&program().to_string_lossy())
        .args(["status", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut out);
        out
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let ok = loop {
        match child.try_wait() {
            Ok(Some(exit)) => break exit.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let out = reader.join().unwrap_or_default();
    Some(if ok { Ok(out) } else { Err(()) })
}

fn parse_status(json: &[u8]) -> Status {
    let v: Value = serde_json::from_slice(json).unwrap_or(Value::Null);
    let text = |v: &Value| v.as_str().map(str::to_owned).filter(|s| !s.is_empty());
    let ip = v["Self"]["TailscaleIPs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find(|ip| ip.parse::<std::net::Ipv4Addr>().is_ok())
        .map(str::to_owned);
    let state = match v["BackendState"].as_str() {
        Some("Running") if ip.is_some() => State::Running,
        Some("NeedsLogin" | "NeedsMachineAuth") => State::NeedsLogin,
        _ => State::Stopped,
    };
    let running = state == State::Running;
    Status {
        ip: ip.filter(|_| running),
        dns_name: text(&v["Self"]["DNSName"]).map(|n| n.trim_end_matches('.').to_owned()).filter(|_| running),
        tailnet: text(&v["CurrentTailnet"]["Name"]).filter(|_| running),
        state,
    }
}

/// The Tailscale pages the app links to.
pub fn url(page: &str) -> Result<&'static str, String> {
    match page {
        "download" => Ok("https://tailscale.com/download"),
        "machines" => Ok("https://login.tailscale.com/admin/machines"),
        "sharing" => Ok("https://tailscale.com/kb/1084/sharing"),
        _ => Err(format!("No Tailscale page called {page}.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connected() {
        let json = br#"{"BackendState":"Running",
            "Self":{"HostName":"pc","DNSName":"pc.tail1234.ts.net.","TailscaleIPs":["fd7a:115c:a1e0::1","100.101.102.103"]},
            "CurrentTailnet":{"Name":"steve@example.com","MagicDNSSuffix":"tail1234.ts.net"}}"#;
        assert_eq!(
            parse_status(json),
            Status {
                state: State::Running,
                ip: Some("100.101.102.103".into()),
                dns_name: Some("pc.tail1234.ts.net".into()),
                tailnet: Some("steve@example.com".into()),
            }
        );
    }

    #[test]
    fn not_connected() {
        let login = parse_status(br#"{"BackendState":"NeedsLogin","Self":{"TailscaleIPs":null},"CurrentTailnet":null}"#);
        assert_eq!(login.state, State::NeedsLogin);
        assert_eq!(login.ip, None);
        // Switched off in the tray: the last address is still listed, but nobody can reach it.
        let stopped = parse_status(br#"{"BackendState":"Stopped","Self":{"TailscaleIPs":["100.101.102.103"]}}"#);
        assert_eq!(stopped.state, State::Stopped);
        assert_eq!(stopped.ip, None);
        assert_eq!(parse_status(b"not json").state, State::Stopped);
    }

    #[test]
    fn links() {
        assert!(url("download").unwrap().starts_with("https://tailscale.com/"));
        assert!(url("https://example.com").is_err());
    }
}
