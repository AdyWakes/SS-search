//! File-system watcher: turns OS notifications into index updates.

use anyhow::Result;
use crossbeam_channel::Sender;
use notify::{EventKind, RecursiveMode};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// Path created or modified (file or directory).
    Changed(PathBuf),
    /// Path removed or moved away.
    Removed(PathBuf),
    /// Events were dropped; a full rescan is needed.
    Rescan,
}

pub struct FolderWatcher {
    _debouncer: Debouncer<notify::RecommendedWatcher, RecommendedCache>,
    pub watching: Vec<PathBuf>,
}

/// Watches `folders` recursively. Screenshot tools often write a file in several
/// steps, so events are debounced for 1.5 s before being forwarded.
pub fn watch(folders: &[PathBuf], tx: Sender<WatchEvent>) -> Result<FolderWatcher> {
    let mut debouncer = new_debouncer(Duration::from_millis(1500), None, move |res: DebounceEventResult| match res {
        Ok(events) => {
            for ev in events {
                if ev.need_rescan() {
                    let _ = tx.send(WatchEvent::Rescan);
                    continue;
                }
                for e in map_event(&ev.event) {
                    let _ = tx.send(e);
                }
            }
        }
        Err(errs) => {
            for e in errs {
                tracing::warn!("watch error: {e}");
            }
            let _ = tx.send(WatchEvent::Rescan);
        }
    })?;
    let mut watching = Vec::new();
    for f in folders {
        if !f.is_dir() {
            continue;
        }
        match debouncer.watch(f, RecursiveMode::Recursive) {
            Ok(()) => watching.push(f.clone()),
            Err(e) => tracing::warn!("cannot watch {}: {e}", f.display()),
        }
    }
    Ok(FolderWatcher { _debouncer: debouncer, watching })
}

pub fn map_event(ev: &notify::Event) -> Vec<WatchEvent> {
    use notify::event::{ModifyKind, RenameMode};
    let paths = &ev.paths;
    match &ev.kind {
        EventKind::Create(_) => paths.iter().cloned().map(WatchEvent::Changed).collect(),
        EventKind::Remove(_) => paths.iter().cloned().map(WatchEvent::Removed).collect(),
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
            paths.iter().cloned().map(WatchEvent::Removed).collect()
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => paths.iter().cloned().map(WatchEvent::Changed).collect(),
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if paths.len() == 2 => {
            vec![WatchEvent::Removed(paths[0].clone()), WatchEvent::Changed(paths[1].clone())]
        }
        // Rename of unknown direction: let the handler stat the path to decide.
        EventKind::Modify(ModifyKind::Name(_)) => paths.iter().cloned().map(WatchEvent::Changed).collect(),
        EventKind::Modify(ModifyKind::Metadata(_)) => Vec::new(),
        EventKind::Modify(_) => paths.iter().cloned().map(WatchEvent::Changed).collect(),
        _ => Vec::new(),
    }
}
