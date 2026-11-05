mod common;

use common::*;
use shotfind_core::appdata::AppData;
use shotfind_core::db::{Db, ScannedFile, MAX_ATTEMPTS};
use shotfind_core::indexer::Indexer;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(30);

fn ocr_texts(data: &std::path::Path) -> Vec<(String, String)> {
    let db = Db::open(&AppData::open(data).unwrap().db_path()).unwrap();
    let mut st =
        db.conn().prepare("SELECT f.name, o.text FROM files f JOIN ocr o ON o.file_id=f.id ORDER BY f.name").unwrap();
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect()
}

/// (path, state, error) of every row, for assertion messages.
fn dump(data: &std::path::Path) -> Vec<(String, i64, Option<String>)> {
    let db = Db::open(&AppData::open(data).unwrap().db_path()).unwrap();
    let mut st = db.conn().prepare("SELECT path, state, error FROM files ORDER BY path").unwrap();
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(|r| r.unwrap()).collect()
}

#[test]
fn initial_index_applies_skip_rules() {
    let fx = Fixture::new();
    fx.standard_set();
    let provider = FakeProvider::new();
    let ix = Indexer::start(AppData::open(&fx.data).unwrap(), fx.settings(), provider.clone()).unwrap();
    assert!(ix.wait_idle(WAIT), "indexer did not finish: {:?}", ix.status());
    let st = ix.status();
    // 4 real images + icon + broken file are registered; txt, hidden dir, node_modules are not.
    assert_eq!(st.counts.total, 6, "{st:?}");
    assert_eq!(st.counts.done, 4, "{st:?}");
    assert_eq!(st.counts.skipped, 2, "icon + broken file: {st:?}");
    assert_eq!(provider.counters.ocr_calls.load(Ordering::SeqCst), 4);
    let texts = ocr_texts(&fx.data);
    assert!(texts.iter().any(|(n, t)| n.starts_with("Screenshot") && t.contains("UPI payment to Rahul")));
    // Thumbnails exist for indexed files.
    let ad = AppData::open(&fx.data).unwrap();
    let thumbs =
        walkdir::WalkDir::new(ad.thumbs_dir()).into_iter().flatten().filter(|e| e.file_type().is_file()).count();
    assert_eq!(thumbs, 4);
    assert_eq!(ix.shared().vectors.read().len(), 4);
}

#[test]
fn incremental_rescan_handles_add_change_delete() {
    let fx = Fixture::new();
    fx.standard_set();
    let provider = FakeProvider::new();
    let mut ix = Indexer::start(AppData::open(&fx.data).unwrap(), fx.settings(), provider.clone()).unwrap();
    assert!(ix.wait_idle(WAIT));
    let calls_before = provider.counters.ocr_calls.load(Ordering::SeqCst);
    ix.shutdown();

    // While the app is closed: one new, one changed, one deleted file.
    write_png(&fx.user.join("new.png"), &synth_image(700, 300, &["fresh invoice 77"], "receipt"));
    std::thread::sleep(Duration::from_millis(20));
    write_png(&fx.user.join("error.png"), &synth_image(820, 410, &["Error 500 internal"], "code"));
    std::fs::remove_file(fx.user.join("sub/deeper/cat.png")).unwrap();

    let ix = Indexer::start(AppData::open(&fx.data).unwrap(), fx.settings(), provider.clone()).unwrap();
    assert!(ix.wait_idle(WAIT));
    // Only the new and the changed file were processed again.
    assert_eq!(provider.counters.ocr_calls.load(Ordering::SeqCst) - calls_before, 2);
    let texts = ocr_texts(&fx.data);
    assert!(texts.iter().any(|(_, t)| t.contains("fresh invoice 77")));
    assert!(texts.iter().any(|(n, t)| n == "error.png" && t.contains("Error 500")));
    assert!(!texts.iter().any(|(_, t)| t.contains("Error 404")), "stale text must be gone");
    assert!(!texts.iter().any(|(n, _)| n == "cat.png"));
    assert_eq!(ix.shared().vectors.read().len(), 4);
}

