//! Background indexer: scans folders, follows the watcher, and runs the
//! per-file pipeline on low-priority worker threads. Pausable and resumable;
//! progress survives restarts (state lives in SQLite, committed per file).

use crate::appdata::AppData;
use crate::config::Settings;
use crate::db::{Counts, Db};
use crate::pipeline::{self, EngineProvider, Job, Outcome, WorkerEngines};
use crate::scanner::{self, path_str, ScanStats};
use crate::vectors::VectorIndex;
use crate::watcher::{self, FolderWatcher, WatchEvent};
use anyhow::Result;
use crossbeam_channel::{bounded, select, unbounded, Receiver, Sender};
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Starting,
    Scanning,
    Indexing,
    Idle,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub phase: Phase,
    pub counts: Counts,
    /// File currently being processed (display only).
    pub current: Option<String>,
    /// Files per minute over the last few dozen files.
    pub per_minute: f64,
    pub last_scan: Option<ScanStats>,
    pub ocr_available: bool,
    pub semantic_available: bool,
    pub watching: usize,
    pub last_error: Option<String>,
}

pub type Listener = Box<dyn Fn(&Status) + Send + Sync>;

enum Cmd {
    Wake,
    Rescan,
    Wipe(Sender<Result<()>>),
    Stop,
}

pub struct Shared {
    pub appdata: AppData,
    pub settings: RwLock<Settings>,
    pub vectors: Arc<RwLock<VectorIndex>>,
    db: Mutex<Db>,
    provider: RwLock<Arc<dyn EngineProvider>>,
    provider_gen: AtomicU64,
    paused: AtomicBool,
    cancel_scan: AtomicBool,
    status: Mutex<Status>,
    listener: RwLock<Option<Listener>>,
    cmd_tx: Sender<Cmd>,
}

pub struct Indexer {
    shared: Arc<Shared>,
    coordinator: Option<JoinHandle<()>>,
}

impl Indexer {
    /// Opens the index and starts background work (unless settings say paused).
    pub fn start(appdata: AppData, settings: Settings, provider: Arc<dyn EngineProvider>) -> Result<Indexer> {
        let db = Db::open(&appdata.db_path())?;
        let crashed = db.quarantine_crashers()?;
        if crashed > 0 {
            tracing::warn!("{crashed} file(s) crashed the indexer repeatedly; marked as failed");
        }
        let vectors = Arc::new(RwLock::new(VectorIndex::load(&db, &provider.embed_model_id())?));
        let (cmd_tx, cmd_rx) = unbounded();
        let status = Status {
            phase: Phase::Starting,
            counts: db.counts()?,
            current: None,
            per_minute: 0.0,
            last_scan: None,
            ocr_available: provider.ocr_version() > 0,
            semantic_available: provider.embed_version() > 0,
            watching: 0,
            last_error: None,
        };
        let paused = settings.paused;
        let shared = Arc::new(Shared {
            appdata,
            settings: RwLock::new(settings),
            vectors,
            db: Mutex::new(db),
            provider: RwLock::new(provider),
            provider_gen: AtomicU64::new(1),
            paused: AtomicBool::new(paused),
            cancel_scan: AtomicBool::new(false),
            status: Mutex::new(status),
            listener: RwLock::new(None),
            cmd_tx,
        });
        shared.requeue_outdated()?;
        let s2 = shared.clone();
        let coordinator = std::thread::Builder::new().name("shotfind-indexer".into()).spawn(move || {
            lower_thread_priority();
            // Supervisor: a transient failure (disk full, database busy)
            // restarts the loop after a pause instead of ending indexing.
            let mut failures = 0u32;
            loop {
                match coordinator_loop(&s2, &cmd_rx) {
                    Ok(()) => break,
                    Err(e) => {
                        failures += 1;
                        tracing::error!("indexer error (restart #{failures}): {e:#}");
                        s2.set_error(format!("{e:#}"));
                        let wait = Duration::from_secs((5 * failures as u64).min(120));
                        if let Ok(Cmd::Stop) = cmd_rx.recv_timeout(wait) {
                            break;
                        }
                    }
                }
            }
            s2.set_phase(Phase::Stopped);
        })?;
        Ok(Indexer { shared, coordinator: Some(coordinator) })
    }

    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    pub fn set_listener(&self, l: Listener) {
        *self.shared.listener.write() = Some(l);
    }

