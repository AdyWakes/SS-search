//! Search: typo-tolerant full-text over OCR text + file names, CLIP vector
//! search, and a combined ranking. Runs on its own read-only SQLite connection.

use crate::db::{row_to_file, Db, FileRow};
use crate::pipeline::TextEmbedder;
use crate::textnorm::{edit_distance, fold, max_edits, ocr_fold, tokens};
use crate::vectors::VectorIndex;
use anyhow::Result;
use parking_lot::{Mutex, RwLock};
use rusqlite::{params_from_iter, types::Value};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Filters {
    /// Only files under this folder.
    pub folder: Option<String>,
    /// Modified at or after (unix ms).
    pub from_ms: Option<i64>,
    /// Modified before (unix ms).
    pub to_ms: Option<i64>,
    pub screenshots_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Query {
    pub text: String,
    pub filters: Filters,
    pub limit: usize,
}

impl Default for Query {
    fn default() -> Self {
        Query { text: String::new(), filters: Filters::default(), limit: 60 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchKind {
    /// Empty query: most recent first.
    Recent,
    Text,
    Name,
    Meaning,
    Both,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    #[serde(flatten)]
    pub file: FileRow,
    pub score: f32,
    pub text_score: f32,
    pub semantic_score: f32,
    pub kind: MatchKind,
    /// The OCR line that matched best (for display).
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResults {
    pub hits: Vec<Hit>,
    pub took_ms: f64,
    pub semantic_used: bool,
    /// Query terms that were matched approximately, e.g. ("paymnet", "payment").
    pub corrections: Vec<(String, String)>,
}

/// Ranking constants. Measured with the real int8 CLIP ViT-B/32 (CI log of
/// `clip_ranks_matching_concepts`): matching text-image pairs scored
/// 0.233-0.257 and non-matching ones 0.18-0.237, i.e. the useful range is
/// narrow. Photos of real objects usually separate more than these synthetic
/// images; re-calibrate on a real library.
pub mod tuning {
    pub const SEM_FLOOR: f32 = 0.18;
    pub const SEM_SPAN: f32 = 0.10;
    /// Semantic hits must be within this much of the best one...
    pub const SEM_WINDOW: f32 = 0.04;
    /// ...and above this absolute similarity.
    pub const SEM_MIN: f32 = 0.215;
    pub const SEM_WEIGHT: f32 = 0.8;
    pub const PHRASE_BONUS: f32 = 0.15;
    pub const NAME_WEIGHT: f32 = 0.8;
    /// Multi-word text hits matching less than this fraction of the words are
    /// dropped once there are `ENOUGH_GOOD_HITS` better ones.
    pub const MIN_COVERAGE: f32 = 0.5;
    pub const ENOUGH_GOOD_HITS: usize = 10;
}

pub struct Searcher {
    db: Mutex<Db>,
    vectors: Arc<RwLock<VectorIndex>>,
    text_embedder: Mutex<Option<Box<dyn TextEmbedder>>>,
    query_cache: Mutex<lru::LruCache<String, Vec<f32>>>,
    vocab: RwLock<Vocab>,
}

#[derive(Default)]
struct Vocab {
    terms: Vec<(String, String, i64)>, // (term, ocr_fold(term), doc count)
    built: Option<Instant>,
    stamp: (i64, i64),
}

impl Searcher {
    pub fn open(db_path: &std::path::Path, vectors: Arc<RwLock<VectorIndex>>) -> Result<Searcher> {
        let db = Db::open(db_path)?;
        Ok(Searcher {
            db: Mutex::new(db),
            vectors,
            text_embedder: Mutex::new(None),
            query_cache: Mutex::new(lru::LruCache::new(NonZeroUsize::new(256).unwrap())),
            vocab: RwLock::new(Vocab::default()),
        })
    }

    pub fn set_text_embedder(&self, e: Option<Box<dyn TextEmbedder>>) {
        *self.text_embedder.lock() = e;
        self.query_cache.lock().clear();
    }

    pub fn has_semantic(&self) -> bool {
        self.text_embedder.lock().is_some() && !self.vectors.read().is_empty()
    }

    pub fn file(&self, id: i64) -> Result<Option<FileRow>> {
        self.db.lock().file(id)
    }

    pub fn ocr_text(&self, id: i64) -> Result<Option<String>> {
        self.db.lock().ocr_text(id)
    }

    /// Distinct parent folders with indexed files (for the folder filter).
    pub fn folders(&self) -> Result<Vec<(String, u64)>> {
        let db = self.db.lock();
        let mut st = db
            .conn()
            .prepare("SELECT dir, COUNT(*) FROM files WHERE state=1 GROUP BY dir ORDER BY COUNT(*) DESC LIMIT 200")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn search(&self, q: &Query) -> Result<SearchResults> {
        let start = Instant::now();
        let limit = q.limit.clamp(1, 500);
        let qtokens = tokens(&q.text);
        if qtokens.is_empty() {
            let hits = self.recent(&q.filters, limit)?;
            return Ok(SearchResults { hits, took_ms: ms(start), semantic_used: false, corrections: Vec::new() });
        }
        let (text_hits, corrections) = self.text_search(&q.text, &qtokens, &q.filters)?;
        let sem_hits = self.semantic_search(&q.text, &q.filters)?;
        let semantic_used = sem_hits.is_some();

        // Merge.
        let mut by_id: HashMap<i64, (f32, f32, bool, Option<String>)> = HashMap::new();
        for t in text_hits {
            by_id.insert(t.id, (t.score, 0.0, t.name_only, t.snippet));
        }
        for (id, s) in sem_hits.unwrap_or_default() {
            by_id.entry(id).or_insert((0.0, 0.0, false, None)).1 = s;
        }
        let mut scored: Vec<(i64, f32, f32, f32, MatchKind, Option<String>)> = by_id
            .into_iter()
            .map(|(id, (ts, ss, name_only, snip))| {
                let kind = match (ts > 0.0, ss > 0.0) {
                    (true, true) => MatchKind::Both,
                    (true, false) if name_only => MatchKind::Name,
                    (true, false) => MatchKind::Text,
                    _ => MatchKind::Meaning,
                };
                (id, ts + ss * tuning::SEM_WEIGHT, ts, ss, kind, snip)
            })
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.0.cmp(&a.0)));
        scored.truncate(limit);

        let ids: Vec<i64> = scored.iter().map(|s| s.0).collect();
        let files = self.files_by_id(&ids)?;
        let hits = scored
            .into_iter()
            .filter_map(|(id, score, ts, ss, kind, snippet)| {
                files.get(&id).cloned().map(|file| Hit {
                    file,
                    score,
                    text_score: ts,
                    semantic_score: ss,
                    kind,
                    snippet,
                })
            })
            .collect();
        Ok(SearchResults { hits, took_ms: ms(start), semantic_used, corrections })
    }

    fn recent(&self, f: &Filters, limit: usize) -> Result<Vec<Hit>> {
        let db = self.db.lock();
        let (wh, mut params) = filter_sql(f);
        params.push(Value::Integer(limit as i64));
        let sql = format!(
            "SELECT f.id, f.path, f.dir, f.name, f.size, f.mtime, f.width, f.height, f.is_screenshot, f.state
             FROM files f WHERE f.state=1 {wh} ORDER BY f.mtime DESC LIMIT ?"
        );
        let mut st = db.conn().prepare_cached(&sql)?;
        let rows = st.query_map(params_from_iter(params), row_to_file)?;
        Ok(rows
            .map(|r| {
                r.map(|file| Hit {
                    file,
                    score: 0.0,
                    text_score: 0.0,
                    semantic_score: 0.0,
                    kind: MatchKind::Recent,
                    snippet: None,
                })
            })
            .collect::<Result<_, _>>()?)
    }

    fn files_by_id(&self, ids: &[i64]) -> Result<HashMap<i64, FileRow>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let db = self.db.lock();
        let placeholders = vec!["?"; ids.len()].join(",");
        let sql = format!(
            "SELECT id, path, dir, name, size, mtime, width, height, is_screenshot, state FROM files WHERE id IN ({placeholders})"
        );
        let mut st = db.conn().prepare(&sql)?;
        let rows = st.query_map(params_from_iter(ids.iter()), row_to_file)?;
        let mut out = HashMap::new();
        for r in rows {
            let f = r?;
            out.insert(f.id, f);
        }
        Ok(out)
    }

    fn refresh_vocab(&self) -> Result<()> {
        let stamp: (i64, i64) = self.db.lock().conn().query_row(
            "SELECT COUNT(*), COALESCE(MAX(indexed_at), 0) FROM files WHERE state=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        {
            let v = self.vocab.read();
            let fresh = v.built.is_some_and(|t| t.elapsed() < Duration::from_secs(5));
            if v.built.is_some() && (v.stamp == stamp || fresh) {
                return Ok(());
            }
        }
        let db = self.db.lock();
        let mut st = db.conn().prepare("SELECT term, doc FROM ocr_vocab")?;
        let terms: Vec<(String, String, i64)> = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .filter_map(|r| r.ok())
            .map(|(t, d)| {
                let f = ocr_fold(&t);
                (t, f, d)
            })
            .collect();
        *self.vocab.write() = Vocab { terms, built: Some(Instant::now()), stamp };
        Ok(())
    }

    /// For each query token: the index terms it may match, with a quality in (0,1].
    fn expand(&self, qtokens: &[String]) -> Result<Vec<Vec<(String, f32)>>> {
        self.refresh_vocab()?;
        let vocab = self.vocab.read();
        let last = qtokens.len() - 1;
        Ok(qtokens
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let k = max_edits(t);
                let numeric = t.chars().all(|c| c.is_ascii_digit());
                let tf = ocr_fold(t);
                let mut cands: Vec<(String, f32, i64)> = Vec::new();
                for (term, folded, doc) in &vocab.terms {
                    let q = if term == t {
                        1.0
                    } else if (i == last && t.chars().count() >= 2 && term.starts_with(t.as_str()))
                        || (!numeric && *folded == tf)
                    {
                        0.9 // still typing, or an OCR look-alike (UPl ~ UPI)
                    } else if k > 0 {
                        match edit_distance(term, t, k) {
                            Some(1) => 0.75,
                            Some(_) => 0.55,
                            None => continue,
                        }
                    } else {
                        continue;
                    };
                    cands.push((term.clone(), q, *doc));
                }
                // Best quality first, then most common; cap the OR-group size.
                cands.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.2.cmp(&a.2)));
                cands.truncate(24);
                cands.into_iter().map(|(t, q, _)| (t, q)).collect()
            })
            .collect())
    }

    fn text_search(&self, raw: &str, qtokens: &[String], f: &Filters) -> Result<(Vec<TextHit>, Corrections)> {
        let groups = self.expand(qtokens)?;
        let mut corrections = Vec::new();
        for (t, g) in qtokens.iter().zip(&groups) {
            if !g.iter().any(|(c, _)| c == t) {
                if let Some((best, q)) = g.first() {
                    if *q < 0.9 {
                        corrections.push((t.clone(), best.clone()));
                    }
                }
            }
        }
        let usable: Vec<&Vec<(String, f32)>> = groups.iter().filter(|g| !g.is_empty()).collect();
        if usable.is_empty() {
            return Ok((Vec::new(), corrections));
        }
        let or_group = |g: &Vec<(String, f32)>| -> String {
            let parts: Vec<String> = g.iter().map(|(t, _)| format!("\"{}\"", t.replace('"', "\"\""))).collect();
            format!("({})", parts.join(" OR "))
        };
        // Files with every word first; when those are few, also files with
        // some of the words (ranked below by coverage).
        let and_q = usable.iter().map(|g| or_group(g)).collect::<Vec<_>>().join(" AND ");
        let mut rows = self.fts_rows(&and_q, f, 400)?;
        if usable.len() > 1 && rows.len() < 40 {
            let or_q = usable.iter().map(|g| or_group(g)).collect::<Vec<_>>().join(" OR ");
            let seen: HashSet<i64> = rows.iter().map(|r| r.0).collect();
            rows.extend(self.fts_rows(&or_q, f, 400)?.into_iter().filter(|r| !seen.contains(&r.0)));
        }

        let qmap: Vec<HashMap<&str, f32>> =
            groups.iter().map(|g| g.iter().map(|(t, q)| (t.as_str(), *q)).collect()).collect();
        let phrase = tokens(raw).join(" ");
        let n = qtokens.len() as f32;
        let mut hits = Vec::new();
        for (id, text, name) in rows {
            let text_toks = tokens(&text);
            let name_toks = tokens(&name);
            let (mut sum, mut name_only) = (0f32, true);
            for m in &qmap {
                let in_text = text_toks.iter().filter_map(|w| m.get(w.as_str())).fold(0f32, |a, b| a.max(*b));
                let in_name = name_toks.iter().filter_map(|w| m.get(w.as_str())).fold(0f32, |a, b| a.max(*b))
                    * tuning::NAME_WEIGHT;
                if in_text >= in_name && in_text > 0.0 {
                    name_only = false;
                }
                sum += in_text.max(in_name);
            }
            let coverage = sum / n;
            let joined = text_toks.join(" ");
            let mut score = coverage;
            if n >= 2.0 && joined.contains(&phrase) {
                score += tuning::PHRASE_BONUS;
            }
            let snippet = best_line(&text, &qmap);
            hits.push(TextHit { id, score, name_only, snippet, coverage });
        }
        // Weak partial matches only pad out a short result list.
        let good = hits.iter().filter(|h| h.coverage >= tuning::MIN_COVERAGE).count();
        if good >= tuning::ENOUGH_GOOD_HITS {
            hits.retain(|h| h.coverage >= tuning::MIN_COVERAGE);
        }
        Ok((hits, corrections))
    }

    fn fts_rows(&self, match_q: &str, f: &Filters, limit: usize) -> Result<Vec<(i64, String, String)>> {
        let db = self.db.lock();
        let (wh, mut params) = filter_sql(f);
        let mut all = vec![Value::Text(match_q.to_string())];
        all.append(&mut params);
        all.push(Value::Integer(limit as i64));
        let sql = format!(
            "SELECT f.id, o.text, f.name FROM ocr_fts
             JOIN files f ON f.id = ocr_fts.rowid
             JOIN ocr o ON o.file_id = ocr_fts.rowid
             WHERE ocr_fts MATCH ? AND f.state=1 {wh}
             ORDER BY bm25(ocr_fts, 1.0, 0.6) LIMIT ?"
        );
        let mut st = db.conn().prepare_cached(&sql)?;
        let rows = st.query_map(params_from_iter(all), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn semantic_search(&self, text: &str, f: &Filters) -> Result<Option<Vec<(i64, f32)>>> {
        let key = fold(text.trim());
        let cached = self.query_cache.lock().get(&key).cloned();
        let qv = match cached {
            Some(v) => Some(v),
            None => {
                let mut emb = self.text_embedder.lock();
                match emb.as_mut() {
                    Some(e) => {
                        let v = e.embed_text(text.trim())?;
                        self.query_cache.lock().put(key, v.clone());
                        Some(v)
                    }
                    None => None,
                }
            }
        };
        let Some(qv) = qv else { return Ok(None) };
        let vectors = self.vectors.read();
        if vectors.is_empty() {
            return Ok(None);
        }
        let allow = if f.folder.is_some() || f.from_ms.is_some() || f.to_ms.is_some() || f.screenshots_only {
            Some(self.allowed_ids(f)?)
        } else {
            None
        };
        let top = vectors.search(&qv, 200, allow.as_ref());
        let best = top.first().map(|t| t.1).unwrap_or(0.0);
        let cut = (best - tuning::SEM_WINDOW).max(tuning::SEM_MIN);
        Ok(Some(
            top.into_iter()
                .filter(|(_, c)| *c >= cut)
                .map(|(id, c)| (id, ((c - tuning::SEM_FLOOR) / tuning::SEM_SPAN).clamp(0.0, 1.0)))
                .filter(|(_, s)| *s > 0.0)
                .collect(),
        ))
    }

    fn allowed_ids(&self, f: &Filters) -> Result<HashSet<i64>> {
        let db = self.db.lock();
        let (wh, params) = filter_sql(f);
        let sql = format!("SELECT f.id FROM files f WHERE f.state=1 {wh}");
        let mut st = db.conn().prepare_cached(&sql)?;
        let rows = st.query_map(params_from_iter(params), |r| r.get::<_, i64>(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

type Corrections = Vec<(String, String)>;

struct TextHit {
    id: i64,
    score: f32,
    coverage: f32,
    name_only: bool,
    snippet: Option<String>,
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn filter_sql(f: &Filters) -> (String, Vec<Value>) {
    let mut wh = String::new();
    let mut p = Vec::new();
    if let Some(folder) = f.folder.as_deref().filter(|s| !s.is_empty()) {
        let sep = std::path::MAIN_SEPARATOR;
        let prefix = format!("{}{}", folder.trim_end_matches(['/', '\\']), sep);
        wh.push_str(" AND substr(f.path, 1, length(?)) = ?");
        p.push(Value::Text(prefix.clone()));
        p.push(Value::Text(prefix));
    }
    if let Some(from) = f.from_ms {
        wh.push_str(" AND f.mtime >= ?");
        p.push(Value::Integer(from));
    }
    if let Some(to) = f.to_ms {
        wh.push_str(" AND f.mtime < ?");
        p.push(Value::Integer(to));
    }
    if f.screenshots_only {
        wh.push_str(" AND f.is_screenshot = 1");
    }
    (wh, p)
}

/// The OCR line containing the most (best) query matches.
fn best_line(text: &str, qmap: &[HashMap<&str, f32>]) -> Option<String> {
    let mut best: Option<(&str, f32)> = None;
    for line in text.lines() {
        let toks = tokens(line);
        let s: f32 =
            qmap.iter().map(|m| toks.iter().filter_map(|w| m.get(w.as_str())).fold(0f32, |a, b| a.max(*b))).sum();
        if s > 0.0 && best.is_none_or(|(_, b)| s > b) {
            best = Some((line, s));
        }
    }
    best.map(|(l, _)| {
        let l = l.trim();
        if l.chars().count() > 140 {
            format!("{}…", l.chars().take(140).collect::<String>())
        } else {
            l.to_string()
        }
    })
}
