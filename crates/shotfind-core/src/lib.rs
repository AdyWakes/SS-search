//! shotfind-core: offline indexing + search for screenshots and images.
//!
//! The crate never writes to, moves, or deletes user images. User files are
//! opened read-only via [`userfs`]; every write goes through [`appdata::AppData`],
//! which refuses paths outside the app data directory.

pub mod appdata;
pub mod config;
pub mod db;
pub mod embed;
pub mod engine;
pub mod engines;
pub mod imgutil;
pub mod indexer;
pub mod models;
pub mod ocr;
pub mod pipeline;
pub mod runtime;
pub mod scanner;
pub mod search;
pub mod textnorm;
pub mod userfs;
pub mod vectors;
pub mod watcher;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
