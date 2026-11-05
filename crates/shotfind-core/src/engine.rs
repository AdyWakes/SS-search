//! One handle for the app: settings, indexer, searcher and models.

use crate::appdata::AppData;
use crate::config::Settings;
use crate::db::FileRow;
use crate::engines::OnnxProvider;
use crate::indexer::{Indexer, Listener, Status};
use crate::models::{self, ModelStatus};
use crate::pipeline::EngineProvider;
use crate::search::{Query, SearchResults, Searcher};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

pub struct Engine {
    appdata: AppData,
    indexer: Indexer,
    searcher: Searcher,
    runtime_error: Option<String>,
}

impl Engine {
    /// Opens the real engine. `ort_candidates`: where the bundled ONNX Runtime may be.
    pub fn open(appdata_dir: PathBuf, ort_candidates: &[PathBuf]) -> Result<Engine> {
        let appdata = AppData::open(appdata_dir)?;
        let settings = Settings::load(&appdata);
        let runtime_error = crate::runtime::init(ort_candidates).err().map(|e| format!("{e:#}"));
        if let Some(e) = &runtime_error {
            tracing::error!("ONNX Runtime unavailable; indexing file names only: {e}");
        }
        let provider = Arc::new(OnnxProvider::new(&appdata.models_dir(), settings.threads_per_worker()));
        let mut e = Engine::open_with_provider(appdata, settings, provider)?;
        e.runtime_error = runtime_error;
        Ok(e)
    }

    /// Opens with an arbitrary engine provider (tests use fake engines).
    pub fn open_with_provider(
        appdata: AppData,
        settings: Settings,
        provider: Arc<dyn EngineProvider>,
    ) -> Result<Engine> {
        let semantic = settings.semantic_enabled;
        let text = if semantic { provider.new_text_embedder()? } else { None };
        let indexer = Indexer::start(appdata.clone(), settings, provider)?;
        let searcher = Searcher::open(&appdata.db_path(), indexer.shared().vectors.clone())?;
        searcher.set_text_embedder(text);
        Ok(Engine { appdata, indexer, searcher, runtime_error: None })
    }

    pub fn appdata(&self) -> &AppData {
        &self.appdata
    }
    pub fn runtime_error(&self) -> Option<&str> {
        self.runtime_error.as_deref()
    }

    pub fn search(&self, q: &Query) -> Result<SearchResults> {
        self.searcher.search(q)
    }
    pub fn searcher(&self) -> &Searcher {
        &self.searcher
    }
    pub fn file(&self, id: i64) -> Result<Option<FileRow>> {
        self.searcher.file(id)
    }
    pub fn ocr_text(&self, id: i64) -> Result<Option<String>> {
        self.searcher.ocr_text(id)
    }
    pub fn thumb_path(&self, id: i64) -> PathBuf {
        self.appdata.thumb_path(id)
    }

    pub fn status(&self) -> Status {
        self.indexer.status()
    }
    pub fn set_listener(&self, l: Listener) {
        self.indexer.set_listener(l)
    }
    pub fn indexer(&self) -> &Indexer {
        &self.indexer
    }

    pub fn settings(&self) -> Settings {
        self.indexer.shared().settings.read().clone()
    }

    pub fn update_settings(&self, s: Settings) -> Result<()> {
        let semantic_changed = s.semantic_enabled != self.settings().semantic_enabled;
        self.indexer.update_settings(s.clone())?;
        if semantic_changed {
            let p = self.indexer.provider();
            self.searcher.set_text_embedder(if s.semantic_enabled { p.new_text_embedder()? } else { None });
        }
        Ok(())
    }

    pub fn models(&self) -> Vec<ModelStatus> {
        models::statuses(&self.appdata.models_dir())
    }

    /// Re-reads the models dir (after a download/import) and swaps engines in.
    pub fn reload_models(&self) -> Result<()> {
        let settings = self.settings();
        let p = Arc::new(OnnxProvider::new(&self.appdata.models_dir(), settings.threads_per_worker()));
        let text = if settings.semantic_enabled { p.new_text_embedder()? } else { None };
        self.indexer.set_provider(p)?;
        self.searcher.set_text_embedder(text);
        Ok(())
    }

    pub fn shutdown(&mut self) {
        self.indexer.shutdown();
    }
}
