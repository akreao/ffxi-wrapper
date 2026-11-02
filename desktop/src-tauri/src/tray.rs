//! The tray icon. Closing the window leaves the app here; Start and Stop
//! go through the window's own buttons, so their output shows in its
//! Activity pane as if they were clicked there.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};

pub const ID: &str = "main";

pub fn add(app: &tauri::App) -> tauri::Result<()> {
    let item = |id: &str, text: &str| MenuItem::with_id(app, id, text, true, None::<&str>);
    let menu = Menu::with_items(
        app,
        &[
            &item("open", "Open ffxi-wrapper")?,
            &PredefinedMenuItem::separator(app)?,
            &item("start", "Start server")?,
            &item("stop", "Stop server")?,
            &item("play", "Play")?,
            &PredefinedMenuItem::separator(app)?,
            &item("quit", "Quit (the server keeps running)")?,
        ],
    )?;
    let mut tray = TrayIconBuilder::with_id(ID)
        .tooltip("ffxi-wrapper")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|handle, event| match event.id().as_ref() {
            "open" => show_main(handle),
            "play" => crate::play_from_tray(handle),
            "start" | "stop" => {
                let _ = handle.emit("tray", event.id().as_ref());
            }
            "quit" => handle.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Brings the main window forward, from the tray or a second launch.
pub fn show_main(handle: &AppHandle) {
    if let Some(window) = handle.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}
