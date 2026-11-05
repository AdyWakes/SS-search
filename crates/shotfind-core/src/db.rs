//! SQLite storage: file registry, OCR text (+FTS5 index), embeddings.
//!
//! The database lives in the app data dir. It records *about* user files; it
//! never stores a path we would write to.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;

/// Bump when the schema changes in a way that needs a rebuild.
pub const SCHEMA_VERSION: i64 = 1;

pub const STATE_PENDING: i64 = 0;
pub const STATE_DONE: i64 = 1;
pub const STATE_FAILED: i64 = 2;
pub const STATE_SKIPPED: i64 = 3;

/// A file that crashed the process this many times is given up on.
pub const MAX_ATTEMPTS: i64 = 3;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS files(
    id            INTEGER PRIMARY KEY,
    path          TEXT NOT NULL UNIQUE,
    root          TEXT NOT NULL,
    dir           TEXT NOT NULL,
    name          TEXT NOT NULL,
    size          INTEGER NOT NULL,
    mtime         INTEGER NOT NULL,
    width         INTEGER,
    height        INTEGER,
    is_screenshot INTEGER NOT NULL DEFAULT 0,
    state         INTEGER NOT NULL DEFAULT 0,
    attempts      INTEGER NOT NULL DEFAULT 0,
    ocr_version   INTEGER NOT NULL DEFAULT 0,
    embed_version INTEGER NOT NULL DEFAULT 0,
    error         TEXT,
    indexed_at    INTEGER,
    seen_epoch    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS files_state ON files(state, is_screenshot, mtime);
CREATE INDEX IF NOT EXISTS files_mtime ON files(mtime);
CREATE INDEX IF NOT EXISTS files_root ON files(root, seen_epoch);

CREATE TABLE IF NOT EXISTS ocr(
    file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    text    TEXT NOT NULL,
    name    TEXT NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS ocr_fts USING fts5(
    text, name,
    content='ocr', content_rowid='file_id',
    tokenize='unicode61 remove_diacritics 2'
);
CREATE VIRTUAL TABLE IF NOT EXISTS ocr_vocab USING fts5vocab(ocr_fts, 'row');

CREATE TRIGGER IF NOT EXISTS ocr_ai AFTER INSERT ON ocr BEGIN
    INSERT INTO ocr_fts(rowid, text, name) VALUES (new.file_id, new.text, new.name);
END;
CREATE TRIGGER IF NOT EXISTS ocr_ad AFTER DELETE ON ocr BEGIN
    INSERT INTO ocr_fts(ocr_fts, rowid, text, name) VALUES ('delete', old.file_id, old.text, old.name);
END;
CREATE TRIGGER IF NOT EXISTS ocr_au AFTER UPDATE ON ocr BEGIN
    INSERT INTO ocr_fts(ocr_fts, rowid, text, name) VALUES ('delete', old.file_id, old.text, old.name);
    INSERT INTO ocr_fts(rowid, text, name) VALUES (new.file_id, new.text, new.name);
END;

CREATE TABLE IF NOT EXISTS embeddings(
    file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,
    model   TEXT NOT NULL,
    vec     BLOB NOT NULL
);
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upsert {
    New(i64),
    Changed(i64),
    Unchanged(i64),
}

impl Upsert {
    pub fn id(self) -> i64 {
        match self {
            Upsert::New(i) | Upsert::Changed(i) | Upsert::Unchanged(i) => i,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScannedFile<'a> {
    pub path: &'a str,
    pub root: &'a str,
    pub dir: &'a str,
    pub name: &'a str,
    pub size: u64,
    pub mtime_ms: i64,
    pub is_screenshot: bool,
}

#[derive(Debug, Clone)]
pub struct PendingFile {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub size: u64,
    pub mtime_ms: i64,
    pub ocr_version: i64,
    pub embed_version: i64,
    pub attempts: i64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct Counts {
    pub total: u64,
    pub done: u64,
    pub pending: u64,
    pub failed: u64,
    pub skipped: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileRow {
    pub id: i64,
    pub path: String,
    pub dir: String,
    pub name: String,
    pub size: u64,
    pub mtime_ms: i64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub is_screenshot: bool,
    pub state: i64,
}

/// Everything the indexer learned about one file, written in one transaction.
#[derive(Debug, Clone, Default)]
pub struct IndexedData {
    pub width: u32,
    pub height: u32,
    pub is_screenshot: bool,
    /// `Some` when OCR ran this time (text may be empty).
    pub ocr_text: Option<String>,
    pub ocr_version: i64,
    /// `Some((model_id, vector))` when the embedder ran this time.
    pub embedding: Option<(String, Vec<f32>)>,
    pub embed_version: i64,
}

pub struct Db {
    pub(crate) conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        let conn = Connection::open(path).with_context(|| format!("opening db {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Db> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Db> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON; PRAGMA temp_store=MEMORY;",
        )?;
        conn.execute_batch(SCHEMA)?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION.to_string()],
        )?;
        Ok(Db { conn })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn tx<T>(&mut self, f: impl FnOnce(&rusqlite::Transaction) -> Result<T>) -> Result<T> {
        let tx = self.conn.transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM meta WHERE key=?1", params![key], |r| r.get(0)).optional()?)
    }

    pub fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Inserts or refreshes a scanned file. A file whose size or mtime changed is
    /// reset to pending and its derived data dropped.
    pub fn upsert_scanned(conn: &Connection, f: &ScannedFile, epoch: i64) -> Result<Upsert> {
        let existing: Option<(i64, i64, i64)> = conn
            .query_row("SELECT id, size, mtime FROM files WHERE path=?1", params![f.path], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        match existing {
            None => {
                conn.execute(
                    "INSERT INTO files(path, root, dir, name, size, mtime, is_screenshot, state, seen_epoch)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8)",
                    params![f.path, f.root, f.dir, f.name, f.size as i64, f.mtime_ms, f.is_screenshot, epoch],
                )?;
                Ok(Upsert::New(conn.last_insert_rowid()))
            }
            Some((id, size, mtime)) if size == f.size as i64 && mtime == f.mtime_ms => {
                conn.execute(
                    "UPDATE files SET seen_epoch=?2, root=?3, is_screenshot=MAX(is_screenshot, ?4) WHERE id=?1",
                    params![id, epoch, f.root, f.is_screenshot],
                )?;
                Ok(Upsert::Unchanged(id))
            }
            Some((id, _, _)) => {
                conn.execute(
                    "UPDATE files SET size=?2, mtime=?3, root=?4, is_screenshot=?5, state=0, attempts=0,
                        ocr_version=0, embed_version=0, error=NULL, width=NULL, height=NULL, seen_epoch=?6
                     WHERE id=?1",
                    params![id, f.size as i64, f.mtime_ms, f.root, f.is_screenshot, epoch],
                )?;
                conn.execute("DELETE FROM ocr WHERE file_id=?1", params![id])?;
                conn.execute("DELETE FROM embeddings WHERE file_id=?1", params![id])?;
                Ok(Upsert::Changed(id))
            }
        }
    }

    /// Removes rows under `root` that were not seen in scan `epoch` (deleted files).
    /// Returns the removed ids so thumbnails can be cleaned up.
    pub fn remove_unseen(conn: &Connection, root: &str, epoch: i64) -> Result<Vec<i64>> {
        let mut st = conn.prepare("SELECT id FROM files WHERE root=?1 AND seen_epoch<>?2")?;
        let ids: Vec<i64> = st.query_map(params![root, epoch], |r| r.get(0))?.collect::<Result<_, _>>()?;
        for id in &ids {
            conn.execute("DELETE FROM files WHERE id=?1", params![id])?;
        }
        Ok(ids)
    }

    /// Removes rows whose root is not one of `roots` (a folder was removed in settings).
    pub fn remove_roots_except(&mut self, roots: &[String]) -> Result<Vec<i64>> {
        self.tx(|tx| {
            let mut st = tx.prepare("SELECT id, root FROM files")?;
            let rows: Vec<(i64, String)> =
                st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
            let mut removed = Vec::new();
            for (id, root) in rows {
                if !roots.iter().any(|r| r == &root) {
                    tx.execute("DELETE FROM files WHERE id=?1", params![id])?;
                    removed.push(id);
                }
            }
            Ok(removed)
        })
    }

    pub fn remove_path(conn: &Connection, path: &str) -> Result<Vec<i64>> {
        // Also covers a removed directory: everything under it goes.
        let sep = std::path::MAIN_SEPARATOR;
        let prefix = format!("{}{}", path.trim_end_matches(sep), sep);
        let mut st = conn.prepare("SELECT id FROM files WHERE path=?1 OR substr(path, 1, length(?2))=?2")?;
        let ids: Vec<i64> = st.query_map(params![path, prefix], |r| r.get(0))?.collect::<Result<_, _>>()?;
        for id in &ids {
            conn.execute("DELETE FROM files WHERE id=?1", params![id])?;
        }
        Ok(ids)
    }

    /// Next files to index: screenshots first, newest first (what people search for).
    pub fn next_pending(&self, limit: usize, exclude: &[i64]) -> Result<Vec<PendingFile>> {
        let mut st = self.conn.prepare_cached(
            "SELECT id, path, name, size, mtime, ocr_version, embed_version, attempts FROM files
             WHERE state=0 ORDER BY is_screenshot DESC, mtime DESC LIMIT ?1",
        )?;
        let rows = st
            .query_map(params![(limit + exclude.len()) as i64], |r| {
                Ok(PendingFile {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    name: r.get(2)?,
                    size: r.get::<_, i64>(3)? as u64,
                    mtime_ms: r.get(4)?,
                    ocr_version: r.get(5)?,
                    embed_version: r.get(6)?,
                    attempts: r.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows.into_iter().filter(|p| !exclude.contains(&p.id)).take(limit).collect())
    }

    /// Records that we are about to process `id`. Committed before the work starts,
    /// so a hard crash mid-file is counted on restart.
    pub fn begin_attempt(&self, id: i64) -> Result<()> {
        self.conn.execute("UPDATE files SET attempts=attempts+1 WHERE id=?1", params![id])?;
        Ok(())
    }

    /// Files that crashed the indexer `MAX_ATTEMPTS` times are marked failed.
    pub fn quarantine_crashers(&self) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE files SET state=2, error='indexing crashed repeatedly on this file' WHERE state=0 AND attempts>=?1",
            params![MAX_ATTEMPTS],
        )?)
    }

    pub fn complete(&mut self, id: i64, name: &str, d: &IndexedData) -> Result<()> {
        self.tx(|tx| {
            if let Some(text) = &d.ocr_text {
                tx.execute(
                    "INSERT INTO ocr(file_id, text, name) VALUES (?1, ?2, ?3)
                     ON CONFLICT(file_id) DO UPDATE SET text=excluded.text, name=excluded.name",
                    params![id, text, searchable_name(name)],
                )?;
            } else {
                // Filename search works even before OCR models are installed.
                tx.execute(
                    "INSERT INTO ocr(file_id, text, name) VALUES (?1, '', ?2) ON CONFLICT(file_id) DO NOTHING",
                    params![id, searchable_name(name)],
                )?;
            }
            if let Some((model, v)) = &d.embedding {
                tx.execute(
                    "INSERT INTO embeddings(file_id, model, vec) VALUES (?1, ?2, ?3)
                     ON CONFLICT(file_id) DO UPDATE SET model=excluded.model, vec=excluded.vec",
                    params![id, model, f32s_to_bytes(v)],
                )?;
            }
            tx.execute(
                "UPDATE files SET state=1, attempts=0, error=NULL, width=?2, height=?3,
                    is_screenshot=MAX(is_screenshot, ?4),
                    ocr_version=MAX(ocr_version, ?5), embed_version=MAX(embed_version, ?6),
                    indexed_at=strftime('%s','now')
                 WHERE id=?1",
                params![id, d.width, d.height, d.is_screenshot, d.ocr_version, d.embed_version],
            )?;
            Ok(())
        })
    }

    pub fn fail(&self, id: i64, err: &str) -> Result<()> {
        self.conn
            .execute("UPDATE files SET state=2, error=?2, attempts=0 WHERE id=?1", params![id, truncate(err, 500)])?;
        Ok(())
    }

    pub fn skip(&self, id: i64, reason: &str, dims: Option<(u32, u32)>) -> Result<()> {
        self.conn.execute(
            "UPDATE files SET state=3, error=?2, attempts=0, width=?3, height=?4 WHERE id=?1",
            params![id, reason, dims.map(|d| d.0), dims.map(|d| d.1)],
        )?;
        Ok(())
    }

    /// Re-queues finished files that are missing a pipeline stage that is now
    /// available (models installed later, or a pipeline version bump).
    pub fn requeue_outdated(&self, ocr_version: i64, embed_version: i64) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE files SET state=0 WHERE state=1 AND (ocr_version<?1 OR embed_version<?2)",
            params![ocr_version, embed_version],
        )?)
    }

    pub fn retry_failed(&self) -> Result<usize> {
        Ok(self.conn.execute("UPDATE files SET state=0, attempts=0, error=NULL WHERE state=2", [])?)
    }

    pub fn counts(&self) -> Result<Counts> {
        let mut c = Counts::default();
        let mut st = self.conn.prepare_cached("SELECT state, COUNT(*) FROM files GROUP BY state")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)? as u64)))?;
        for row in rows {
            let (state, n) = row?;
            c.total += n;
            match state {
                STATE_PENDING => c.pending = n,
                STATE_DONE => c.done = n,
                STATE_FAILED => c.failed = n,
                STATE_SKIPPED => c.skipped = n,
                _ => {}
            }
        }
        Ok(c)
    }

    pub fn file(&self, id: i64) -> Result<Option<FileRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, path, dir, name, size, mtime, width, height, is_screenshot, state FROM files WHERE id=?1",
                params![id],
                row_to_file,
            )
            .optional()?)
    }

    pub fn ocr_text(&self, id: i64) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT text FROM ocr WHERE file_id=?1", params![id], |r| r.get(0)).optional()?)
    }

    /// Drops everything. Used by "wipe index" in settings.
    pub fn wipe(&mut self) -> Result<()> {
        self.tx(|tx| {
            tx.execute_batch(
                "DELETE FROM embeddings; DELETE FROM ocr; DELETE FROM files;
                 INSERT INTO ocr_fts(ocr_fts) VALUES('rebuild');",
            )?;
            Ok(())
        })?;
        self.conn.execute_batch("VACUUM;")?;
        Ok(())
    }
}

