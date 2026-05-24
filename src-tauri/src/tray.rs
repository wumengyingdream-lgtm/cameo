//! System-tray icon + "close to tray" support.
//!
//! When `config.close_to_tray` is on (default), closing the window hides it
//! instead of quitting; the app keeps running behind a tray icon. Left-click
//! the tray icon (or "Show Cameo") to restore the window; "Quit Cameo" exits.
//! The hide-vs-quit decision lives in `lib.rs`'s window-close handler (it reads
//! the config at close time) — this module owns only the tray itself and the
//! canonical "bring the window back" routine, reused by the macOS dock reopen.

use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};

const MENU_SHOW: &str = "tray_show";
const MENU_QUIT: &str = "tray_quit";

/// Build the tray icon + menu. Called once from `lib.rs`'s `.setup()`.
pub fn setup<R: Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    // Reuse the bundled app icon. On macOS this shows a small colored icon in
    // the menu bar (not a monochrome template) — fine for now; can be swapped
    // for a template glyph later without touching the close-to-tray logic.
    let Some(icon) = app.default_window_icon().cloned() else {
        tracing::warn!(module = "tray", "no default window icon; skipping tray");
        return Ok(());
    };

    let show = MenuItemBuilder::with_id(MENU_SHOW, "Show Cameo").build(app)?;
    let quit = MenuItemBuilder::with_id(MENU_QUIT, "Quit Cameo").build(app)?;
    let menu = MenuBuilder::new(app).item(&show).separator().item(&quit).build()?;

    TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .tooltip("Cameo")
        // Left-click restores the window (handled below); right-click opens menu.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_SHOW => show_main_window(app),
            MENU_QUIT => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    tracing::info!(module = "tray", "tray icon ready");
    Ok(())
}

/// Bring the main window back to the foreground (tray click, "Show Cameo", or
/// macOS dock reopen). Idempotent; safe if the window is already visible.
pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
