//! The real engine provider: ONNX models from the app data `models/` dir.

use crate::models::{self, ModelId};
use crate::ocr::{PaddleOcr, OCR_VERSION};
use crate::pipeline::{EngineProvider, ImageEmbedder, OcrEngine, TextEmbedder};
use anyhow::Result;
use std::path::{Path, PathBuf};

pub struct OnnxProvider {
    models_dir: PathBuf,
    threads: usize,
    ocr: bool,
    clip: bool,
}

impl OnnxProvider {
    /// Inspects `models_dir` and enables whichever pipelines are installed and verified.
    pub fn new(models_dir: &Path, threads_per_worker: usize) -> OnnxProvider {
        let ready = crate::runtime::is_ready();
        OnnxProvider {
            models_dir: models_dir.to_path_buf(),
            threads: threads_per_worker.max(1),
            ocr: ready && models::is_installed(models_dir, ModelId::Ocr),
            clip: ready && models::is_installed(models_dir, ModelId::Clip),
        }
    }

    pub fn ocr_ready(&self) -> bool {
        self.ocr
    }
    pub fn clip_ready(&self) -> bool {
        self.clip
    }

    fn file(&self, id: ModelId, name: &str) -> PathBuf {
        models::model_dir(&self.models_dir, id).join(name)
    }
}

impl EngineProvider for OnnxProvider {
    fn ocr_version(&self) -> i64 {
        if self.ocr {
            OCR_VERSION
        } else {
            0
        }
    }

    fn embed_version(&self) -> i64 {
        if self.clip {
            crate::embed::EMBED_VERSION
        } else {
            0
        }
    }

    fn embed_model_id(&self) -> String {
        if self.clip {
            crate::embed::MODEL_ID.to_string()
        } else {
            String::new()
        }
    }

    fn new_ocr(&self) -> Result<Option<Box<dyn OcrEngine>>> {
        if !self.ocr {
            return Ok(None);
        }
        let o = PaddleOcr::load(
            &self.file(ModelId::Ocr, "det.onnx"),
            &self.file(ModelId::Ocr, "rec.onnx"),
            None,
            self.threads,
        )?;
        Ok(Some(Box::new(o)))
    }

    fn new_image_embedder(&self) -> Result<Option<Box<dyn ImageEmbedder>>> {
        if !self.clip {
            return Ok(None);
        }
        Ok(Some(Box::new(crate::embed::ClipImage::load(&self.file(ModelId::Clip, "vision.onnx"), self.threads)?)))
    }

    fn new_text_embedder(&self) -> Result<Option<Box<dyn TextEmbedder>>> {
        if !self.clip {
            return Ok(None);
        }
        // Query-time: one thread is plenty for a 77-token text encoder.
        Ok(Some(Box::new(crate::embed::ClipText::load(&self.file(ModelId::Clip, "text.onnx"), 1)?)))
    }
}
