//! Walks the configured folders and records image files in the database.
//! Read-only: only directory listings and metadata are touched.

use crate::appdata::AppData;
use crate::config::{has_image_extension, looks_like_screenshot_name, Settings};
use crate::db::{Db, ScannedFile, Upsert};
use crate::userfs;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use walkdir::{DirEntry, WalkDir};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ScanStats {
    pub seen: u64,
    pub new: u64,
    pub changed: u64,
    pub removed: u64,
    pub skipped: u64,
}

impl std::ops::AddAssign for ScanStats {
    fn add_assign(&mut self, o: Self) {
        self.seen += o.seen;
        self.new += o.new;
        self.changed += o.changed;
        self.removed += o.removed;
        self.skipped += o.skipped;
    }
}

/// Directory names never descended into.
const SKIP_DIRS: &[&str] =
    &["node_modules", "$recycle.bin", "system volume information", "__pycache__", "appdata", "thumbs.db"];

/// Directory suffixes (macOS bundles, catalog caches) that contain derived or
/// private copies of images rather than the user's own files.
const SKIP_DIR_SUFFIXES: &[&str] = &[".photoslibrary", ".lrdata", ".app", ".photolibrary", ".aplibrary"];

pub fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// The most specific enabled folder that contains `path`.
pub fn owning_root(settings: &Settings, path: &Path) -> Option<PathBuf> {
    settings
        .enabled_folders()
        .filter(|f| path.starts_with(&f.path))
        .max_by_key(|f| f.path.components().count())
        .map(|f| f.path.clone())
}

fn is_skipped_dir(e: &DirEntry, root: &Path, other_roots: &[PathBuf], appdata: Option<&AppData>) -> bool {
    if e.depth() == 0 {
        return false;
    }
    let name = e.file_name().to_string_lossy().to_lowercase();
    if name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
        return true;
    }
    if SKIP_DIR_SUFFIXES.iter().any(|s| name.ends_with(s)) {
        return true;
    }
    let p = e.path();
    // Nested enabled folders are scanned on their own (as their own root).
    if other_roots.iter().any(|r| r != root && r.starts_with(root) && p == r.as_path()) {
        return true;
    }
    appdata.is_some_and(|a| a.contains(p))
}

/// Scans one folder; adds new files, flags changed ones, removes deleted ones.
/// Returns stats and the ids of removed rows (for thumbnail cleanup).
pub fn scan_folder(
    db: &mut Db,
    settings: &Settings,
    root: &Path,
    epoch: i64,
    appdata: Option<&AppData>,
    cancel: &AtomicBool,
) -> Result<(ScanStats, Vec<i64>)> {
    let mut stats = ScanStats::default();
    let root_s = path_str(root);
    if !root.is_dir() {
        // Folder vanished (unplugged drive?) — keep the index; don't treat as deletion.
        tracing::warn!("folder not available, skipping: {}", root.display());
        return Ok((stats, Vec::new()));
    }
    let other_roots: Vec<PathBuf> = settings.enabled_folders().map(|f| f.path.clone()).collect();
    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !(e.file_type().is_dir() && is_skipped_dir(e, root, &other_roots, appdata)));

    let mut batch: Vec<(PathBuf, userfs::FileStamp)> = Vec::with_capacity(512);
    let flush = |db: &mut Db, batch: &mut Vec<(PathBuf, userfs::FileStamp)>, stats: &mut ScanStats| -> Result<()> {
        db.tx(|tx| {
            for (p, st) in batch.iter() {
                let path = path_str(p);
                let dir = p.parent().map(path_str).unwrap_or_default();
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let is_ss = settings.is_screenshot_folder(p) || looks_like_screenshot_name(&name);
                let f = ScannedFile {
                    path: &path,
                    root: &root_s,
                    dir: &dir,
                    name: &name,
                    size: st.size,
                    mtime_ms: st.mtime_ms,
                    is_screenshot: is_ss,
                };
                match Db::upsert_scanned(tx, &f, epoch)? {
                    Upsert::New(_) => stats.new += 1,
                    Upsert::Changed(_) => stats.changed += 1,
                    Upsert::Unchanged(_) => {}
                }
            }
            Ok(())
        })?;
        batch.clear();
        Ok(())
    };

    for entry in walker {
        if cancel.load(Ordering::Relaxed) {
            // Partial scan: don't remove anything we didn't get to see.
            flush(db, &mut batch, &mut stats)?;
            return Ok((stats, Vec::new()));
        }
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::debug!("walk error: {e}");
                continue;
            }
        };
        if !entry.file_type().is_file() || !has_image_extension(entry.path()) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        stats.seen += 1;
        let st = userfs::stamp(&meta);
        if st.size < settings.min_file_bytes || st.size > settings.max_file_bytes {
            stats.skipped += 1;
            continue;
        }
        batch.push((entry.into_path(), st));
        if batch.len() >= 500 {
            flush(db, &mut batch, &mut stats)?;
        }
    }
    flush(db, &mut batch, &mut stats)?;
    let removed = db.tx(|tx| Db::remove_unseen(tx, &root_s, epoch))?;
    stats.removed = removed.len() as u64;
    Ok((stats, removed))
}

/// Applies a single file-system change (from the watcher). Returns removed ids.
pub fn apply_path_change(db: &mut Db, settings: &Settings, path: &Path, epoch: i64) -> Result<(bool, Vec<i64>)> {
    let path_s = path_str(path);
    let Some(root) = owning_root(settings, path) else {
        return Ok((false, Vec::new()));
    };
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => {
            if !has_image_extension(path) {
                return Ok((false, Vec::new()));
            }
            let st = userfs::stamp(&meta);
            if st.size < settings.min_file_bytes || st.size > settings.max_file_bytes {
                // Possibly a file that grew past the limit: drop any stale row.
                let removed = db.tx(|tx| Db::remove_path(tx, &path_s))?;
                return Ok((false, removed));
            }
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let dir = path.parent().map(path_str).unwrap_or_default();
            let root_s = path_str(&root);
            let f = ScannedFile {
                path: &path_s,
                root: &root_s,
                dir: &dir,
                name: &name,
                size: st.size,
                mtime_ms: st.mtime_ms,
                is_screenshot: settings.is_screenshot_folder(path) || looks_like_screenshot_name(&name),
            };
            let up = db.tx(|tx| Db::upsert_scanned(tx, &f, epoch))?;
            Ok((!matches!(up, Upsert::Unchanged(_)), Vec::new()))
        }
        Ok(meta) if meta.is_dir() => {
            // A directory appeared (moved in, or unzipped). Scan it without
            // deleting anything else under the root.
            let mut changed = false;
            let walker = WalkDir::new(path).follow_links(false).into_iter().filter_entry(|e| {
                !(e.file_type().is_dir() && e.depth() > 0 && e.file_name().to_string_lossy().starts_with('.'))
            });
            for e in walker.flatten() {
                if e.file_type().is_file() {
                    let (c, _) = apply_path_change(db, settings, e.path(), epoch)?;
                    changed |= c;
                }
            }
            Ok((changed, Vec::new()))
        }
        _ => {
            // Gone: a file or a whole directory was deleted / moved away.
            let removed = db.tx(|tx| Db::remove_path(tx, &path_s))?;
            Ok((false, removed))
        }
    }
}
