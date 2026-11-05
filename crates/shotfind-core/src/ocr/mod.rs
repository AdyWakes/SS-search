//! OCR: PaddleOCR PP-OCRv4 (detection + recognition) on ONNX Runtime.
//!
//! Detection (DBNet) finds text regions on a downscaled copy; recognition
//! (SVTR/CRNN + CTC) reads each region cropped from the full-resolution image.
//! Screenshot text is horizontal, so boxes are axis-aligned and the angle
//! classifier is skipped.

mod db_post;

use crate::imgutil;
use crate::pipeline::OcrEngine;
use crate::runtime;
use anyhow::{anyhow, bail, Context, Result};
use fast_image_resize::FilterType;
use image::RgbImage;
use ort::session::Session;
use ort::value::Tensor;
use std::path::Path;

pub use db_post::{boxes_from_probmap, BoxF};

/// Bump when OCR output changes materially (forces re-OCR of the library).
pub const OCR_VERSION: i64 = 1;

#[derive(Debug, Clone)]
pub struct OcrOptions {
    /// Detection runs with the longest side scaled to at most this (multiple of 32).
    pub det_max_side: u32,
    pub det_thresh: f32,
    pub box_thresh: f32,
    pub unclip_ratio: f32,
    pub rec_height: u32,
    pub rec_max_width: u32,
    pub rec_batch: usize,
    /// Lines with mean character confidence below this are dropped.
    pub min_confidence: f32,
}

impl Default for OcrOptions {
    fn default() -> Self {
        Self {
            det_max_side: 1280,
            det_thresh: 0.3,
            box_thresh: 0.5,
            unclip_ratio: 1.6,
            rec_height: 48,
            rec_max_width: 1920,
            rec_batch: 1,
            min_confidence: 0.5,
        }
    }
}

pub struct PaddleOcr {
    det: Session,
    rec: Session,
    det_input: String,
    rec_input: String,
    /// Class index -> string; index 0 is the CTC blank.
    classes: Vec<String>,
    opts: OcrOptions,
}

#[derive(Debug, Clone)]
pub struct TextLine {
    pub text: String,
    pub confidence: f32,
    pub bbox: BoxF,
}

impl PaddleOcr {
    pub fn load(det_model: &Path, rec_model: &Path, dict: Option<&Path>, threads: usize) -> Result<PaddleOcr> {
        let det = runtime::session(det_model, threads)?;
        let rec = runtime::session(rec_model, threads)?;
        let det_input = det.inputs().first().ok_or_else(|| anyhow!("det model has no inputs"))?.name().to_string();
        let rec_input = rec.inputs().first().ok_or_else(|| anyhow!("rec model has no inputs"))?.name().to_string();
        let chars: Vec<String> = match rec.metadata().ok().and_then(|m| m.custom("character")) {
            Some(s) => s.lines().map(|l| l.to_string()).collect(),
            None => {
                let d = dict.context("recognition model has no embedded dictionary and no dict file was given")?;
                std::fs::read_to_string(d)?.lines().map(|l| l.to_string()).collect()
            }
        };
        if chars.is_empty() {
            bail!("empty OCR dictionary");
        }
        // PaddleOCR layout: [blank] + dictionary + [space].
        let mut classes = Vec::with_capacity(chars.len() + 2);
        classes.push(String::new());
        classes.extend(chars);
        classes.push(" ".into());
        Ok(PaddleOcr { det, rec, det_input, rec_input, classes, opts: OcrOptions::default() })
    }

    pub fn with_options(mut self, o: OcrOptions) -> Self {
        self.opts = o;
        self
    }

    /// Full pipeline: detect boxes, recognise each, return lines in reading order.
    pub fn read_lines(&mut self, img: &RgbImage) -> Result<Vec<TextLine>> {
        let boxes = self.detect(img)?;
        if boxes.is_empty() {
            return Ok(Vec::new());
        }
        let crops: Vec<RgbImage> = boxes.iter().map(|b| crop(img, b)).collect();
        let recs = self.recognize_batch(&crops)?;
        let mut lines: Vec<TextLine> = boxes
            .into_iter()
            .zip(recs)
            .filter(|(_, (t, c))| !t.trim().is_empty() && *c >= self.opts.min_confidence)
            .map(|(bbox, (text, confidence))| TextLine { text: fix_digit_confusions(text.trim()), confidence, bbox })
            .collect();
        sort_reading_order(&mut lines);
        Ok(lines)
    }

