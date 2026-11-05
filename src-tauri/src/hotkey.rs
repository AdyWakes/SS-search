//! Global hotkey: toggles the Spotlight-style quick search bar.

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

pub fn register(app: &AppHandle, accelerator: &str) -> anyhow::Result<()> {
    let gs = app.global_shortcut();
    gs.unregister_all()?;
    if accelerator.trim().is_empty() {
        return Ok(());
    }
    gs.on_shortcut(accelerator, |app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
            toggle_bar(app);
        }
    })?;
    Ok(())
}

pub fn toggle_bar(app: &AppHandle) {
    let Some(bar) = app.get_webview_window("bar") else { return };
    if bar.is_visible().unwrap_or(false) {
        let _ = bar.hide();
        return;
    }
    // Open on the monitor the cursor is on, a third of the way down.
    if let Ok(Some(mon)) = bar.current_monitor() {
        let (sz, pos, scale) = (mon.size(), mon.position(), mon.scale_factor());
        let w = (760.0 * scale) as i32;
        let x = pos.x + (sz.width as i32 - w) / 2;
        let y = pos.y + (sz.height as f64 * 0.22) as i32;
        let _ = bar.set_position(tauri::PhysicalPosition::new(x, y));
    }
    let _ = bar.show();
    let _ = bar.set_focus();
    let _ = bar.emit("bar-opened", ());
}