pub(crate) fn row_to_file(r: &rusqlite::Row) -> rusqlite::Result<FileRow> {
    Ok(FileRow {
        id: r.get(0)?,
        path: r.get(1)?,
        dir: r.get(2)?,
        name: r.get(3)?,
        size: r.get::<_, i64>(4)? as u64,
        mtime_ms: r.get(5)?,
        width: r.get(6)?,
        height: r.get(7)?,
        is_screenshot: r.get(8)?,
        state: r.get(9)?,
    })
}

/// "Screenshot_2024-05-01_WhatsApp.png" -> "Screenshot 2024-05-01 WhatsApp" so the
/// tokenizer sees separate words (and the extension is not noise in results).
pub fn searchable_name(name: &str) -> String {
    let stem = Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let mut out = String::with_capacity(stem.len() + 8);
    let mut prev_lower = false;
    for ch in stem.chars() {
        let c = if ch == '_' || ch == '.' { ' ' } else { ch };
        // Split camelCase: "WhatsAppImage" -> "Whats App Image"
        if c.is_uppercase() && prev_lower {
            out.push(' ');
        }
        prev_lower = c.is_lowercase();
        out.push(c);
    }
    format!("{stem} {out}")
}

pub fn f32s_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

pub fn bytes_to_f32s(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scanned<'a>(path: &'a str, size: u64, mtime: i64) -> ScannedFile<'a> {
        ScannedFile { path, root: "/r", dir: "/r", name: "a.png", size, mtime_ms: mtime, is_screenshot: false }
    }

    #[test]
    fn upsert_detects_changes_and_drops_derived_data() {
        let mut db = Db::open_in_memory().unwrap();
        let id = Db::upsert_scanned(db.conn(), &scanned("/r/a.png", 10, 1), 1).unwrap();
        assert!(matches!(id, Upsert::New(_)));
        let id = id.id();
        db.complete(
            id,
            "a.png",
            &IndexedData {
                ocr_text: Some("hello world".into()),
                ocr_version: 1,
                embedding: Some(("m".into(), vec![1.0, 0.0])),
                embed_version: 1,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(db.ocr_text(id).unwrap().as_deref(), Some("hello world"));
        assert_eq!(Db::upsert_scanned(db.conn(), &scanned("/r/a.png", 10, 1), 2).unwrap(), Upsert::Unchanged(id));
        assert_eq!(db.counts().unwrap().done, 1);
        assert_eq!(Db::upsert_scanned(db.conn(), &scanned("/r/a.png", 11, 1), 3).unwrap(), Upsert::Changed(id));
        assert_eq!(db.counts().unwrap().pending, 1);
        assert_eq!(db.ocr_text(id).unwrap(), None);
        let n: i64 = db.conn().query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn fts_follows_cascading_deletes() {
        let mut db = Db::open_in_memory().unwrap();
        let id = Db::upsert_scanned(db.conn(), &scanned("/r/a.png", 10, 1), 1).unwrap().id();
        db.complete(id, "a.png", &IndexedData { ocr_text: Some("invoice total".into()), ..Default::default() })
            .unwrap();
        let hits = |db: &Db| -> i64 {
            db.conn().query_row("SELECT COUNT(*) FROM ocr_fts WHERE ocr_fts MATCH 'invoice'", [], |r| r.get(0)).unwrap()
        };
        assert_eq!(hits(&db), 1);
        Db::remove_path(db.conn(), "/r/a.png").unwrap();
        assert_eq!(hits(&db), 0);
    }

    #[test]
    fn remove_unseen_and_dirs() {
        let db = Db::open_in_memory().unwrap();
        Db::upsert_scanned(db.conn(), &scanned("/r/a.png", 1, 1), 1).unwrap();
        Db::upsert_scanned(db.conn(), &scanned("/r/sub/b.png", 1, 1), 1).unwrap();
        Db::upsert_scanned(db.conn(), &scanned("/r/subway.png", 1, 1), 1).unwrap();
        Db::upsert_scanned(db.conn(), &scanned("/r/a.png", 1, 1), 2).unwrap();
        assert_eq!(Db::remove_unseen(db.conn(), "/r", 2).unwrap().len(), 2);
        Db::upsert_scanned(db.conn(), &scanned("/r/sub/b.png", 1, 1), 3).unwrap();
        Db::upsert_scanned(db.conn(), &scanned("/r/subway.png", 1, 1), 3).unwrap();
        let sep = std::path::MAIN_SEPARATOR;
        if sep == '/' {
            assert_eq!(Db::remove_path(db.conn(), "/r/sub").unwrap().len(), 1, "prefix must not eat /r/subway.png");
        }
    }

    #[test]
    fn crash_attempts_quarantine() {
        let db = Db::open_in_memory().unwrap();
        let id = Db::upsert_scanned(db.conn(), &scanned("/r/a.png", 1, 1), 1).unwrap().id();
        for _ in 0..MAX_ATTEMPTS {
            db.begin_attempt(id).unwrap();
        }
        assert_eq!(db.quarantine_crashers().unwrap(), 1);
        assert_eq!(db.counts().unwrap().failed, 1);
    }

    #[test]
    fn searchable_names() {
        assert_eq!(searchable_name("WhatsAppImage_2024.png"), "WhatsAppImage_2024 Whats App Image 2024");
    }
}