    /// Text regions in original-image pixel coordinates. Very tall or wide
    /// images (scrolling captures) are split into overlapping tiles so text is
    /// not shrunk below a readable size.
    pub fn detect(&mut self, img: &RgbImage) -> Result<Vec<BoxF>> {
        let (w, h) = img.dimensions();
        let (long, short) = (w.max(h), w.min(h));
        if long as f32 / short.max(1) as f32 <= 2.2 {
            return self.detect_tile(img);
        }
        let vertical = h > w;
        // Tiles are as long as detection can take at the scale the short side
        // already forces, so text is never shrunk more than in a normal image.
        let tile = short.max(self.opts.det_max_side);
        let overlap = tile / 4;
        let step = tile - overlap;
        let mut all: Vec<BoxF> = Vec::new();
        let mut start = 0u32;
        loop {
            let end = (start + tile).min(long);
            let sub = if vertical {
                image::imageops::crop_imm(img, 0, start, w, end - start).to_image()
            } else {
                image::imageops::crop_imm(img, start, 0, end - start, h).to_image()
            };
            for mut b in self.detect_tile(&sub)? {
                let (lo, hi) = if vertical { (b.y0, b.y1) } else { (b.x0, b.x1) };
                // Boxes touching an inner tile edge are probably cut; the
                // neighbouring tile sees them whole (overlap > line height).
                let cut = (start > 0 && lo <= 2.0) || (end < long && hi >= (end - start) as f32 - 2.0);
                if cut {
                    continue;
                }
                if vertical {
                    b.y0 += start as f32;
                    b.y1 += start as f32;
                } else {
                    b.x0 += start as f32;
                    b.x1 += start as f32;
                }
                all.push(b);
            }
            if end >= long {
                break;
            }
            start += step;
        }
        // Lines inside an overlap are found twice: keep the larger box.
        all.sort_by(|a, b| b.area().total_cmp(&a.area()));
        let mut out: Vec<BoxF> = Vec::with_capacity(all.len());
        for b in all {
            if !out.iter().any(|k| k.overlap_ratio(&b) > 0.5) {
                out.push(b);
            }
        }
        Ok(out)
    }

    fn detect_tile(&mut self, img: &RgbImage) -> Result<Vec<BoxF>> {
        let (w, h) = img.dimensions();
        let scale = (self.opts.det_max_side as f32 / w.max(h) as f32).min(1.0);
        let round32 = |v: f32| ((v / 32.0).round() as u32).max(1) * 32;
        let (dw, dh) = (round32(w as f32 * scale), round32(h as f32 * scale));
        let small = imgutil::resize(img, dw, dh, FilterType::Bilinear);
        // Normalise: ImageNet mean/std, NCHW.
        const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
        const STD: [f32; 3] = [0.229, 0.224, 0.225];
        let plane = (dw * dh) as usize;
        let mut data = vec![0f32; plane * 3];
        for (i, p) in small.pixels().enumerate() {
            for c in 0..3 {
                data[c * plane + i] = (p[c] as f32 / 255.0 - MEAN[c]) / STD[c];
            }
        }
        let input = Tensor::from_array(([1usize, 3, dh as usize, dw as usize], data))?;
        let outputs = self.det.run(ort::inputs![self.det_input.as_str() => input])?;
        let (shape, prob) = outputs[0].try_extract_tensor::<f32>()?;
        let (ph, pw) = (shape[2] as usize, shape[3] as usize);
        let boxes =
            boxes_from_probmap(prob, pw, ph, self.opts.det_thresh, self.opts.box_thresh, self.opts.unclip_ratio);
        let (sx, sy) = (w as f32 / pw as f32, h as f32 / ph as f32);
        Ok(boxes
            .into_iter()
            .map(|b| BoxF {
                x0: (b.x0 * sx).clamp(0.0, w as f32),
                y0: (b.y0 * sy).clamp(0.0, h as f32),
                x1: (b.x1 * sx).clamp(0.0, w as f32),
                y1: (b.y1 * sy).clamp(0.0, h as f32),
                score: b.score,
            })
            .filter(|b| b.x1 - b.x0 >= 4.0 && b.y1 - b.y0 >= 4.0)
            .collect())
    }

