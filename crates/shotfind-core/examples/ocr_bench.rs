//! Runs OCR on images and prints text + timings.
//! Usage: ORT_DYLIB_PATH=... cargo run --release --example ocr_bench -- <ocr-model-dir> <image>...
use shotfind_core::{imgutil, ocr::PaddleOcr, runtime};
use std::path::PathBuf;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("model dir"));
    runtime::init(&[])?;
    let threads: usize = std::env::var("THREADS").ok().and_then(|s| s.parse().ok()).unwrap_or(2);
    let t = Instant::now();
    let mut opts = shotfind_core::ocr::OcrOptions::default();
    if let Some(v) = std::env::var("DET_MAX").ok().and_then(|s| s.parse().ok()) {
        opts.det_max_side = v;
    }
    if let Some(v) = std::env::var("REC_BATCH").ok().and_then(|s| s.parse().ok()) {
        opts.rec_batch = v;
    }
    if let Some(v) = std::env::var("REC_H").ok().and_then(|s| s.parse().ok()) {
        opts.rec_height = v;
    }
    let mut ocr = PaddleOcr::load(&dir.join("det.onnx"), &dir.join("rec.onnx"), None, threads)?.with_options(opts);
    println!("load: {:?} {}", t.elapsed(), peak_rss());
    for p in args {
        let bytes = std::fs::read(&p)?;
        let img = imgutil::decode_rgb(&bytes, 100_000_000)?;
        let t = Instant::now();
        let boxes = ocr.detect(&img)?;
        let td = t.elapsed();
        println!("after det: {}", peak_rss());
        let t = Instant::now();
        let lines = ocr.read_lines(&img)?;
        let total = t.elapsed();
        println!("== {p} {}x{}: {} boxes, det {:?}, full {:?}", img.width(), img.height(), boxes.len(), td, total);
        for l in &lines {
            println!("  [{:.2}] {}", l.confidence, l.text);
        }
    }
    println!("peak memory: {}", peak_rss());
    Ok(())
}

#[allow(dead_code)]
fn peak_rss() -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmHWM")).map(|l| l.to_string()))
        .unwrap_or_default()
}
