//! Commands invoked from the UI. The UI only ever passes file *ids*; paths come
//! from the index, so the webview cannot ask us to touch arbitrary files.

use crate::state::AppState;
use serde::Serialize;
use shotfind_core::config::{default_folders, Folder, Settings};
use shotfind_core::db::FileRow;
use shotfind_core::indexer::Status;
use shotfind_core::models::{self, ModelId, ModelStatus};
use shotfind_core::search::{Query, SearchResults};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_opener::OpenerExt;

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

fn file_of(st: &AppState, id: i64) -> CmdResult<FileRow> {
    st.engine.file(id).map_err(err)?.ok_or_else(|| "That image is no longer in the index.".to_string())
}

#[tauri::command]
pub async fn search(st: State<'_, AppState>, query: Query) -> CmdResult<SearchResults> {
    let engine = st.engine.clone();
    blocking(move || engine.search(&query)).await
}

#[tauri::command]
pub fn status(st: State<'_, AppState>) -> Status {
    st.engine.status()
}

#[derive(Serialize)]
pub struct SettingsView {
    settings: Settings,
    defaults: Vec<Folder>,
    data_dir: String,
}

#[tauri::command]
pub fn get_settings(st: State<'_, AppState>) -> SettingsView {
    SettingsView {
        settings: st.engine.settings(),
        defaults: default_folders(),
        data_dir: st.engine.appdata().root().display().to_string(),
    }
}

/// Runs blocking engine work off the main thread (Tauri runs sync commands on
/// it, and the indexer's database may be busy with a long folder scan).
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(err)?.map_err(err)
}

