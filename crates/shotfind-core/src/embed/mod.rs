//! Image/text embeddings with CLIP ViT-B/32 (8-bit quantized ONNX).

pub mod tokenizer;

use crate::imgutil;
use crate::pipeline::{ImageEmbedder, TextEmbedder};
use crate::runtime;
use crate::vectors::normalize;
use anyhow::{anyhow, bail, Result};
use fast_image_resize::FilterType;
use image::{Rgb, RgbImage};
use ort::session::Session;
use ort::value::{Tensor, ValueType};
use std::path::Path;
use tokenizer::ClipTokenizer;

/// Identifies the embedding space; stored next to every vector.
pub const MODEL_ID: &str = "clip-vit-b32-q8";
/// Bump when preprocessing changes (forces re-embedding).
pub const EMBED_VERSION: i64 = 1;
pub const INPUT: u32 = 224;
const MEAN: [f32; 3] = [0.481_454_66, 0.457_827_5, 0.408_210_73];
const STD: [f32; 3] = [0.268_629_54, 0.261_302_6, 0.275_777_1];

/// CLIP preprocessing. Near-square images get the standard resize + centre
/// crop. Wide or tall images (most screenshots) are letterboxed onto a neutral
/// background instead, so the whole screen is seen rather than its middle.
pub fn preprocess(img: &RgbImage) -> Vec<f32> {
    let (w, h) = img.dimensions();
    let aspect = w.max(h) as f32 / w.min(h).max(1) as f32;
    let square = if aspect <= 1.3 {
        let s = INPUT as f32 / w.min(h) as f32;
        let (rw, rh) = (((w as f32 * s).round() as u32).max(INPUT), ((h as f32 * s).round() as u32).max(INPUT));
        let r = imgutil::resize(img, rw, rh, FilterType::CatmullRom);
        image::imageops::crop_imm(&r, (rw - INPUT) / 2, (rh - INPUT) / 2, INPUT, INPUT).to_image()
    } else {
        let s = INPUT as f32 / w.max(h) as f32;
        let (rw, rh) = (((w as f32 * s).round() as u32).max(1), ((h as f32 * s).round() as u32).max(1));
        let r = imgutil::resize(img, rw, rh, FilterType::CatmullRom);
        let fill = Rgb([(MEAN[0] * 255.0) as u8, (MEAN[1] * 255.0) as u8, (MEAN[2] * 255.0) as u8]);
        let mut canvas = RgbImage::from_pixel(INPUT, INPUT, fill);
        image::imageops::overlay(&mut canvas, &r, ((INPUT - rw) / 2) as i64, ((INPUT - rh) / 2) as i64);
        canvas
    };
    let plane = (INPUT * INPUT) as usize;
    let mut data = vec![0f32; plane * 3];
    for (i, p) in square.pixels().enumerate() {
        for c in 0..3 {
            data[c * plane + i] = (p[c] as f32 / 255.0 - MEAN[c]) / STD[c];
        }
    }
    data
}

/// Picks the projected-embedding output ("image_embeds"/"text_embeds"), else the
/// first 2-D output.
fn embedding_output(s: &Session) -> Result<String> {
    let outs = s.outputs();
    if let Some(o) = outs.iter().find(|o| o.name().ends_with("embeds")) {
        return Ok(o.name().to_string());
    }
    outs.iter()
        .find(|o| matches!(o.dtype(), ValueType::Tensor { shape, .. } if shape.len() == 2))
        .map(|o| o.name().to_string())
        .ok_or_else(|| anyhow!("model has no 2-D embedding output"))
}

pub struct ClipImage {
    session: Session,
    input: String,
    output: String,
}

impl ClipImage {
    pub fn load(model: &Path, threads: usize) -> Result<ClipImage> {
        let session = runtime::session(model, threads)?;
        let input = session.inputs().first().ok_or_else(|| anyhow!("vision model has no inputs"))?.name().to_string();
        let output = embedding_output(&session)?;
        Ok(ClipImage { session, input, output })
    }
}

impl ImageEmbedder for ClipImage {
    fn embed_image(&mut self, img: &RgbImage) -> Result<Vec<f32>> {
        let data = preprocess(img);
        let t = Tensor::from_array(([1usize, 3, INPUT as usize, INPUT as usize], data))?;
        let out = self.session.run(ort::inputs![self.input.as_str() => t])?;
        let (_, v) = out[self.output.as_str()].try_extract_tensor::<f32>()?;
        let mut v = v.to_vec();
        if v.is_empty() {
            bail!("empty embedding");
        }
        normalize(&mut v);
        Ok(v)
    }
}

pub struct ClipText {
    session: Session,
    ids_name: String,
    mask_name: Option<String>,
    output: String,
}

impl ClipText {
    pub fn load(model: &Path, threads: usize) -> Result<ClipText> {
        let session = runtime::session(model, threads)?;
        let names: Vec<String> = session.inputs().iter().map(|i| i.name().to_string()).collect();
        let ids_name = names.iter().find(|n| n.contains("input_ids")).cloned().or_else(|| names.first().cloned());
        let ids_name = ids_name.ok_or_else(|| anyhow!("text model has no inputs"))?;
        let mask_name = names.iter().find(|n| n.contains("attention_mask")).cloned();
        let output = embedding_output(&session)?;
        Ok(ClipText { session, ids_name, mask_name, output })
    }
}

impl TextEmbedder for ClipText {
    fn embed_text(&mut self, text: &str) -> Result<Vec<f32>> {
        let (ids, mask) = ClipTokenizer::shared().encode_padded(text);
        let n = ids.len();
        let ids_t = Tensor::from_array(([1usize, n], ids))?;
        let out = match &self.mask_name {
            Some(m) => {
                let mask_t = Tensor::from_array(([1usize, n], mask))?;
                self.session.run(ort::inputs![self.ids_name.as_str() => ids_t, m.as_str() => mask_t])?
            }
            None => self.session.run(ort::inputs![self.ids_name.as_str() => ids_t])?,
        };
        let (_, v) = out[self.output.as_str()].try_extract_tensor::<f32>()?;
        let mut v = v.to_vec();
        normalize(&mut v);
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocess_shapes_and_letterbox() {
        let wide = RgbImage::from_pixel(1920, 1080, Rgb([255, 255, 255]));
        let d = preprocess(&wide);
        assert_eq!(d.len(), 3 * 224 * 224);
        // Top-left corner is letterbox fill (≈ mean colour -> ≈ 0 after normalisation).
        assert!(d[0].abs() < 0.05, "{}", d[0]);
        // Centre is white.
        let centre = 112 * 224 + 112;
        assert!(d[centre] > 1.5);
        let sq = RgbImage::from_pixel(300, 280, Rgb([0, 0, 0]));
        let d = preprocess(&sq);
        assert!(d[0] < -1.5, "centre-cropped, no letterbox");
    }
}
