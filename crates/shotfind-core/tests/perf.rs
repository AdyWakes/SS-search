//! Search latency on a large synthetic index (no images needed).
//! Run: cargo test --release -p shotfind-core --test perf -- --ignored --nocapture
use parking_lot::RwLock;
use shotfind_core::db::{Db, IndexedData, ScannedFile};
use shotfind_core::pipeline::TextEmbedder;
use shotfind_core::search::{Filters, Query, Searcher};
use shotfind_core::vectors::VectorIndex;
use std::sync::Arc;
use std::time::Instant;

const WORDS: &[&str] = &[
    "payment",
    "received",
    "transfer",
    "invoice",
    "total",
    "amount",
    "error",
    "warning",
    "server",
    "login",
    "password",
    "meeting",
    "agenda",
    "roadmap",
    "quarterly",
    "review",
    "deploy",
    "build",
    "failed",
    "success",
    "order",
    "shipped",
    "delivery",
    "tracking",
    "address",
    "receipt",
    "subscription",
    "renewal",
    "account",
    "balance",
    "message",
    "reply",
    "photo",
    "video",
    "settings",
    "network",
    "connection",
    "timeout",
    "database",
    "query",
    "customer",
    "support",
];

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn f(&mut self) -> f32 {
        (self.next() % 2000) as f32 / 1000.0 - 1.0
    }
}

struct RandText(Lcg);
impl TextEmbedder for RandText {
    fn embed_text(&mut self, _t: &str) -> anyhow::Result<Vec<f32>> {
        Ok((0..512).map(|_| self.0.f()).collect())
    }
}

#[test]
#[ignore]
fn search_latency_20k() {
    const N: usize = 20_000;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.sqlite3");
    let mut rng = Lcg(42);
    let vectors = Arc::new(RwLock::new(VectorIndex::default()));
    {
        let mut db = Db::open(&path).unwrap();
        let t = Instant::now();
        let mut rows = Vec::with_capacity(N);
        db.tx(|tx| {
            for i in 0..N {
                let p = format!("/u/Pictures/shot{i}.png");
                let f = ScannedFile {
                    path: &p,
                    root: "/u/Pictures",
                    dir: "/u/Pictures",
                    name: &p[12..],
                    size: 1000,
                    mtime_ms: i as i64,
                    is_screenshot: i % 2 == 0,
                };
                rows.push(Db::upsert_scanned(tx, &f, 1)?.id());
            }
            Ok(())
        })
        .unwrap();
        for (i, id) in rows.into_iter().enumerate() {
            let mut text = String::new();
            for l in 0..12 {
                for _ in 0..6 {
                    text.push_str(WORDS[rng.next() as usize % WORDS.len()]);
                    text.push(' ');
                }
                text.push_str(&format!("ID{} {}\n", rng.next() % 100_000, l));
            }
            if i == 12_345 {
                text.push_str("UPI payment to Rahul Sharma\n");
            }
            let v: Vec<f32> = (0..512).map(|_| rng.f()).collect();
            vectors.write().upsert(id, v.clone());
            db.complete(
                id,
                &format!("shot{i}.png"),
                &IndexedData {
                    ocr_text: Some(text),
                    ocr_version: 1,
                    embedding: Some(("m".into(), v)),
                    embed_version: 1,
                    width: 1920,
                    height: 1080,
                    ..Default::default()
                },
            )
            .unwrap();
        }
        eprintln!("built {N} docs in {:?}", t.elapsed());
    }
    let s = Searcher::open(&path, vectors).unwrap();
    s.set_text_embedder(Some(Box::new(RandText(Lcg(7)))));
    let queries = ["UPI payment Rahul", "paymnet", "error login", "quartely revew", "datab", "ID4242", "cat meme", "x"];
    let mut worst = 0f64;
    // Warm-up builds the vocabulary cache once.
    s.search(&Query { text: "warm".into(), ..Default::default() }).unwrap();
    for q in queries {
        for filters in [Filters::default(), Filters { screenshots_only: true, ..Default::default() }] {
            let t = Instant::now();
            let r = s.search(&Query { text: q.into(), filters, limit: 60 }).unwrap();
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            worst = worst.max(ms);
            eprintln!("{q:>20}: {:>3} hits in {ms:6.1} ms", r.hits.len());
        }
    }
    let r = s.search(&Query { text: "UPI payment Rahul".into(), ..Default::default() }).unwrap();
    assert_eq!(r.hits[0].file.name, "shot12345.png");
    eprintln!("worst: {worst:.1} ms");
    assert!(worst < 300.0, "search slower than 300 ms: {worst:.1} ms");
}
