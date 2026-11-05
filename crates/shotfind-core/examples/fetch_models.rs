//! Downloads and verifies all models into `<dir>/models` using the same code
//! path as the app. Used by CI. Usage: cargo run --example fetch_models -- <appdata-dir>
use shotfind_core::{appdata::AppData, models};
use std::sync::atomic::AtomicBool;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("target dir");
    let ad = AppData::open(&dir)?;
    // Optional list of model ids (ocr, clip); default: every model with pinned hashes.
    let wanted: Vec<String> = args.collect();
    let ids: Vec<models::ModelId> = models::manifest()
        .iter()
        .filter(|m| {
            if wanted.is_empty() {
                m.is_pinned()
            } else {
                wanted.iter().any(|w| format!("{:?}", m.id).eq_ignore_ascii_case(w))
            }
        })
        .map(|m| m.id)
        .collect();
    for m in models::manifest().iter().filter(|m| !ids.contains(&m.id)) {
        eprintln!("skipping {:?} (not requested or hash not pinned)", m.id);
    }
    let cancel = AtomicBool::new(false);
    let mut last = 0u64;
    models::install(&ad, &ids, &cancel, &mut |p| {
        let pct = (p.overall_downloaded * 100).checked_div(p.overall_total).unwrap_or(0);
        if pct != last {
            last = pct;
            eprintln!("{:?} {} {} {}%", p.model, p.file, p.stage, pct);
        }
    })?;
    for s in models::statuses(&ad.models_dir()) {
        println!("{:?}: installed={}", s.id, s.installed);
    }
    println!("models dir: {}", ad.models_dir().display());
    Ok(())
}
