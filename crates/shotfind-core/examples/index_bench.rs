//! Real-engine indexing throughput.
//! Usage: ORT_DYLIB_PATH=... cargo run --release --example index_bench -- <appdata-with-models> <images-folder> [gentle|balanced|fast]
use shotfind_core::config::{Folder, FolderKind, IndexingSpeed, Settings};
use shotfind_core::engine::Engine;
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let mut a = std::env::args().skip(1);
    let data = std::path::PathBuf::from(a.next().expect("appdata dir"));
    let folder = std::path::PathBuf::from(a.next().expect("images dir"));
    let speed = match a.next().as_deref() {
        Some("gentle") => IndexingSpeed::Gentle,
        Some("fast") => IndexingSpeed::Fast,
        _ => IndexingSpeed::Balanced,
    };
    let ad = shotfind_core::appdata::AppData::open(&data)?;
    let s = Settings {
        folders: vec![Folder { path: folder, kind: FolderKind::Screenshots, enabled: true }],
        indexing_speed: speed,
        models_consented: true,
        ..Default::default()
    };
    s.save(&ad)?;
    println!("workers={} threads/worker={}", s.effective_workers(), s.threads_per_worker());
    let t = Instant::now();
    let e = Engine::open(data, &[])?;
    let st = e.status();
    println!("ocr={} clip={}", st.ocr_available, st.semantic_available);
    e.indexer().wait_idle(Duration::from_secs(3600));
    let st = e.status();
    let secs = t.elapsed().as_secs_f64();
    println!(
        "{} files in {:.1}s = {:.2} s/file, {:.0} files/min",
        st.counts.done,
        secs,
        secs / st.counts.done as f64,
        st.counts.done as f64 * 60.0 / secs
    );
    let r = e.search(&shotfind_core::search::Query { text: "nginx config".into(), ..Default::default() })?;
    println!("search: {} hits in {:.1} ms", r.hits.len(), r.took_ms);
    Ok(())
}