#[test]
fn watcher_picks_up_new_and_deleted_files() {
    let fx = Fixture::new();
    fx.standard_set();
    let ix = Indexer::start(AppData::open(&fx.data).unwrap(), fx.settings(), FakeProvider::new()).unwrap();
    assert!(ix.wait_idle(WAIT));
    assert_eq!(ix.status().counts.done, 4);

    write_png(&fx.user.join("sub/live.png"), &synth_image(700, 300, &["watched file arrives"], "none"));
    std::fs::remove_file(fx.user.join("error.png")).unwrap();
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        let texts = ocr_texts(&fx.data);
        let added = texts.iter().any(|(_, t)| t.contains("watched file arrives"));
        let removed = !texts.iter().any(|(n, _)| n == "error.png");
        if added && removed {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "watcher did not apply changes: {texts:?}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn resumes_after_crash_and_quarantines_repeat_crashers() {
    let fx = Fixture::new();
    fx.standard_set();
    let ad = AppData::open(&fx.data).unwrap();
    let settings = fx.settings();
    // Simulate a previous run that was killed mid-way: files registered, one
    // file "in flight" (attempt counted, never finished), one that crashed
    // the process MAX_ATTEMPTS times.
    {
        let mut db = Db::open(&ad.db_path()).unwrap();
        let root = shotfind_core::scanner::path_str(&settings.folders[0].path);
        let never = std::sync::atomic::AtomicBool::new(false);
        shotfind_core::scanner::scan_folder(&mut db, &settings, &settings.folders[0].path, 1, Some(&ad), &never)
            .unwrap();
        let id_of = |db: &Db, name: &str| -> i64 {
            db.conn().query_row("SELECT id FROM files WHERE name=?1", [name], |r| r.get(0)).unwrap()
        };
        db.begin_attempt(id_of(&db, "error.png")).unwrap();
        let cat = id_of(&db, "cat.png");
        for _ in 0..MAX_ATTEMPTS {
            db.begin_attempt(cat).unwrap();
        }
        let _ =
            (root, ScannedFile { path: "", root: "", dir: "", name: "", size: 0, mtime_ms: 0, is_screenshot: false });
    }
    let ix = Indexer::start(ad, settings, FakeProvider::new()).unwrap();
    assert!(ix.wait_idle(WAIT));
    let st = ix.status();
    assert_eq!(st.counts.done, 3, "{st:?}");
    assert_eq!(st.counts.failed, 1, "the repeat crasher is quarantined: {st:?}");
    let texts = ocr_texts(&fx.data);
    assert!(texts.iter().any(|(n, t)| n == "error.png" && t.contains("Error 404")), "in-flight file is redone");
    // Retry brings the quarantined file back.
    assert_eq!(ix.retry_failed().unwrap(), 1);
    assert!(ix.wait_idle(WAIT));
    assert_eq!(ix.status().counts.done, 4);
}

#[test]
fn panicking_engine_marks_file_failed_and_continues() {
    let fx = Fixture::new();
    fx.standard_set();
    let provider = Arc::new(FakeProvider {
        counters: Default::default(),
        ocr: true,
        embed: true,
        ocr_version: 1,
        panic_on: Some("Quarterly".into()),
    });
    let ix = Indexer::start(AppData::open(&fx.data).unwrap(), fx.settings(), provider).unwrap();
    assert!(ix.wait_idle(WAIT));
    let st = ix.status();
    assert_eq!(st.counts.done, 3, "{st:?}");
    assert_eq!(st.counts.failed, 1, "{st:?}");
}

#[test]
fn pause_resume_and_restart_continue_where_left_off() {
    let fx = Fixture::new();
    for i in 0..40 {
        write_png(
            &fx.user.join(format!("shot{i:02}.png")),
            &synth_image(640, 360, &[&format!("item number {i}")], "none"),
        );
    }
    let provider = FakeProvider::new();
    let mut ix = Indexer::start(AppData::open(&fx.data).unwrap(), fx.settings(), provider.clone()).unwrap();
    // Pause as soon as some work has happened.
    let deadline = std::time::Instant::now() + WAIT;
    while provider.counters.ocr_calls.load(Ordering::SeqCst) < 3 {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    ix.pause();
    std::thread::sleep(Duration::from_millis(300));
    let at_pause = provider.counters.ocr_calls.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(provider.counters.ocr_calls.load(Ordering::SeqCst), at_pause, "no work while paused");
    ix.shutdown();
    drop(ix);

    // Restart: settings remember "paused"; resume and finish without redoing work.
    let ad = AppData::open(&fx.data).unwrap();
    let mut settings = shotfind_core::config::Settings::load(&ad);
    assert!(settings.paused);
    settings.paused = false;
    let ix = Indexer::start(ad, settings, provider.clone()).unwrap();
    assert!(ix.wait_idle(WAIT));
    assert_eq!(ix.status().counts.done, 40);
    assert_eq!(provider.counters.ocr_calls.load(Ordering::SeqCst), 40, "each file OCR'd exactly once");
}

#[test]
fn models_installed_later_requeue_only_missing_stages() {
    let fx = Fixture::new();
    fx.standard_set();
    let ocr_only = FakeProvider::ocr_only();
    let ix = Indexer::start(AppData::open(&fx.data).unwrap(), fx.settings(), ocr_only.clone()).unwrap();
    assert!(ix.wait_idle(WAIT));
    assert_eq!(ix.shared().vectors.read().len(), 0);
    let full = FakeProvider::new();
    ix.set_provider(full.clone()).unwrap();
    assert!(ix.wait_idle(WAIT));
    assert_eq!(ix.shared().vectors.read().len(), 4);
    assert_eq!(full.counters.ocr_calls.load(Ordering::SeqCst), 0, "OCR is not redone");
    assert_eq!(full.counters.embed_calls.load(Ordering::SeqCst), 4);
}

#[test]
fn removing_a_folder_drops_its_files_and_wipe_clears_everything() {
    let fx = Fixture::new();
    fx.standard_set();
    let ix = Indexer::start(AppData::open(&fx.data).unwrap(), fx.settings(), FakeProvider::new()).unwrap();
    assert!(ix.wait_idle(WAIT));
    ix.wipe().unwrap();
    // Wipe triggers a rescan: everything comes back.
    assert!(ix.wait_idle(WAIT));
    assert_eq!(ix.status().counts.done, 4, "rows after wipe: {:#?}", dump(&fx.data));
    let mut s = fx.settings();
    s.folders.clear();
    ix.update_settings(s).unwrap();
    assert!(ix.wait_idle(WAIT));
    assert_eq!(ix.status().counts.total, 0);
    let ad = AppData::open(&fx.data).unwrap();
    let thumbs =
        walkdir::WalkDir::new(ad.thumbs_dir()).into_iter().flatten().filter(|e| e.file_type().is_file()).count();
    assert_eq!(thumbs, 0, "thumbnails of removed files are cleaned up");
}
