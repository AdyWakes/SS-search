//! Scan + index throughput on a large folder (fake engines, real decode,
//! thumbnails and SQLite). Run: cargo test --release --test large_folder -- --ignored --nocapture
mod common;

use common::*;
use shotfind_core::appdata::AppData;
use shotfind_core::indexer::Indexer;
use std::time::{Duration, Instant};

#[test]
#[ignore]
fn ten_thousand_files() {
    const N: usize = 10_000;
    let fx = Fixture::new();
    let img = synth_image(1280, 720, &["Quarterly report", "Revenue up 12 percent"], "chart");
    let mut png = Vec::new();
    image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    for i in 0..N {
        let dir = fx.user.join(format!("d{}", i / 500));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("shot{i}.png")), &png).unwrap();
    }
    let mut s = fx.settings();
    s.workers = 4;
    let t = Instant::now();
    let ix = Indexer::start(AppData::open(&fx.data).unwrap(), s, FakeProvider::new()).unwrap();
    while ix.status().last_scan.is_none() {
        std::thread::sleep(Duration::from_millis(10));
    }
    let scan = t.elapsed();
    assert!(ix.wait_idle(Duration::from_secs(900)));
    let total = t.elapsed();
    let st = ix.status();
    eprintln!(
        "scan of {N} files: {scan:?}; full index: {total:?} ({:.0} files/s with fake OCR)",
        N as f64 / total.as_secs_f64()
    );
    assert_eq!(st.counts.done as usize, N);
    // Rescan of an unchanged library must be fast (no re-indexing).
    let t = Instant::now();
    ix.rescan();
    std::thread::sleep(Duration::from_millis(100));
    assert!(ix.wait_idle(Duration::from_secs(120)));
    eprintln!("rescan unchanged: {:?}", t.elapsed());
}
