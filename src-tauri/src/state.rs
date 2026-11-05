use shotfind_core::engine::Engine;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub struct AppState {
    pub engine: Arc<Engine>,
    pub download_cancel: Arc<AtomicBool>,
    pub downloading: Arc<AtomicBool>,
}

impl AppState {
    pub fn new(engine: Arc<Engine>) -> Self {
        Self { engine, download_cancel: Default::default(), downloading: Default::default() }
    }
}
