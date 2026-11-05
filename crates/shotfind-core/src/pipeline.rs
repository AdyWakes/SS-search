//! Per-file processing: read (read-only), decode, thumbnail, OCR, embed.

use crate::appdata::AppData;
use crate::config::Settings;
use crate::db::{IndexedData, PendingFile};
use crate::imgutil;
use crate::userfs;
use anyhow::Result;
use image::RgbImage;
use std::path::Path;

/// Longest side of cached thumbnails, in pixels (sharp on HiDPI grids).
pub const THUMB_MAX_SIDE: u32 = 384;
const THUMB_QUALITY: u8 = 78;

pub trait OcrEngine: Send {
    /// Returns the recognised text, one line per text line, in reading order.
    fn recognize(&mut self, img: &RgbImage) -> Result<String>;
}

pub trait ImageEmbedder: Send {
    fn embed_image(&mut self, img: &RgbImage) -> Result<Vec<f32>>;
}

pub trait TextEmbedder: Send {
    fn embed_text(&mut self, text: &str) -> Result<Vec<f32>>;
}

/// Creates engines. Each indexing worker gets its own instances.
pub trait EngineProvider: Send + Sync {
    /// Version of the OCR pipeline; 0 means OCR is unavailable.
    fn ocr_version(&self) -> i64 {
        0
    }
    /// Version of the embedding pipeline; 0 means unavailable / disabled.
    fn embed_version(&self) -> i64 {
        0
    }
    /// Identifies the embedding space (vectors from different models never mix).
    fn embed_model_id(&self) -> String {
        String::new()
    }
    fn new_ocr(&self) -> Result<Option<Box<dyn OcrEngine>>> {
        Ok(None)
    }
    fn new_image_embedder(&self) -> Result<Option<Box<dyn ImageEmbedder>>> {
        Ok(None)
    }
    fn new_text_embedder(&self) -> Result<Option<Box<dyn TextEmbedder>>> {
        Ok(None)
    }
}

/// No models installed: files are still registered and searchable by name.
pub struct NoEngines;
impl EngineProvider for NoEngines {}

#[derive(Default)]
pub struct WorkerEngines {
    pub ocr: Option<Box<dyn OcrEngine>>,
    pub embed: Option<Box<dyn ImageEmbedder>>,
}

#[derive(Debug, Clone)]
pub struct Job {
    pub file: PendingFile,
    pub need_ocr: bool,
    pub need_embed: bool,
    pub ocr_version: i64,
    pub embed_version: i64,
    pub embed_model: String,
}

#[derive(Debug)]
pub enum Outcome {
    Done(IndexedData),
    Skipped { reason: String, dims: Option<(u32, u32)> },
    Failed(String),
}

pub fn process(job: &Job, settings: &Settings, appdata: &AppData, engines: &mut WorkerEngines) -> Outcome {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| process_inner(job, settings, appdata, engines))) {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => Outcome::Failed(format!("{e:#}")),
        Err(p) => {
            let msg = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".into());
            Outcome::Failed(format!("internal error while indexing: {msg}"))
        }
    }
}

fn process_inner(job: &Job, settings: &Settings, appdata: &AppData, engines: &mut WorkerEngines) -> Result<Outcome> {
    let path = Path::new(&job.file.path);
    // Opened read-only. Never written, moved, or deleted.
    let bytes = userfs::read_limited(path, settings.max_file_bytes)?;
    let (w, h) = match imgutil::dimensions(&bytes) {
        Ok(d) => d,
        Err(e) => return Ok(Outcome::Skipped { reason: format!("not a readable image: {e}"), dims: None }),
    };
    if w.min(h) < settings.min_dimension {
        return Ok(Outcome::Skipped { reason: "too small (icon)".into(), dims: Some((w, h)) });
    }
    if (w as u64) * (h as u64) > settings.max_pixels {
        return Ok(Outcome::Skipped { reason: "too many pixels".into(), dims: Some((w, h)) });
    }
    let img = imgutil::decode_rgb(&bytes, settings.max_pixels)?;
    drop(bytes);

    let thumb = appdata.thumb_path(job.file.id);
    let fresh = job.file.ocr_version == 0 && job.file.embed_version == 0;
    if fresh || !thumb.exists() {
        let t = imgutil::fit_within(&img, THUMB_MAX_SIDE);
        appdata.write_atomic(&thumb, &imgutil::encode_jpeg(&t, THUMB_QUALITY)?)?;
    }

    let mut data = IndexedData {
        width: w,
        height: h,
        is_screenshot: imgutil::looks_like_screen_dims(w, h) && is_lossless(path),
        ..Default::default()
    };
    if job.need_ocr {
        if let Some(ocr) = engines.ocr.as_mut() {
            data.ocr_text = Some(ocr.recognize(&img)?);
            data.ocr_version = job.ocr_version;
        }
    }
    if job.need_embed {
        if let Some(emb) = engines.embed.as_mut() {
            data.embedding = Some((job.embed_model.clone(), emb.embed_image(&img)?));
            data.embed_version = job.embed_version;
        }
    }
    Ok(Outcome::Done(data))
}

fn is_lossless(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "png" | "webp" | "bmp"))
}
