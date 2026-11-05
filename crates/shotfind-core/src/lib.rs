//! shotfind-core: offline indexing + search for screenshots and images.
//!
//! The crate never writes to, moves, or deletes user images. All writes go to
//! the app data directory (see [`appdata`]).

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