    pub fn status(&self) -> Status {
        self.shared.status.lock().clone()
    }

    pub fn pause(&self) {
        self.shared.paused.store(true, Ordering::SeqCst);
        self.shared.cancel_scan.store(true, Ordering::SeqCst);
        self.shared.persist_paused(true);
        self.shared.send(Cmd::Wake);
    }

    pub fn resume(&self) {
        self.shared.paused.store(false, Ordering::SeqCst);
        self.shared.persist_paused(false);
        self.shared.send(Cmd::Rescan);
    }

    pub fn is_paused(&self) -> bool {
        self.shared.paused.load(Ordering::SeqCst)
    }

    pub fn rescan(&self) {
        self.shared.send(Cmd::Rescan);
    }

    pub fn retry_failed(&self) -> Result<usize> {
        let n = self.shared.db.lock().retry_failed()?;
        self.shared.send(Cmd::Wake);
        Ok(n)
    }

    /// Swaps in new engines (e.g. after models finish downloading).
    pub fn set_provider(&self, p: Arc<dyn EngineProvider>) -> Result<()> {
        let model = p.embed_model_id();
        *self.shared.provider.write() = p;
        self.shared.provider_gen.fetch_add(1, Ordering::SeqCst);
        if self.shared.vectors.read().model() != model {
            let db = self.shared.db.lock();
            *self.shared.vectors.write() = VectorIndex::load(&db, &model)?;
        }
        self.shared.requeue_outdated()?;
        self.shared.send(Cmd::Wake);
        Ok(())
    }

    pub fn provider(&self) -> Arc<dyn EngineProvider> {
        self.shared.provider.read().clone()
    }

    /// Applies new settings; rescans when folders or skip rules changed.
    pub fn update_settings(&self, new: Settings) -> Result<()> {
        let old = self.shared.settings.read().clone();
        new.save(&self.shared.appdata)?;
        *self.shared.settings.write() = new.clone();
        if old.folders != new.folders
            || old.min_file_bytes != new.min_file_bytes
            || old.max_file_bytes != new.max_file_bytes
        {
            let roots: Vec<String> = new.enabled_folders().map(|f| path_str(&f.path)).collect();
            let removed = self.shared.db.lock().remove_roots_except(&roots)?;
            self.shared.forget(&removed);
            self.shared.cancel_scan.store(true, Ordering::SeqCst);
            self.shared.send(Cmd::Rescan);
        }
        if old.semantic_enabled != new.semantic_enabled {
            self.shared.provider_gen.fetch_add(1, Ordering::SeqCst);
            self.shared.requeue_outdated()?;
            self.shared.send(Cmd::Wake);
        }
        Ok(())
    }

    /// Deletes the whole index (database rows + thumbnails), then rescans.
    pub fn wipe(&self) -> Result<()> {
        let (tx, rx) = bounded(1);
        self.shared.cancel_scan.store(true, Ordering::SeqCst);
        self.shared.send(Cmd::Wipe(tx));
        rx.recv_timeout(Duration::from_secs(120)).map_err(|_| anyhow::anyhow!("indexer did not respond to wipe"))?
    }

    /// Stops background threads and waits for them (current file finishes first).
    pub fn shutdown(&mut self) {
        self.shared.cancel_scan.store(true, Ordering::SeqCst);
        self.shared.send(Cmd::Stop);
        if let Some(h) = self.coordinator.take() {
            let _ = h.join();
        }
    }

    /// Blocks until the queue is empty and nothing is running (tests, CLI).
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let start = Instant::now();
        // Give a freshly sent command time to be picked up.
        std::thread::sleep(Duration::from_millis(50));
        while start.elapsed() < timeout {
            let st = self.status();
            if matches!(st.phase, Phase::Idle | Phase::Stopped) && st.counts.pending == 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }
}

impl Drop for Indexer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Shared {
    fn send(&self, c: Cmd) {
        let _ = self.cmd_tx.send(c);
    }

    pub fn with_db<T>(&self, f: impl FnOnce(&mut Db) -> Result<T>) -> Result<T> {
        f(&mut self.db.lock())
    }

