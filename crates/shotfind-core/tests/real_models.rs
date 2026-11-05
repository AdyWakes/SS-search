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
