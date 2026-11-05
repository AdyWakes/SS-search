//! Read-only access to user files. This module is the only code that touches
//! user images, and it only ever opens them for reading.

use anyhow::{bail, Context, Result};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::Path;
use std::time::UNIX_EPOCH;

/// Opens a user file strictly read-only.
pub fn open_read(path: &Path) -> Result<File> {
    OpenOptions::new().read(true).open(path).with_context(|| format!("opening {}", path.display()))
}

/// Reads a whole user file into memory, refusing anything over `max_bytes`.
pub fn read_limited(path: &Path, max_bytes: u64) -> Result<Vec<u8>> {
    let f = open_read(path)?;
    let len = f.metadata()?.len();
    if len > max_bytes {
        bail!("file is {} bytes, over the {} byte limit", len, max_bytes);
    }
    let mut buf = Vec::with_capacity(len as usize);
    f.take(max_bytes + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > max_bytes {
        bail!("file grew past the size limit while reading");
    }
    Ok(buf)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub size: u64,
    /// Modification time, unix milliseconds.
    pub mtime_ms: i64,
}

pub fn stamp(meta: &std::fs::Metadata) -> FileStamp {
    let mtime_ms =
        meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0);
    FileStamp { size: meta.len(), mtime_ms }
}

pub fn stat(path: &Path) -> Result<FileStamp> {
    Ok(stamp(&std::fs::metadata(path)?))
}
