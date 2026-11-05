//! Test helpers: synthetic screenshots with known text, fake engines, and a
//! file-tree snapshot for the never-modify guarantee.
#![allow(dead_code)]

use ab_glyph::{FontRef, PxScale};
use anyhow::Result;
use image::{Rgb, RgbImage};
use sha2::{Digest, Sha256};
use shotfind_core::pipeline::{EngineProvider, ImageEmbedder, OcrEngine, TextEmbedder};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub const FONT: &[u8] = include_bytes!("../fixtures/DejaVuSans.ttf");

/// Concepts the fake embedder knows. Index = dimension of a one-hot vector.
pub const CONCEPTS: &[&str] = &["none", "cat", "receipt", "whiteboard", "chart", "code"];

/// Renders a screenshot-like image with `lines` of visible text. The ground
/// truth ("line1\nline2|concept") is also encoded in the first pixel row so the
/// fake OCR / embedder can read it back without ML models.
pub fn synth_image(w: u32, h: u32, lines: &[&str], concept: &str) -> RgbImage {
    let mut img = RgbImage::from_pixel(w, h, Rgb([250, 250, 250]));
    let font = FontRef::try_from_slice(FONT).unwrap();
    let size = (h as f32 / (lines.len() as f32 + 3.0)).clamp(14.0, 48.0);
    for (i, line) in lines.iter().enumerate() {
        let y = (size * 0.8 + i as f32 * size * 1.4) as i32;
        imageproc::drawing::draw_text_mut(&mut img, Rgb([20, 20, 20]), 24, y, PxScale::from(size), &font, line);
    }
    let payload = format!("{}|{}", lines.join("\n"), concept);
    encode_payload(&mut img, payload.as_bytes());
    img
}

pub fn write_png(path: &Path, img: &RgbImage) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    img.save_with_format(path, image::ImageFormat::Png).unwrap();
}

pub fn write_jpg(path: &Path, img: &RgbImage) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    img.save_with_format(path, image::ImageFormat::Jpeg).unwrap();
}

/// Ground truth is stored as a JPEG-proof barcode: 8x8 black/white blocks
/// (aligned to the JPEG DCT grid), bottom rows of the image, 16-bit length prefix.
const BLOCK: u32 = 8;

fn encode_payload(img: &mut RgbImage, bytes: &[u8]) {
    let mut bits = Vec::new();
    let len = bytes.len() as u16;
    for i in (0..16).rev() {
        bits.push((len >> i) & 1 == 1);
    }
    for b in bytes {
        for i in (0..8).rev() {
            bits.push((b >> i) & 1 == 1);
        }
    }
    let per_row = (img.width() / BLOCK) as usize;
    let rows = bits.len().div_ceil(per_row) as u32;
    if rows * BLOCK * 2 > img.height() {
        return; // tiny images (icons) carry no payload
    }
    let top = (img.height() / BLOCK - rows) * BLOCK;
    for (i, bit) in bits.iter().enumerate() {
        let bx = (i % per_row) as u32 * BLOCK;
        let by = top + (i / per_row) as u32 * BLOCK;
        let v = if *bit { 0 } else { 255 };
        for y in by..by + BLOCK {
            for x in bx..bx + BLOCK {
                img.put_pixel(x, y, Rgb([v, v, v]));
            }
        }
    }
}

fn decode_payload(img: &RgbImage) -> (String, String) {
    let per_row = (img.width() / BLOCK) as usize;
    let read_bits = |n: usize, rows: u32| -> Vec<bool> {
        let top = (img.height() / BLOCK - rows) * BLOCK;
        (0..n)
            .map(|i| {
                let x = (i % per_row) as u32 * BLOCK + BLOCK / 2;
                let y = top + (i / per_row) as u32 * BLOCK + BLOCK / 2;
                img.get_pixel(x, y)[0] < 128
            })
            .collect()
    };
    // Length is unknown until read; try each plausible row count.
    for rows in 1..=(img.height() / BLOCK).min(64) {
        let head = read_bits(16.min(per_row * rows as usize), rows);
        if head.len() < 16 {
            continue;
        }
        let len = head.iter().fold(0usize, |a, b| (a << 1) | *b as usize);
        let total = 16 + len * 8;
        if total.div_ceil(per_row) as u32 != rows {
            continue;
        }
        let bits = read_bits(total, rows);
        let bytes: Vec<u8> = bits[16..].chunks(8).map(|c| c.iter().fold(0u8, |a, b| (a << 1) | *b as u8)).collect();
        let s = String::from_utf8_lossy(&bytes).into_owned();
        return match s.rsplit_once('|') {
            Some((t, c)) => (t.to_string(), c.to_string()),
            None => (s, "none".into()),
        };
    }
    (String::new(), "none".into())
}

pub fn concept_vec(concept: &str) -> Vec<f32> {
    let mut v = vec![0.05f32; CONCEPTS.len()];
    let i = CONCEPTS.iter().position(|c| *c == concept).unwrap_or(0);
    v[i] = 1.0;
    v
}

#[derive(Default)]
pub struct Counters {
    pub ocr_calls: AtomicUsize,
    pub embed_calls: AtomicUsize,
}

pub struct FakeOcr {
    counters: Arc<Counters>,
    panic_on: Option<String>,
}
impl OcrEngine for FakeOcr {
    fn recognize(&mut self, img: &RgbImage) -> Result<String> {
        self.counters.ocr_calls.fetch_add(1, Ordering::SeqCst);
        let (text, _) = decode_payload(img);
        if let Some(p) = &self.panic_on {
            if text.contains(p.as_str()) {
                panic!("simulated OCR crash");
            }
        }
        Ok(text)
    }
}