    /// Recognises crops in width-sorted batches. Returns (text, confidence) per crop.
    pub fn recognize_batch(&mut self, crops: &[RgbImage]) -> Result<Vec<(String, f32)>> {
        let hgt = self.opts.rec_height;
        let target_w = |c: &RgbImage| -> u32 {
            let ratio = c.width() as f32 / c.height().max(1) as f32;
            ((hgt as f32 * ratio).ceil() as u32).clamp(16, self.opts.rec_max_width)
        };
        let mut order: Vec<usize> = (0..crops.len()).collect();
        order.sort_by_key(|&i| target_w(&crops[i]));
        let mut out = vec![(String::new(), 0f32); crops.len()];
        for chunk in order.chunks(self.opts.rec_batch.max(1)) {
            let max_w = chunk.iter().map(|&i| target_w(&crops[i])).max().unwrap_or(16);
            // Width padded to a multiple of 8 (model stride); padding value 0 = mid-grey.
            let bw = max_w.div_ceil(8) * 8;
            let plane = (hgt * bw) as usize;
            let mut data = vec![0f32; chunk.len() * 3 * plane];
            for (bi, &i) in chunk.iter().enumerate() {
                let tw = target_w(&crops[i]);
                let r = imgutil::resize(&crops[i], tw, hgt, FilterType::Bilinear);
                let base = bi * 3 * plane;
                for (x, y, p) in r.enumerate_pixels() {
                    let off = (y * bw + x) as usize;
                    for c in 0..3 {
                        data[base + c * plane + off] = (p[c] as f32 / 255.0 - 0.5) / 0.5;
                    }
                }
            }
            let input = Tensor::from_array(([chunk.len(), 3, hgt as usize, bw as usize], data))?;
            let outputs = self.rec.run(ort::inputs![self.rec_input.as_str() => input])?;
            let (shape, probs) = outputs[0].try_extract_tensor::<f32>()?;
            let (t, k) = (shape[1] as usize, shape[2] as usize);
            if k != self.classes.len() {
                bail!("recognition model has {k} classes but dictionary has {}", self.classes.len());
            }
            for (bi, &i) in chunk.iter().enumerate() {
                out[i] = ctc_decode(&probs[bi * t * k..(bi + 1) * t * k], t, k, &self.classes);
            }
        }
        Ok(out)
    }
}

impl OcrEngine for PaddleOcr {
    fn recognize(&mut self, img: &RgbImage) -> Result<String> {
        let lines = self.read_lines(img)?;
        Ok(join_lines(&lines))
    }
}

/// OCR often reads digits inside numbers as look-alike letters ("4O4", "2O24",
/// "l2:3O"). In tokens that already contain a digit and otherwise only digits
/// and look-alikes, map O/o->0 and l/I/|->1.
pub fn fix_digit_confusions(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        let has_digit = token.chars().any(|c| c.is_ascii_digit());
        let lookalike = |c: char| matches!(c, 'O' | 'o' | 'l' | 'I' | '|');
        let fixable = has_digit
            && token.chars().any(lookalike)
            && token.chars().all(|c| c.is_ascii_digit() || lookalike(c) || matches!(c, ':' | '.' | ',' | '-' | '/'));
        if fixable {
            out.extend(token.chars().map(|c| match c {
                'O' | 'o' => '0',
                'l' | 'I' | '|' => '1',
                c => c,
            }));
        } else {
            out.push_str(token);
        }
        token.clear();
    };
    for c in text.chars() {
        if c.is_whitespace() {
            flush(&mut token, &mut out);
            out.push(c);
        } else {
            token.push(c);
        }
    }
    flush(&mut token, &mut out);
    out
}