    fn persist_paused(&self, paused: bool) {
        let mut s = self.settings.write();
        s.paused = paused;
        if let Err(e) = s.save(&self.appdata) {
            tracing::warn!("could not save settings: {e}");
        }
    }

    fn requeue_outdated(&self) -> Result<()> {
        let p = self.provider.read().clone();
        let ocr_v = p.ocr_version();
        let emb_v = if self.settings.read().semantic_enabled { p.embed_version() } else { 0 };
        let n = self.db.lock().requeue_outdated(ocr_v, emb_v)?;
        if n > 0 {
            tracing::info!("re-queued {n} file(s) for newly available pipeline stages");
        }
        let mut st = self.status.lock();
        st.ocr_available = ocr_v > 0;
        st.semantic_available = p.embed_version() > 0;
        Ok(())
    }

    fn forget(&self, ids: &[i64]) {
        if ids.is_empty() {
            return;
        }
        let mut v = self.vectors.write();
        for &id in ids {
            v.remove(id);
            let _ = self.appdata.remove_file(&self.appdata.thumb_path(id));
        }
    }

    fn set_phase(&self, p: Phase) {
        let changed = {
            let mut st = self.status.lock();
            let changed = st.phase != p;
            st.phase = p;
            if p != Phase::Indexing {
                st.current = None;
            }
            changed
        };
        if changed {
            self.refresh_counts();
            self.notify(true);
        }
    }

    fn set_error(&self, e: String) {
        self.status.lock().last_error = Some(e);
    }

    fn refresh_counts(&self) {
        if let Ok(c) = self.db.lock().counts() {
            self.status.lock().counts = c;
        }
    }

    fn notify(&self, force: bool) {
        thread_local! {
            static LAST: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
        }
        let now = Instant::now();
        let due = LAST.with(|l| {
            let due = force || l.get().is_none_or(|t| now.duration_since(t) > Duration::from_millis(250));
            if due {
                l.set(Some(now));
            }
            due
        });
        if !due {
            return;
        }
        let st = self.status.lock().clone();
        if let Some(l) = self.listener.read().as_ref() {
            l(&st);
        }
    }
}

use crate::runtime::lower_thread_priority;

struct Pool {
    job_tx: Sender<Job>,
    result_rx: Receiver<(Job, Option<Outcome>)>,
    handles: Vec<JoinHandle<()>>,
}

impl Pool {
    fn new(shared: &Arc<Shared>, n: usize) -> Result<Pool> {
        let (job_tx, job_rx) = bounded::<Job>(n * 2);
        let (result_tx, result_rx) = unbounded();
        let mut handles = Vec::new();
        for i in 0..n {
            let (job_rx, result_tx, shared) = (job_rx.clone(), result_tx.clone(), shared.clone());
            handles.push(std::thread::Builder::new().name(format!("shotfind-worker-{i}")).spawn(move || {
                lower_thread_priority();
                let mut engines = WorkerEngines::default();
                let mut gen = 0u64;
                loop {
                    let job = match job_rx.recv_timeout(Duration::from_secs(30)) {
                        Ok(j) => j,
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                            // Idle: free the models (hundreds of MB) until work arrives.
                            if gen != 0 {
                                engines = WorkerEngines::default();
                                gen = 0;
                            }
                            continue;
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                    };
                    if shared.paused.load(Ordering::SeqCst) {
                        let _ = result_tx.send((job, None));
                        continue;
                    }
                    let g = shared.provider_gen.load(Ordering::SeqCst);
                    if g != gen {
                        engines = build_engines(&shared);
                        gen = g;
                    }
                    let settings = shared.settings.read().clone();
                    let out = pipeline::process(&job, &settings, &shared.appdata, &mut engines);
                    if result_tx.send((job, Some(out))).is_err() {
                        break;
                    }
                }
            })?);
        }
        Ok(Pool { job_tx, result_rx, handles })
    }

    fn shutdown(self) {
        drop(self.job_tx);
        for h in self.handles {
            let _ = h.join();
        }
    }
}

fn build_engines(shared: &Shared) -> WorkerEngines {
    let p = shared.provider.read().clone();
    let semantic = shared.settings.read().semantic_enabled;
    let ocr = p.new_ocr().unwrap_or_else(|e| {
        tracing::error!("OCR engine failed to load: {e:#}");
        shared.set_error(format!("OCR engine failed to load: {e:#}"));
        None
    });
    let embed = if semantic {
        p.new_image_embedder().unwrap_or_else(|e| {
            tracing::error!("image embedder failed to load: {e:#}");
            shared.set_error(format!("Image model failed to load: {e:#}"));
            None
        })
    } else {
        None
    };
    WorkerEngines { ocr, embed }
}

