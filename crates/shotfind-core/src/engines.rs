//! The real engine provider: ONNX models from the app data `models/` dir.

use crate::models::{self, ModelId};
use crate::ocr::{PaddleOcr, OCR_VERSION};
use crate::pipeline::{EngineProvider, ImageEmbedder, OcrEngine, TextEmbedder};
use anyhow::Result;
use image::RgbImage;
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

pub struct OnnxProvider {
    models_dir: PathBuf,
    threads: usize,
    ocr: bool,
    clip: bool,
    /// One CLIP vision session shared by all workers (it is ~10% of the work
    /// per file, so sharing costs little and saves a model copy per worker).
    /// Weak: freed when every worker has released it.
    shared_clip: Mutex<Weak<Mutex<crate::embed::ClipImage>>>,
}

struct SharedClip(Arc<Mutex<crate::embed::ClipImage>>);
impl ImageEmbedder for SharedClip {
    fn embed_image(&mut self, img: &RgbImage) -> Result<Vec<f32>> {
        self.0.lock().embed_image(img)
    }
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
            shared_clip: Mutex::new(Weak::new()),
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
        let mut slot = self.shared_clip.lock();
        let clip = match slot.upgrade() {
            Some(c) => c,
            None => {
                let c = Arc::new(Mutex::new(crate::embed::ClipImage::load(
                    &self.file(ModelId::Clip, "vision.onnx"),
                    self.threads,
                )?));
                *slot = Arc::downgrade(&c);
                c
            }
        };
        Ok(Some(Box::new(SharedClip(clip))))
    }

    fn new_text_embedder(&self) -> Result<Option<Box<dyn TextEmbedder>>> {
        if !self.clip {
            return Ok(None);
        }
        // Query-time: one thread is plenty for a 77-token text encoder.
        Ok(Some(Box::new(crate::embed::ClipText::load(&self.file(ModelId::Clip, "text.onnx"), 1)?)))
    }
}