/// Greedy CTC: argmax per step, collapse repeats, drop blanks.
pub fn ctc_decode(probs: &[f32], t: usize, k: usize, classes: &[String]) -> (String, f32) {
    let mut text = String::new();
    let mut conf_sum = 0f32;
    let mut n = 0usize;
    let mut prev = usize::MAX;
    for step in 0..t {
        let row = &probs[step * k..(step + 1) * k];
        let (mut best, mut best_p) = (0usize, f32::MIN);
        for (j, &p) in row.iter().enumerate() {
            if p > best_p {
                best = j;
                best_p = p;
            }
        }
        if best != 0 && best != prev {
            if let Some(s) = classes.get(best) {
                text.push_str(s);
                conf_sum += best_p;
                n += 1;
            }
        }
        prev = best;
    }
    let conf = if n > 0 { conf_sum / n as f32 } else { 0.0 };
    (text, conf)
}

fn crop(img: &RgbImage, b: &BoxF) -> RgbImage {
    let x0 = b.x0.floor().max(0.0) as u32;
    let y0 = b.y0.floor().max(0.0) as u32;
    let x1 = (b.x1.ceil() as u32).min(img.width()).max(x0 + 1);
    let y1 = (b.y1.ceil() as u32).min(img.height()).max(y0 + 1);
    image::imageops::crop_imm(img, x0, y0, x1 - x0, y1 - y0).to_image()
}

/// Groups boxes into rows (vertical overlap) and sorts rows top-down, boxes left-right.
pub fn sort_reading_order(lines: &mut Vec<TextLine>) {
    lines.sort_by(|a, b| a.bbox.y0.total_cmp(&b.bbox.y0));
    let mut rows: Vec<Vec<TextLine>> = Vec::new();
    for l in lines.drain(..) {
        let cy = (l.bbox.y0 + l.bbox.y1) / 2.0;
        match rows.last_mut() {
            Some(row)
                if {
                    let r = &row[0].bbox;
                    cy > r.y0 && cy < r.y1
                } =>
            {
                row.push(l)
            }
            _ => rows.push(vec![l]),
        }
    }
    for mut row in rows {
        row.sort_by(|a, b| a.bbox.x0.total_cmp(&b.bbox.x0));
        lines.extend(row);
    }
}

/// Same-row boxes are joined with spaces; rows with newlines.
pub fn join_lines(lines: &[TextLine]) -> String {
    let mut out = String::new();
    let mut prev: Option<&BoxF> = None;
    for l in lines {
        if let Some(p) = prev {
            let cy = (l.bbox.y0 + l.bbox.y1) / 2.0;
            out.push(if cy > p.y0 && cy < p.y1 { ' ' } else { '\n' });
        }
        out.push_str(&l.text);
        prev = Some(&l.bbox);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctc_collapses_repeats_and_blanks() {
        let classes: Vec<String> = ["", "a", "b", " "].iter().map(|s| s.to_string()).collect();
        // steps: a a blank a b b  -> "aab"
        let seq = [1, 1, 0, 1, 2, 2];
        let mut probs = vec![0f32; seq.len() * 4];
        for (t, &c) in seq.iter().enumerate() {
            probs[t * 4 + c] = 0.9;
        }
        let (s, conf) = ctc_decode(&probs, seq.len(), 4, &classes);
        assert_eq!(s, "aab");
        assert!((conf - 0.9).abs() < 1e-6);
    }

    #[test]
    fn digit_confusions() {
        assert_eq!(fix_digit_confusions("Error 4O4 at l2:3O"), "Error 404 at 12:30");
        assert_eq!(fix_digit_confusions("Hello World 2O24-O1-l5"), "Hello World 2024-01-15");
        assert_eq!(fix_digit_confusions("OK lol IO"), "OK lol IO");
    }

    #[test]
    fn reading_order() {
        let mk = |t: &str, x0: f32, y0: f32| TextLine {
            text: t.into(),
            confidence: 1.0,
            bbox: BoxF { x0, y0, x1: x0 + 50.0, y1: y0 + 20.0, score: 1.0 },
        };
        let mut v = vec![mk("world", 100.0, 12.0), mk("second", 0.0, 50.0), mk("hello", 0.0, 10.0)];
        sort_reading_order(&mut v);
        assert_eq!(join_lines(&v), "hello world\nsecond");
    }
}