pub struct FakeEmbed(Arc<Counters>);
impl ImageEmbedder for FakeEmbed {
    fn embed_image(&mut self, img: &RgbImage) -> Result<Vec<f32>> {
        self.0.embed_calls.fetch_add(1, Ordering::SeqCst);
        Ok(concept_vec(&decode_payload(img).1))
    }
}

pub struct FakeText;
impl TextEmbedder for FakeText {
    fn embed_text(&mut self, text: &str) -> Result<Vec<f32>> {
        let t = text.to_lowercase();
        let c = CONCEPTS.iter().skip(1).find(|c| t.contains(*c)).copied().unwrap_or("none");
        Ok(concept_vec(c))
    }
}

pub struct FakeProvider {
    pub counters: Arc<Counters>,
    pub ocr: bool,
    pub embed: bool,
    pub ocr_version: i64,
    pub panic_on: Option<String>,
}

impl FakeProvider {
    pub fn new() -> Arc<FakeProvider> {
        Arc::new(FakeProvider { counters: Default::default(), ocr: true, embed: true, ocr_version: 1, panic_on: None })
    }
    pub fn ocr_only() -> Arc<FakeProvider> {
        Arc::new(FakeProvider { counters: Default::default(), ocr: true, embed: false, ocr_version: 1, panic_on: None })
    }
}

impl EngineProvider for FakeProvider {
    fn ocr_version(&self) -> i64 {
        if self.ocr {
            self.ocr_version
        } else {
            0
        }
    }
    fn embed_version(&self) -> i64 {
        i64::from(self.embed)
    }
    fn embed_model_id(&self) -> String {
        "fake-concepts".into()
    }
    fn new_ocr(&self) -> Result<Option<Box<dyn OcrEngine>>> {
        Ok(self.ocr.then(|| {
            Box::new(FakeOcr { counters: self.counters.clone(), panic_on: self.panic_on.clone() }) as Box<dyn OcrEngine>
        }))
    }
    fn new_image_embedder(&self) -> Result<Option<Box<dyn ImageEmbedder>>> {
        Ok(self.embed.then(|| Box::new(FakeEmbed(self.counters.clone())) as Box<dyn ImageEmbedder>))
    }
    fn new_text_embedder(&self) -> Result<Option<Box<dyn TextEmbedder>>> {
        Ok(self.embed.then(|| Box::new(FakeText) as Box<dyn TextEmbedder>))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    pub sha256: String,
    pub len: u64,
    pub mtime: Option<std::time::SystemTime>,
    pub readonly: bool,
}

/// Content hash + metadata of every file and dir under `root`.
pub fn snapshot(root: &Path) -> BTreeMap<PathBuf, Entry> {
    let mut out = BTreeMap::new();
    for e in walkdir::WalkDir::new(root).into_iter().flatten() {
        let meta = e.metadata().unwrap();
        let sha =
            if meta.is_file() { hex::encode(Sha256::digest(std::fs::read(e.path()).unwrap())) } else { "dir".into() };
        out.insert(
            e.path().to_path_buf(),
            Entry {
                sha256: sha,
                len: if meta.is_file() { meta.len() } else { 0 },
                mtime: if meta.is_file() { meta.modified().ok() } else { None },
                readonly: meta.permissions().readonly(),
            },
        );
    }
    out
}

/// A user folder with a known set of images.
pub struct Fixture {
    pub dir: tempfile::TempDir,
    pub user: PathBuf,
    pub data: PathBuf,
}

impl Fixture {
    pub fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("Pictures");
        let data = dir.path().join("appdata");
        std::fs::create_dir_all(&user).unwrap();
        Fixture { dir, user, data }
    }

    pub fn settings(&self) -> shotfind_core::config::Settings {
        shotfind_core::config::Settings {
            folders: vec![shotfind_core::config::Folder {
                path: shotfind_core::appdata::dunce_canonicalize(&self.user).unwrap(),
                kind: shotfind_core::config::FolderKind::Pictures,
                enabled: true,
            }],
            min_file_bytes: 64,
            workers: 2,
            ..Default::default()
        }
    }

    pub fn standard_set(&self) {
        write_png(
            &self.user.join("Screenshot 2024-05-01 UPI.png"),
            &synth_image(900, 500, &["UPI payment to Rahul", "Amount 1,250.00"], "receipt"),
        );
        write_png(&self.user.join("error.png"), &synth_image(800, 400, &["Error 404 page not found"], "code"));
        write_jpg(
            &self.user.join("sub/meeting.jpg"),
            &synth_image(800, 600, &["Quarterly roadmap", "Q3 goals"], "whiteboard"),
        );
        write_png(&self.user.join("sub/deeper/cat.png"), &synth_image(600, 400, &["so fluffy"], "cat"));
        // Skip rules:
        write_png(&self.user.join("icon.png"), &synth_image(40, 40, &[], "none")); // too small (icon)
        std::fs::write(self.user.join("notes.txt"), "not an image").unwrap(); // wrong type
        write_png(&self.user.join(".hidden/secret.png"), &synth_image(600, 400, &["hidden"], "none")); // hidden dir
        write_png(&self.user.join("node_modules/pkg/logo.png"), &synth_image(600, 400, &["npm"], "none"));
        std::fs::write(self.user.join("broken.png"), [0x89, b'P', b'N', b'G', 0, 1, 2, 3].repeat(40)).unwrap();
    }
}
