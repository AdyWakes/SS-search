//! In-memory brute-force vector index (cosine similarity on unit vectors).
//! Vectors are kept as int8 with a per-vector scale: 100k images x 512 dims is
//! ~51 MB (vs ~205 MB as f32) and a query is ~20 ms on one core, so no ANN
//! structure is needed. Full-precision vectors stay in SQLite.

use crate::db::{bytes_to_f32s, Db};
use anyhow::Result;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Default)]
pub struct VectorIndex {
    dim: usize,
    model: String,
    ids: Vec<i64>,
    data: Vec<i8>,
    scales: Vec<f32>,
    pos: HashMap<i64, usize>,
}

fn quantize(v: &[f32]) -> (Vec<i8>, f32) {
    let max = v.iter().fold(0f32, |m, x| m.max(x.abs()));
    if max == 0.0 {
        return (vec![0; v.len()], 0.0);
    }
    let s = 127.0 / max;
    (v.iter().map(|x| (x * s).round().clamp(-127.0, 127.0) as i8).collect(), max / 127.0)
}

pub fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
}

impl VectorIndex {
    pub fn load(db: &Db, model: &str) -> Result<VectorIndex> {
        let mut idx = VectorIndex { model: model.to_string(), ..Default::default() };
        let mut st = db.conn().prepare("SELECT file_id, vec FROM embeddings WHERE model=?1")?;
        let rows = st.query_map([model], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))?;
        for row in rows {
            let (id, blob) = row?;
            idx.upsert(id, bytes_to_f32s(&blob));
        }
        Ok(idx)
    }

    pub fn model(&self) -> &str {
        &self.model
    }
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn upsert(&mut self, id: i64, mut v: Vec<f32>) {
        if v.is_empty() {
            return;
        }
        if self.dim == 0 {
            self.dim = v.len();
        }
        if v.len() != self.dim {
            tracing::warn!("embedding dim mismatch ({} vs {}), ignoring", v.len(), self.dim);
            return;
        }
        normalize(&mut v);
        let (q, scale) = quantize(&v);
        if let Some(&p) = self.pos.get(&id) {
            self.data[p * self.dim..(p + 1) * self.dim].copy_from_slice(&q);
            self.scales[p] = scale;
        } else {
            self.pos.insert(id, self.ids.len());
            self.ids.push(id);
            self.data.extend_from_slice(&q);
            self.scales.push(scale);
        }
    }

    pub fn remove(&mut self, id: i64) {
        let Some(p) = self.pos.remove(&id) else { return };
        let last = self.ids.len() - 1;
        if p != last {
            let moved = self.ids[last];
            self.ids[p] = moved;
            let (d, dim) = (&mut self.data, self.dim);
            d.copy_within(last * dim..(last + 1) * dim, p * dim);
            self.scales[p] = self.scales[last];
            self.pos.insert(moved, p);
        }
        self.ids.pop();
        self.scales.pop();
        self.data.truncate(last * self.dim);
    }

    /// Top-`k` (id, cosine) for a unit query vector, optionally restricted to `allow`.
    pub fn search(&self, q: &[f32], k: usize, allow: Option<&HashSet<i64>>) -> Vec<(i64, f32)> {
        if self.dim == 0 || q.len() != self.dim || k == 0 {
            return Vec::new();
        }
        let mut scored: Vec<(i64, f32)> = self
            .ids
            .iter()
            .enumerate()
            .filter(|(_, id)| allow.is_none_or(|a| a.contains(id)))
            .map(|(i, &id)| {
                let row = &self.data[i * self.dim..(i + 1) * self.dim];
                (id, dot(row, q) * self.scales[i])
            })
            .collect();
        let k = k.min(scored.len());
        if k == 0 {
            return scored;
        }
        if k < scored.len() {
            scored.select_nth_unstable_by(k - 1, |a, b| b.1.total_cmp(&a.1));
            scored.truncate(k);
        }
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored
    }
}

#[inline]
fn dot(a: &[i8], b: &[f32]) -> f32 {
    // Chunked so the compiler auto-vectorises.
    let mut acc = [0f32; 8];
    let ca = a.chunks_exact(8);
    let cb = b.chunks_exact(8);
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for i in 0..8 {
            acc[i] += x[i] as f32 * y[i];
        }
    }
    let mut s: f32 = acc.iter().sum();
    for (x, y) in ra.iter().zip(rb) {
        s += *x as f32 * y;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantized_scores_are_close_to_exact() {
        let mut idx = VectorIndex::default();
        let mut rng = 1u64;
        let mut rand = || {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            ((rng >> 33) % 2000) as f32 / 1000.0 - 1.0
        };
        let vs: Vec<Vec<f32>> = (0..50).map(|_| (0..512).map(|_| rand()).collect()).collect();
        for (i, v) in vs.iter().enumerate() {
            idx.upsert(i as i64, v.clone());
        }
        let mut q: Vec<f32> = (0..512).map(|_| rand()).collect();
        normalize(&mut q);
        for (id, score) in idx.search(&q, 50, None) {
            let mut v = vs[id as usize].clone();
            normalize(&mut v);
            let exact: f32 = v.iter().zip(&q).map(|(a, b)| a * b).sum();
            assert!((exact - score).abs() < 0.01, "{exact} vs {score}");
        }
    }

    #[test]
    fn upsert_remove_search() {
        let mut idx = VectorIndex::default();
        idx.upsert(1, vec![1.0, 0.0, 0.0]);
        idx.upsert(2, vec![0.0, 1.0, 0.0]);
        idx.upsert(3, vec![0.7, 0.7, 0.0]);
        let r = idx.search(&[1.0, 0.0, 0.0], 2, None);
        assert_eq!(r.iter().map(|x| x.0).collect::<Vec<_>>(), vec![1, 3]);
        idx.remove(1);
        assert_eq!(idx.len(), 2);
        let r = idx.search(&[1.0, 0.0, 0.0], 5, None);
        assert_eq!(r[0].0, 3);
        idx.upsert(3, vec![0.0, 0.0, 1.0]); // update in place
        let r = idx.search(&[0.0, 0.0, 1.0], 1, None);
        assert_eq!(r[0].0, 3);
        let allow: HashSet<i64> = [2].into();
        assert_eq!(idx.search(&[0.0, 0.0, 1.0], 5, Some(&allow)).len(), 1);
    }
}
