//! The app data directory: the ONLY place shotfind writes to.
//!
//! Every write the engine performs (database, thumbnails, downloaded models,
//! settings) goes through [`AppData`], which refuses any path outside its root.
//! User images are only ever opened read-only through [`crate::userfs`].

use anyhow::{bail, Context, Result};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AppData {
    root: PathBuf,
}

impl AppData {
    /// Creates (if needed) and opens the app data directory.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root).with_context(|| format!("creating app data dir {}", root.display()))?;
        let root = dunce_canonicalize(&root)?;
        let me = Self { root };
        fs::create_dir_all(me.thumbs_dir())?;
        fs::create_dir_all(me.models_dir())?;
        Ok(me)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn db_path(&self) -> PathBuf {
        self.root.join("index.sqlite3")
    }
    pub fn thumbs_dir(&self) -> PathBuf {
        self.root.join("thumbs")
    }
    pub fn models_dir(&self) -> PathBuf {
        self.root.join("models")
    }
    pub fn settings_path(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    /// Path of the cached thumbnail for an indexed file id. Sharded by id so no
    /// single directory grows past a few hundred entries.
    pub fn thumb_path(&self, file_id: i64) -> PathBuf {
        self.thumbs_dir().join(format!("{:02x}", file_id.rem_euclid(256))).join(format!("{file_id}.jpg"))
    }

    /// Returns `path` if it lies inside the app data root, else an error.
    /// Purely lexical (works for files that don't exist yet) and rejects `..`.
    pub fn guard(&self, path: &Path) -> Result<PathBuf> {
        if path.components().any(|c| matches!(c, Component::ParentDir)) {
            bail!("refusing path with '..': {}", path.display());
        }
        let abs = if path.is_absolute() { path.to_path_buf() } else { self.root.join(path) };
        if !abs.starts_with(&self.root) {
            bail!("refusing to write outside app data dir: {}", abs.display());
        }
        Ok(abs)
    }

    /// Atomically writes `bytes` to `path` (inside app data) via a temp file + rename.
    pub fn write_atomic(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        let path = self.guard(path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension(format!(
            "{}tmp",
            path.extension().and_then(|e| e.to_str()).map(|e| format!("{e}.")).unwrap_or_default()
        ));
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(bytes)?;
            f.sync_all().ok();
        }
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Creates a new file for streaming writes (used by the model downloader).
    pub fn create_file(&self, path: &Path) -> Result<fs::File> {
        let path = self.guard(path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(fs::File::create(path)?)
    }

    pub fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        let (from, to) = (self.guard(from)?, self.guard(to)?);
        fs::rename(from, to)?;
        Ok(())
    }

    pub fn remove_file(&self, path: &Path) -> Result<()> {
        let path = self.guard(path)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Deletes a directory tree inside app data (never the root itself).
    pub fn remove_dir_all(&self, path: &Path) -> Result<()> {
        let path = self.guard(path)?;
        if path == self.root {
            bail!("refusing to delete the app data root");
        }
        match fs::remove_dir_all(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// True if `p` is inside the app data dir (used by the scanner to skip it).
    pub fn contains(&self, p: &Path) -> bool {
        p.starts_with(&self.root) || dunce_canonicalize(p).map(|c| c.starts_with(&self.root)).unwrap_or(false)
    }
}

/// `fs::canonicalize` without the `\\?\` prefix on Windows, so paths stay
/// comparable with what walkdir / the user see.
pub fn dunce_canonicalize(p: &Path) -> Result<PathBuf> {
    let c = fs::canonicalize(p).with_context(|| format!("canonicalize {}", p.display()))?;
    #[cfg(windows)]
    {
        let s = c.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            if !rest.starts_with("UNC\\") {
                return Ok(PathBuf::from(rest));
            }
        }
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_rejects_outside_paths() {
        let dir = tempfile::tempdir().unwrap();
        let ad = AppData::open(dir.path().join("data")).unwrap();
        assert!(ad.guard(&ad.root().join("thumbs/x.jpg")).is_ok());
        assert!(ad.guard(Path::new("thumbs/x.jpg")).is_ok());
        assert!(ad.guard(&dir.path().join("elsewhere.jpg")).is_err());
        assert!(ad.guard(&ad.root().join("../escape.jpg")).is_err());
        assert!(ad.write_atomic(&dir.path().join("evil.txt"), b"x").is_err());
        assert!(!dir.path().join("evil.txt").exists());
        assert!(ad.remove_dir_all(ad.root()).is_err());
    }

    #[test]
    fn write_atomic_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let ad = AppData::open(dir.path()).unwrap();
        let p = ad.root().join("a/b/c.json");
        ad.write_atomic(&p, b"{}").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"{}");
        ad.write_atomic(&p, b"[1]").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"[1]");
    }
}
