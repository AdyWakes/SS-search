//! Search and ranking with synthetic images and fake (deterministic) engines.
mod common;

use common::*;
use shotfind_core::appdata::AppData;
use shotfind_core::engine::Engine;
use shotfind_core::search::{Filters, MatchKind, Query};
use std::time::Duration;

fn engine(fx: &Fixture) -> Engine {
    let e = Engine::open_with_provider(AppData::open(&fx.data).unwrap(), fx.settings(), FakeProvider::new()).unwrap();
    assert!(e.indexer().wait_idle(Duration::from_secs(30)), "{:?}", e.status());
    e
}

fn names(e: &Engine, q: &str) -> Vec<String> {
    names_f(e, q, Filters::default())
}

fn names_f(e: &Engine, q: &str, filters: Filters) -> Vec<String> {
    e.search(&Query { text: q.into(), filters, limit: 20 }).unwrap().hits.into_iter().map(|h| h.file.name).collect()
}

fn library() -> Fixture {
    let fx = Fixture::new();
    fx.standard_set();
    // Distractors that share some words.
    write_png(
        &fx.user.join("payments-overview.png"),
        &synth_image(900, 400, &["Payment history", "No pending transfers"], "chart"),
    );
    write_png(&fx.user.join("rahul-birthday.png"), &synth_image(700, 400, &["Happy birthday Rahul"], "none"));
    write_png(
        &fx.user.join("Screenshot 2023-01-02 error.png"),
        &synth_image(900, 400, &["Error 401 unauthorized"], "code"),
    );
    write_png(&fx.user.join("whiteboard.png"), &synth_image(800, 600, &["sprint retro"], "whiteboard"));
    fx
}

#[test]
fn exact_multiword_text_ranks_first() {
    let fx = library();
    let e = engine(&fx);
    let r = names(&e, "UPI payment Rahul");
    assert_eq!(r[0], "Screenshot 2024-05-01 UPI.png", "{r:?}");
    // Partial matches still appear, below.
    assert!(r.contains(&"payments-overview.png".to_string()) || r.contains(&"rahul-birthday.png".to_string()), "{r:?}");
}

#[test]
fn typos_are_tolerated() {
    let fx = library();
    let e = engine(&fx);
    assert_eq!(names(&e, "paymnet rahull")[0], "Screenshot 2024-05-01 UPI.png");
    assert_eq!(names(&e, "eror 404")[0], "error.png");
    let res = e.search(&Query { text: "quartely roadmap".into(), ..Default::default() }).unwrap();
    assert_eq!(res.hits[0].file.name, "meeting.jpg");
    assert!(res.corrections.iter().any(|(a, b)| a == "quartely" && b == "quarterly"), "{:?}", res.corrections);
}

#[test]
fn numbers_must_match_exactly() {
    let fx = library();
    let e = engine(&fx);
    let r = names(&e, "error 404");
    assert_eq!(r[0], "error.png");
    let r401 = names(&e, "401");
    assert_eq!(r401, vec!["Screenshot 2023-01-02 error.png"], "404 must not match 401");
}

#[test]
fn prefix_matches_while_typing() {
    let fx = library();
    let e = engine(&fx);
    assert_eq!(names(&e, "quarter")[0], "meeting.jpg");
    assert_eq!(names(&e, "unauth")[0], "Screenshot 2023-01-02 error.png");
}

#[test]
fn meaning_search_finds_images_without_matching_text() {
    let fx = library();
    let e = engine(&fx);
    let res = e.search(&Query { text: "cat meme".into(), ..Default::default() }).unwrap();
    assert!(res.semantic_used);
    assert_eq!(res.hits[0].file.name, "cat.png"); // (its file name also says "cat": kind Both)
    let res = e.search(&Query { text: "whiteboard notes".into(), ..Default::default() }).unwrap();
    let top2: Vec<&str> = res.hits[..2].iter().map(|h| h.file.name.as_str()).collect();
    assert!(top2.contains(&"meeting.jpg") && top2.contains(&"whiteboard.png"), "{top2:?}");
    let meeting = res.hits.iter().find(|h| h.file.name == "meeting.jpg").unwrap();
    assert_eq!(meeting.kind, MatchKind::Meaning, "no text or name match, found by content");
}

#[test]
fn text_and_meaning_combine() {
    let fx = library();
    let e = engine(&fx);
    // "receipt" concept + text "Amount" -> UPI screenshot gets both signals.
    let res = e.search(&Query { text: "receipt amount".into(), ..Default::default() }).unwrap();
    assert_eq!(res.hits[0].file.name, "Screenshot 2024-05-01 UPI.png");
    assert_eq!(res.hits[0].kind, MatchKind::Both);
}

#[test]
fn file_names_are_searchable() {
    let fx = library();
    let e = engine(&fx);
    let res = e.search(&Query { text: "birthday".into(), ..Default::default() }).unwrap();
    assert_eq!(res.hits[0].file.name, "rahul-birthday.png");
    let res = e.search(&Query { text: "overview".into(), ..Default::default() }).unwrap();
    assert_eq!(res.hits[0].file.name, "payments-overview.png");
    assert_eq!(res.hits[0].kind, MatchKind::Name);
}

#[test]
fn filters_apply_to_text_and_meaning() {
    let fx = library();
    let e = engine(&fx);
    // Screenshots only: name/folder heuristic.
    let r = names_f(&e, "error", Filters { screenshots_only: true, ..Default::default() });
    assert_eq!(r, vec!["Screenshot 2023-01-02 error.png"]);
    // Folder.
    let sub = shotfind_core::appdata::dunce_canonicalize(&fx.user.join("sub")).unwrap();
    let r = names_f(&e, "", Filters { folder: Some(sub.to_string_lossy().into()), ..Default::default() });
    assert_eq!(r.len(), 2, "{r:?}");
    let r = names_f(&e, "cat", Filters { folder: Some(sub.to_string_lossy().into()), ..Default::default() });
    assert_eq!(r, vec!["cat.png"]);
    // Date range: everything is "now", so a range in the past is empty.
    let r = names_f(&e, "error", Filters { to_ms: Some(1_000_000), ..Default::default() });
    assert!(r.is_empty());
}

#[test]
fn empty_query_lists_recent_first() {
    let fx = library();
    std::thread::sleep(Duration::from_millis(30));
    write_png(&fx.user.join("newest.png"), &synth_image(600, 300, &["latest"], "none"));
    let e = engine(&fx);
    let res = e.search(&Query::default()).unwrap();
    assert_eq!(res.hits[0].file.name, "newest.png");
    assert!(res.hits.iter().all(|h| h.kind == MatchKind::Recent));
    assert_eq!(res.hits.len(), 9); // 4 standard + 4 library + newest
}

#[test]
fn snippet_is_the_matching_line() {
    let fx = library();
    let e = engine(&fx);
    let res = e.search(&Query { text: "amount".into(), ..Default::default() }).unwrap();
    assert_eq!(res.hits[0].snippet.as_deref(), Some("Amount 1,250.00"));
}

#[test]
fn weird_queries_do_not_break_fts() {
    let fx = library();
    let e = engine(&fx);
    for q in ["\"", "AND OR NOT", "*", "near(", "a\"b", "😀", "-", "UPI\" OR \"x", "    "] {
        e.search(&Query { text: q.into(), ..Default::default() }).unwrap();
    }
}
