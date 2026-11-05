//! End-to-end tests with the real ONNX models. Ignored by default; CI runs
//! them in the `model-tests` job. Locally:
//!   ORT_DYLIB_PATH=.../libonnxruntime.so SHOTFIND_MODELS_DIR=... cargo test --release -- --ignored
mod common;

use ab_glyph::{FontRef, PxScale};
use image::{Rgb, RgbImage};
use shotfind_core::ocr::PaddleOcr;
use shotfind_core::runtime;
use std::path::PathBuf;

fn models_dir() -> PathBuf {
    PathBuf::from(std::env::var("SHOTFIND_MODELS_DIR").expect("set SHOTFIND_MODELS_DIR"))
}

/// A plain, model-friendly rendering (no ground-truth barcode).
pub fn render(w: u32, h: u32, lines: &[&str], px: f32, dark: bool) -> RgbImage {
    let (bg, fg) =
        if dark { (Rgb([28, 28, 30]), Rgb([230, 230, 230])) } else { (Rgb([255, 255, 255]), Rgb([25, 25, 25])) };
    let mut img = RgbImage::from_pixel(w, h, bg);
    let font = FontRef::try_from_slice(common::FONT).unwrap();
    for (i, l) in lines.iter().enumerate() {
        imageproc::drawing::draw_text_mut(
            &mut img,
            fg,
            30,
            30 + (i as f32 * px * 1.8) as i32,
            PxScale::from(px),
            &font,
            l,
        );
    }
    img
}

/// Every expected word appears in the OCR output, allowing one edit (OCR
/// confuses l/I/1 and the like; search is typo tolerant for the same reason).
fn assert_words(ocr: &str, expected: &str) {
    let toks: Vec<String> = ocr.split(|c: char| !c.is_alphanumeric()).map(|t| t.to_lowercase()).collect();
    for w in expected.split_whitespace() {
        let w = w.to_lowercase();
        let ok = toks.iter().any(|t| *t == w || (w.len() > 3 && strsim::levenshtein(t, &w) <= 1));
        assert!(ok, "word {w:?} not found in OCR output:\n{ocr}");
    }
}

fn ocr() -> PaddleOcr {
    runtime::init(&[]).expect("ONNX Runtime (set ORT_DYLIB_PATH)");
    let d = models_dir().join("ppocr-v4");
    PaddleOcr::load(&d.join("det.onnx"), &d.join("rec.onnx"), None, 2).unwrap()
}

#[test]
#[ignore]
fn ocr_reads_synthetic_screenshots() {
    let mut ocr = ocr();
    let cases: Vec<(RgbImage, &str)> = vec![
        (
            render(1200, 300, &["UPI payment to Rahul", "Amount 1250 rupees"], 28.0, false),
            "UPI payment Rahul Amount 1250",
        ),
        (render(1000, 200, &["Error 404 page not found"], 22.0, true), "Error 404 page not found"),
        (
            render(1920, 1080, &["Quarterly roadmap review", "Ship the search bar first"], 15.0, false),
            "Quarterly roadmap review search",
        ),
    ];
    for (img, expected) in cases {
        use shotfind_core::pipeline::OcrEngine;
        let text = ocr.recognize(&img).unwrap();
        assert_words(&text, expected);
    }
}

#[test]
#[ignore]
fn ocr_handles_tall_scrolling_capture() {
    let lines: Vec<String> = (0..60).map(|i| format!("Line number {i} of the long article")).collect();
    let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    let img = render(1080, 60 * 32 + 60, &refs, 18.0, false);
    use shotfind_core::pipeline::OcrEngine;
    let text = ocr().recognize(&img).unwrap();
    // A lone "0" is genuinely ambiguous with "O"; numbers with more digits are fixed up.
    for i in [1, 17, 33, 59] {
        assert!(text.contains(&format!("Line number {i} ")), "missing line {i}:\n{text}");
    }
}

fn bar_chart() -> RgbImage {
    let mut img = RgbImage::from_pixel(800, 600, Rgb([255, 255, 255]));
    let colors = [Rgb([66, 133, 244]), Rgb([219, 68, 55]), Rgb([244, 180, 0]), Rgb([15, 157, 88])];
    for (i, h) in [300u32, 450, 200, 380].iter().enumerate() {
        let x0 = 100 + i as u32 * 160;
        for y in (550 - h)..550 {
            for x in x0..x0 + 100 {
                img.put_pixel(x, y, colors[i]);
            }
        }
    }
    for x in 80..760 {
        img.put_pixel(x, 551, Rgb([0, 0, 0]));
    }
    img
}

fn sunset() -> RgbImage {
    RgbImage::from_fn(800, 600, |_, y| {
        let t = y as f32 / 600.0;
        if t > 0.7 {
            Rgb([20, 30, 60])
        } else {
            Rgb([(255.0 - 60.0 * t) as u8, (120.0 + 60.0 * t) as u8, (60.0 + 120.0 * t) as u8])
        }
    })
}

#[test]
#[ignore]
fn clip_ranks_matching_concepts() {
    use shotfind_core::models::{is_installed, ModelId};
    use shotfind_core::pipeline::{ImageEmbedder, TextEmbedder};
    let dir = models_dir();
    if !is_installed(&dir, ModelId::Clip) {
        eprintln!("CLIP not installed (hash not pinned yet?) - skipping");
        return;
    }
    runtime::init(&[]).unwrap();
    let d = dir.join("clip-vit-b32-q8");
    let mut vis = shotfind_core::embed::ClipImage::load(&d.join("vision.onnx"), 2).unwrap();
    let mut txt = shotfind_core::embed::ClipText::load(&d.join("text.onnx"), 1).unwrap();
    let doc = render(
        800,
        1000,
        &["Invoice #2024-117", "Item        Qty   Price", "Widget       2    19.99", "Total             39.98"],
        26.0,
        false,
    );
    let imgs = [("chart", bar_chart()), ("sunset", sunset()), ("document", doc)];
    let vecs: Vec<(&str, Vec<f32>)> = imgs.iter().map(|(n, i)| (*n, vis.embed_image(i).unwrap())).collect();
    for (query, want) in
        [("a bar chart", "chart"), ("a sunset over the sea", "sunset"), ("a printed invoice document", "document")]
    {
        let q = txt.embed_text(query).unwrap();
        // Printed for calibrating search::tuning (SEM_FLOOR / SEM_MIN).
        for (n, v) in &vecs {
            eprintln!("cosine {query:?} vs {n}: {:.3}", v.iter().zip(&q).map(|(a, b)| a * b).sum::<f32>());
        }
        let best = vecs
            .iter()
            .map(|(n, v)| (*n, v.iter().zip(&q).map(|(a, b)| a * b).sum::<f32>()))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        assert_eq!(best.0, want, "query {query:?}");
    }
}
