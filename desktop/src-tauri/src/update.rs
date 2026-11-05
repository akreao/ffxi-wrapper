//! The app updating itself from this repository's GitHub releases (tagged
//! `vX.Y.Z`, made by the Release workflow). Tauri's updater reads
//! `latest.json` from the newest release, and installs a newer version only
//! if its file is signed with the key whose public half is in
//! `tauri.conf.json` and the signature names that version, so an old signed
//! build can't be passed off as new. On Linux it replaces the AppImage and
//! restarts the app; on Windows it runs the new installer, which restarts it.
//! The server keeps running either way.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

/// The update the last check found, kept for Install.
#[derive(Default)]
pub struct Pending(Mutex<Option<Update>>);

#[derive(Serialize)]
pub struct Info {
    /// This copy's version.
    pub current: String,
    /// The newer version, if there is one.
    pub version: Option<String>,
    /// Its release notes.
    pub notes: Option<String>,
}

/// Why this copy can't update itself, if it can't: a build without the
/// signing key's public half, or (on Linux) one not run as an AppImage.
pub fn unsupported(app: &AppHandle) -> Option<String> {
    let pubkey = app.config().plugins.0.get("updater").and_then(|u| u.get("pubkey")).and_then(|k| k.as_str()).unwrap_or("");
    if pubkey.trim().is_empty() {
        return Some("This build of the app can't update itself. Download new versions from the releases page.".into());
    }
    if cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_none() {
        return Some(
            "Only the AppImage updates itself. This copy was installed another way, so update it the same way.".into(),
        );
    }
    None
}

#[tauri::command]
pub async fn check_update(app: AppHandle, pending: State<'_, Pending>) -> Result<Info, String> {
    let current = app.package_info().version.to_string();
    if let Some(why) = unsupported(&app) {
        return Err(why);
    }
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| format!("Couldn't check for a new version: {e}"))?;
    let info = Info {
        current,
        version: update.as_ref().map(|u| u.version.clone()),
        notes: update.as_ref().and_then(|u| u.body.clone()).filter(|n| !n.trim().is_empty()),
    };
    *pending.0.lock().unwrap() = update;
    Ok(info)
}

#[derive(Clone, Serialize)]
struct Progress {
    done: usize,
    total: Option<u64>,
}

/// Downloads the update the last check found, checks its signature, installs
/// it and restarts the app. Progress goes out as `update-progress` events.
#[tauri::command]
pub async fn install_update(app: AppHandle, pending: State<'_, Pending>) -> Result<(), String> {
    let Some(update) = pending.0.lock().unwrap().take() else {
        return Err("Check for a new version first.".into());
    };
    let mut done = 0;
    let events = app.clone();
    update
        .download_and_install(
            |chunk, total| {
                done += chunk;
                let _ = events.emit("update-progress", Progress { done, total });
            },
            || {},
        )
        .await
        .map_err(|e| format!("Couldn't install version {}: {e}", update.version))?;
    // Windows doesn't get here: the installer has closed the app and opens
    // the new version itself.
    app.restart();
}

/// Registers the updater and what its commands share.
pub fn plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R, tauri_plugin_updater::Config> {
    tauri_plugin_updater::Builder::new().build()
}

pub fn manage(app: &tauri::App) {
    app.manage(Pending::default());
}