#[tauri::command]
pub async fn save_settings(app: AppHandle, st: State<'_, AppState>, settings: Settings) -> CmdResult<()> {
    let old = st.engine.settings();
    // Folders must exist; store them canonicalised so prefixes compare reliably.
    let mut s = settings;
    for f in &mut s.folders {
        if let Ok(c) = shotfind_core::appdata::dunce_canonicalize(&f.path) {
            f.path = c;
        }
    }
    s.folders.dedup_by(|a, b| a.path == b.path);
    let (engine, s2) = (st.engine.clone(), s.clone());
    blocking(move || engine.update_settings(s2)).await?;
    if old.hotkey != s.hotkey {
        crate::hotkey::register(&app, &s.hotkey).map_err(|e| format!("Couldn't use that shortcut: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn set_hotkey(app: AppHandle, st: State<'_, AppState>, hotkey: String) -> CmdResult<()> {
    crate::hotkey::register(&app, &hotkey).map_err(|e| format!("Couldn't use that shortcut: {e}"))?;
    let mut s = st.engine.settings();
    s.hotkey = hotkey;
    let engine = st.engine.clone();
    blocking(move || engine.update_settings(s)).await
}

#[tauri::command]
pub fn pause_indexing(st: State<'_, AppState>) {
    st.engine.indexer().pause();
}

#[tauri::command]
pub fn resume_indexing(st: State<'_, AppState>) {
    st.engine.indexer().resume();
}

#[tauri::command]
pub fn rescan(st: State<'_, AppState>) {
    st.engine.indexer().rescan();
}

#[tauri::command]
pub async fn retry_failed(st: State<'_, AppState>) -> CmdResult<usize> {
    let engine = st.engine.clone();
    blocking(move || engine.indexer().retry_failed()).await
}

#[tauri::command]
pub async fn wipe_index(st: State<'_, AppState>) -> CmdResult<()> {
    let engine = st.engine.clone();
    blocking(move || engine.indexer().wipe()).await
}

#[derive(Serialize)]
pub struct ModelsView {
    models: Vec<ModelStatus>,
    consented: bool,
    downloading: bool,
    runtime_error: Option<String>,
}

#[tauri::command]
pub fn models_status(st: State<'_, AppState>) -> ModelsView {
    ModelsView {
        models: st.engine.models(),
        consented: st.engine.settings().models_consented,
        downloading: st.downloading.load(Ordering::SeqCst),
        runtime_error: st.engine.runtime_error().map(|s| s.to_string()),
    }
}

/// Starts the one-time download (after the user agreed on the consent screen).
#[tauri::command]
pub async fn download_models(app: AppHandle, st: State<'_, AppState>, ids: Vec<ModelId>) -> CmdResult<()> {
    if st.downloading.swap(true, Ordering::SeqCst) {
        return Err("A download is already running.".into());
    }
    let mut s = st.engine.settings();
    s.models_consented = true;
    let engine = st.engine.clone();
    if let Err(e) = blocking(move || engine.update_settings(s)).await {
        st.downloading.store(false, Ordering::SeqCst);
        return Err(e);
    }
    st.download_cancel.store(false, Ordering::SeqCst);
    let (engine, cancel, flag) = (st.engine.clone(), st.download_cancel.clone(), st.downloading.clone());
    std::thread::spawn(move || {
        let emit = app.clone();
        let r = models::install(engine.appdata(), &ids, &cancel, &mut |p| {
            let _ = emit.emit("model-progress", p);
        });
        flag.store(false, Ordering::SeqCst);
        let result = r.and_then(|_| engine.reload_models());
        let _ = app.emit("models-changed", result.err().map(|e| format!("{e:#}")));
    });
    Ok(())
}

#[tauri::command]
pub fn cancel_download(st: State<'_, AppState>) {
    st.download_cancel.store(true, Ordering::SeqCst);
}

/// Offline install from a folder the user picked.
#[tauri::command]
pub async fn import_models(st: State<'_, AppState>, dir: String) -> CmdResult<Vec<ModelId>> {
    let engine = st.engine.clone();
    blocking(move || {
        let got = models::import_from_dir(engine.appdata(), Path::new(&dir))?;
        if got.is_empty() {
            anyhow::bail!("No matching model files found in that folder (names and SHA-256 must match).");
        }
        let mut s = engine.settings();
        s.models_consented = true;
        engine.update_settings(s)?;
        engine.reload_models()?;
        Ok(got)
    })
    .await
}

/// "Not now": index file names only; the consent screen is not shown again.
#[tauri::command]
pub async fn skip_models(st: State<'_, AppState>) -> CmdResult<()> {
    let mut s = st.engine.settings();
    s.models_consented = true;
    let engine = st.engine.clone();
    blocking(move || engine.update_settings(s)).await
}

#[tauri::command]
pub fn open_file(app: AppHandle, st: State<'_, AppState>, id: i64) -> CmdResult<()> {
    let f = file_of(&st, id)?;
    app.opener().open_path(f.path, None::<&str>).map_err(err)
}

#[tauri::command]
pub fn reveal_file(app: AppHandle, st: State<'_, AppState>, id: i64) -> CmdResult<()> {
    let f = file_of(&st, id)?;
    app.opener().reveal_item_in_dir(PathBuf::from(f.path)).map_err(err)
}

#[tauri::command]
pub async fn copy_image(app: AppHandle, st: State<'_, AppState>, id: i64) -> CmdResult<()> {
    let f = file_of(&st, id)?;
    let max = st.engine.settings().max_file_bytes;
    let (rgba, w, h) = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<(Vec<u8>, u32, u32)> {
        let bytes = shotfind_core::userfs::read_limited(Path::new(&f.path), max)?;
        let img = image::load_from_memory(&bytes)?.into_rgba8();
        let (w, h) = img.dimensions();
        Ok((img.into_raw(), w, h))
    })
    .await
    .map_err(err)?
    .map_err(err)?;
    app.clipboard().write_image(&tauri::image::Image::new_owned(rgba, w, h)).map_err(err)
}

#[tauri::command]
pub fn copy_text(app: AppHandle, st: State<'_, AppState>, id: i64) -> CmdResult<usize> {
    let text = st.engine.ocr_text(id).map_err(err)?.unwrap_or_default();
    if text.trim().is_empty() {
        return Err("No text was found in this image.".into());
    }
    let n = text.chars().count();
    app.clipboard().write_text(text).map_err(err)?;
    Ok(n)
}

#[derive(Serialize)]
pub struct FileDetails {
    file: FileRow,
    text: String,
}

#[tauri::command]
pub fn file_details(st: State<'_, AppState>, id: i64) -> CmdResult<FileDetails> {
    let file = file_of(&st, id)?;
    let text = st.engine.ocr_text(id).map_err(err)?.unwrap_or_default();
    Ok(FileDetails { file, text })
}

#[tauri::command]
pub async fn folders_in_index(st: State<'_, AppState>) -> CmdResult<Vec<(String, u64)>> {
    let engine = st.engine.clone();
    blocking(move || engine.searcher().folders()).await
}

#[tauri::command]
pub fn hide_bar(app: AppHandle) {
    if let Some(w) = app.get_webview_window("bar") {
        let _ = w.hide();
    }
}

/// Opens the main window, optionally with a query (from the quick bar).
#[tauri::command]
pub fn show_main_window(app: AppHandle, query: Option<String>) {
    hide_bar(app.clone());
    crate::show_main(&app);
    if let Some(q) = query {
        let _ = app.emit_to("main", "set-query", q);
    }
}

#[derive(Serialize)]
pub struct AppInfo {
    version: &'static str,
    os: &'static str,
    onnxruntime: Option<String>,
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        onnxruntime: shotfind_core::runtime::loaded_from().map(|p| p.display().to_string()),
    }
}