fn coordinator_loop(shared: &Arc<Shared>, cmd_rx: &Receiver<Cmd>) -> Result<()> {
    let workers = shared.settings.read().effective_workers();
    let pool = Pool::new(shared, workers)?;
    let (watch_tx, watch_rx) = unbounded::<WatchEvent>();
    let mut watcher: Option<FolderWatcher> = None;
    let mut need_scan = true;
    let mut recent: VecDeque<Instant> = VecDeque::new();
    let mut last_counts = Instant::now();
    let mut stop = false;

    let handle_cmd = |c: Cmd, need_scan: &mut bool, stop: &mut bool| match c {
        Cmd::Wake => {}
        Cmd::Rescan => *need_scan = true,
        Cmd::Stop => *stop = true,
        Cmd::Wipe(reply) => {
            let r = (|| -> Result<()> {
                shared.db.lock().wipe()?;
                shared.appdata.remove_dir_all(&shared.appdata.thumbs_dir())?;
                std::fs::create_dir_all(shared.appdata.thumbs_dir())?;
                let model = shared.vectors.read().model().to_string();
                let fresh = VectorIndex::load(&shared.db.lock(), &model)?;
                *shared.vectors.write() = fresh;
                Ok(())
            })();
            shared.refresh_counts();
            *need_scan = true;
            let _ = reply.send(r);
        }
    };

    while !stop {
        // 1. Commands and file-system events.
        while let Ok(c) = cmd_rx.try_recv() {
            handle_cmd(c, &mut need_scan, &mut stop);
        }
        if stop {
            break;
        }
        let mut changed_any = false;
        while let Ok(ev) = watch_rx.try_recv() {
            changed_any |= apply_watch_event(shared, ev, &mut need_scan);
        }
        if changed_any {
            shared.refresh_counts();
        }

        let paused = shared.paused.load(Ordering::SeqCst);
        if paused {
            shared.set_phase(Phase::Paused);
            select! {
                recv(cmd_rx) -> c => if let Ok(c) = c { handle_cmd(c, &mut need_scan, &mut stop) },
                default(Duration::from_secs(2)) => {},
            }
            continue;
        }

        // 2. Full scan (startup, folder changes, watcher overflow, resume).
        if need_scan {
            need_scan = false;
            shared.cancel_scan.store(false, Ordering::SeqCst);
            shared.set_phase(Phase::Scanning);
            let folders: Vec<PathBuf> = shared.settings.read().enabled_folders().map(|f| f.path.clone()).collect();
            let stats = full_scan(shared, &folders)?;
            shared.status.lock().last_scan = Some(stats);
            shared.refresh_counts();
            // Without a watcher (e.g. inotify limits reached) indexing still works;
            // new files are picked up by the next rescan.
            drop(watcher.take());
            match watcher::watch(&folders, watch_tx.clone()) {
                Ok(w) => {
                    shared.status.lock().watching = w.watching.len();
                    watcher = Some(w);
                }
                Err(e) => {
                    tracing::warn!("file watcher unavailable: {e}");
                    shared.status.lock().watching = 0;
                }
            }
            shared.notify(true);
            continue;
        }

        // 3. Index a batch.
        let batch = {
            let db = shared.db.lock();
            let b = db.next_pending(workers * 2, &[])?;
            for f in &b {
                db.begin_attempt(f.id)?;
            }
            b
        };
        if batch.is_empty() {
            shared.set_phase(Phase::Idle);
            select! {
                recv(cmd_rx) -> c => if let Ok(c) = c { handle_cmd(c, &mut need_scan, &mut stop) },
                recv(watch_rx) -> ev => if let Ok(ev) = ev {
                    if apply_watch_event(shared, ev, &mut need_scan) { shared.refresh_counts(); }
                },
                default(Duration::from_secs(60)) => {},
            }
            continue;
        }
        shared.set_phase(Phase::Indexing);
        let p = shared.provider.read().clone();
        let semantic = shared.settings.read().semantic_enabled;
        let (ocr_v, emb_v, model) = (p.ocr_version(), if semantic { p.embed_version() } else { 0 }, p.embed_model_id());
        let n = batch.len();
        for f in batch {
            let job = Job {
                need_ocr: ocr_v > f.ocr_version,
                need_embed: emb_v > f.embed_version,
                ocr_version: ocr_v,
                embed_version: emb_v,
                embed_model: model.clone(),
                file: f,
            };
            shared.status.lock().current = Some(job.file.path.clone());
            pool.job_tx.send(job)?;
        }
        for _ in 0..n {
            let (job, out) = pool.result_rx.recv()?;
            let id = job.file.id;
            let mut db = shared.db.lock();
            match out {
                None => {
                    db.conn().execute("UPDATE files SET attempts=MAX(0, attempts-1) WHERE id=?1", [id]).map(|_| ())?
                }
                Some(Outcome::Done(data)) => {
                    let emb = data.embedding.as_ref().map(|(_, v)| v.clone());
                    match db.complete(id, &job.file.name, &data) {
                        Ok(()) => {
                            if let Some(v) = emb {
                                shared.vectors.write().upsert(id, v);
                            }
                        }
                        // Row vanished meanwhile (file deleted / wiped): not an error.
                        Err(e) => tracing::debug!("complete({id}) skipped: {e}"),
                    }
                    recent.push_back(Instant::now());
                }
                Some(Outcome::Skipped { reason, dims }) => db.skip(id, &reason, dims)?,
                Some(Outcome::Failed(err)) => {
                    tracing::debug!("failed to index {}: {err}", job.file.path);
                    db.fail(id, &err)?;
                }
            }
            drop(db);
            while recent.len() > 60 {
                recent.pop_front();
            }
            if recent.len() >= 2 {
                let span = recent.back().unwrap().duration_since(*recent.front().unwrap()).as_secs_f64();
                if span > 0.0 {
                    shared.status.lock().per_minute = (recent.len() - 1) as f64 * 60.0 / span;
                }
            }
            // COUNT(*) GROUP BY over a 100k-row table is not free: at most ~3/s.
            if last_counts.elapsed() > Duration::from_millis(300) {
                shared.refresh_counts();
                last_counts = Instant::now();
            }
            shared.notify(false);
        }
    }
    drop(watcher);
    pool.shutdown();
    Ok(())
}

