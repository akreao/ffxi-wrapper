//! Programs the app starts on the host: Docker, and the file manager or
//! browser that opens a folder or link.
//!
//! In the AppImage, its launcher points `LD_LIBRARY_PATH`, `XDG_DATA_DIRS`
//! and the like at the libraries bundled inside it, which the host's programs
//! don't want: a file manager started with them loads the wrong GTK and
//! fails. So every program the app starts gets the session's environment,
//! with the AppImage's entries taken out again. On Windows, programs start
//! without a console window of their own.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{Command, Stdio};

/// Set by the AppImage's launcher but not paths inside it.
const LAUNCHER_ONLY: [&str; 4] = ["GTK_THEME", "APPIMAGE", "ARGV0", "OWD"];

/// A command for a program on the host.
pub fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    if let Some(appdir) = std::env::var_os("APPDIR") {
        for (key, value) in host_env(Path::new(&appdir), std::env::vars_os()) {
            match value {
                Some(value) => cmd.env(key, value),
                None => cmd.env_remove(key),
            };
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// What to change in the app's environment for a host program: each
/// variable naming a path inside the AppImage loses those entries (and is
/// removed if nothing is left), and the launcher's own settings go.
fn host_env(
    appdir: &Path,
    vars: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, Option<OsString>)> {
    let mut changes = Vec::new();
    for (key, value) in vars {
        if LAUNCHER_ONLY.iter().any(|k| OsStr::new(k) == key) {
            changes.push((key, None));
            continue;
        }
        let entries: Vec<_> = std::env::split_paths(&value).collect();
        if !entries.iter().any(|e| e.starts_with(appdir)) {
            continue;
        }
        let kept: Vec<_> = entries
            .into_iter()
            .filter(|e| !e.starts_with(appdir) && !e.as_os_str().is_empty())
            .collect();
        let value = if kept.is_empty() { None } else { std::env::join_paths(kept).ok() };
        changes.push((key, value));
    }
    changes
}

/// Opens a folder or web address with the desktop's default program.
pub fn open(target: &OsStr) -> Result<(), String> {
    #[cfg(windows)]
    let mut cmd = {
        let mut cmd = command("explorer");
        cmd.arg(target);
        cmd
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut cmd = command("xdg-open");
        cmd.arg(target);
        cmd
    };
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Couldn't open {}: {e}", target.to_string_lossy()))?;
    // Reaped in the background: the opener can wait on the program it starts.
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(key: &str, value: Option<&str>) -> (OsString, Option<OsString>) {
        (key.into(), value.map(OsString::from))
    }

    #[cfg(unix)]
    #[test]
    fn takes_the_appimage_out_of_the_environment() {
        let vars = [
            ("PATH", "/tmp/.mount_ffxi1/usr/bin/:/usr/local/bin:/usr/bin"),
            ("LD_LIBRARY_PATH", "/tmp/.mount_ffxi1/usr/lib/:"),
            ("APPIMAGE", "/home/me/ffxi-wrapper.AppImage"),
            ("HOME", "/home/me"),
            ("OTHER", "/tmp/.mount_ffxi12/bin"),
        ]
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        assert_eq!(
            host_env(Path::new("/tmp/.mount_ffxi1"), vars),
            vec![
                change("PATH", Some("/usr/local/bin:/usr/bin")),
                change("LD_LIBRARY_PATH", None),
                change("APPIMAGE", None),
            ]
        );
    }
}
