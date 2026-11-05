//! shotfind desktop shell: windows, global hotkey, tray, and the commands the
//! UI calls. All indexing/search logic lives in `shotfind-core`.

mod commands;
mod hotkey;
mod state;

use state::AppState;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::http::{header, Response, StatusCode};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager, RunEvent, WindowEvent};

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,ort=warn,tao=warn,wry=warn".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .register_asynchronous_uri_scheme_protocol("thumb", |ctx, req, responder| {
            let app = ctx.app_handle().clone();
            let path = req.uri().path().to_string();
            std::thread::spawn(move || responder.respond(serve_image(&app, &path)));
        })
        .invoke_handler(tauri::generate_handler![
            commands::search,
            commands::status,
            commands::get_settings,
            commands::save_settings,
            commands::pause_indexing,
            commands::resume_indexing,
            commands::rescan,
            commands::retry_failed,
            commands::wipe_index,
            commands::models_status,
            commands::download_models,
            commands::cancel_download,
            commands::import_models,
            commands::skip_models,
            commands::open_file,
            commands::reveal_file,
            commands::copy_image,
            commands::copy_text,
            commands::file_details,
            commands::folders_in_index,
            commands::set_hotkey,
            commands::hide_bar,
            commands::show_main_window,
            commands::app_info,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let data_dir = app.path().app_local_data_dir()?;
            let engine = shotfind_core::engine::Engine::open(data_dir, &ort_candidates(app))?;
            let emitter = handle.clone();
            engine.set_listener(Box::new(move |st| {
                let _ = emitter.emit("index-status", st);
            }));
            let hotkey = engine.settings().hotkey.clone();
            app.manage(AppState::new(Arc::new(engine)));
            if let Err(e) = hotkey::register(&handle, &hotkey) {
                tracing::warn!("could not register hotkey {hotkey}: {e}");
            }
            build_tray(app)?;
            if !std::env::args().any(|a| a == "--hidden") {
                show_main(&handle);
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // Closing the main window keeps shotfind in the tray (indexing + hotkey).
            WindowEvent::CloseRequested { api, .. } if window.label() == "main" => {
                api.prevent_close();
                let _ = window.hide();
            }
            WindowEvent::Focused(false) if window.label() == "bar" => {
                let _ = window.hide();
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("error while building shotfind")
        .run(|_app, event| {
            if let RunEvent::ExitRequested { api, code, .. } = event {
                // Only an explicit Quit (code set) exits; closing windows doesn't.
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}

/// Where the bundled ONNX Runtime library may be.
fn ort_candidates(app: &tauri::App) -> Vec<PathBuf> {
    let name = shotfind_core::runtime::dylib_name();
    let mut c = Vec::new();
    if let Ok(res) = app.path().resource_dir() {
        c.push(res.join("onnxruntime").join(name));
        c.push(res.join("resources").join("onnxruntime").join(name));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            c.push(dir.join(name));
            c.push(dir.join("onnxruntime").join(name));
        }
    }
    // `cargo tauri dev`: the copy fetched by scripts/fetch-onnxruntime.mjs.
    c.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join("onnxruntime").join(name));
    c
}

pub(crate) fn show_main(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let search = MenuItem::with_id(app, "search", "Quick search", true, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Open shotfind", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "Pause / resume indexing", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit shotfind", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&search, &open, &pause, &sep, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main").tooltip("shotfind").menu(&menu).on_menu_event(|app, ev| {
        match ev.id().as_ref() {
            "search" => hotkey::toggle_bar(app),
            "open" => show_main(app),
            "pause" => {
                if let Some(st) = app.try_state::<AppState>() {
                    let ix = st.engine.indexer();
                    if ix.is_paused() {
                        ix.resume()
                    } else {
                        ix.pause()
                    }
                }
            }
            "quit" => app.exit(0),
            _ => {}
        }
    });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// `thumb://localhost/<id>` -> cached thumbnail; `thumb://localhost/full/<id>`
/// -> the original image (read-only). Only indexed ids resolve, so the UI can't
/// use this to read arbitrary files.
fn serve_image(app: &tauri::AppHandle, path: &str) -> Response<Vec<u8>> {
    let not_found = || Response::builder().status(StatusCode::NOT_FOUND).body(Vec::new()).unwrap();
    let Some(st) = app.try_state::<AppState>() else { return not_found() };
    let trimmed = path.trim_start_matches('/');
    let (full, id_str) = match trimmed.strip_prefix("full/") {
        Some(rest) => (true, rest),
        None => (false, trimmed),
    };
    let Ok(id) = id_str.split(['?', '.']).next().unwrap_or("").parse::<i64>() else { return not_found() };
    let (bytes, mime) = if full {
        let Ok(Some(f)) = st.engine.file(id) else { return not_found() };
        let max = st.engine.settings().max_file_bytes;
        match shotfind_core::userfs::read_limited(std::path::Path::new(&f.path), max) {
            Ok(b) => (b, mime_for(&f.path)),
            Err(_) => return not_found(),
        }
    } else {
        match std::fs::read(st.engine.thumb_path(id)) {
            Ok(b) => (b, "image/jpeg"),
            Err(_) => return not_found(),
        }
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, if full { "no-cache" } else { "max-age=31536000, immutable" })
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(bytes)
        .unwrap()
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next().map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        _ => "application/octet-stream",
    }
}