fn full_scan(shared: &Arc<Shared>, folders: &[PathBuf]) -> Result<ScanStats> {
    let settings = shared.settings.read().clone();
    let epoch = {
        let db = shared.db.lock();
        let e = db.meta_get("scan_epoch")?.and_then(|s| s.parse::<i64>().ok()).unwrap_or(0) + 1;
        db.meta_set("scan_epoch", &e.to_string())?;
        e
    };
    let mut total = ScanStats::default();
    for f in folders {
        if shared.cancel_scan.load(Ordering::SeqCst) {
            break;
        }
        // Scan holds the db lock per folder; searches use their own connection.
        let (stats, removed) = {
            let mut db = shared.db.lock();
            scanner::scan_folder(&mut db, &settings, f, epoch, Some(&shared.appdata), &shared.cancel_scan)?
        };
        shared.forget(&removed);
        total += stats;
        shared.refresh_counts();
        shared.notify(false);
    }
    tracing::info!("scan done: {total:?}");
    Ok(total)
}

fn apply_watch_event(shared: &Arc<Shared>, ev: WatchEvent, need_scan: &mut bool) -> bool {
    let settings = shared.settings.read().clone();
    let epoch = shared.db.lock().meta_get("scan_epoch").ok().flatten().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
    let path = match ev {
        WatchEvent::Rescan => {
            *need_scan = true;
            return false;
        }
        WatchEvent::Changed(p) | WatchEvent::Removed(p) => p,
    };
    if shared.appdata.contains(&path) {
        return false;
    }
    let r = scanner::apply_path_change(&mut shared.db.lock(), &settings, &path, epoch);
    match r {
        Ok((changed, removed)) => {
            let any = changed || !removed.is_empty();
            shared.forget(&removed);
            any
        }
        Err(e) => {
            tracing::debug!("watch event for {} failed: {e}", path.display());
            false
        }
    }
}
