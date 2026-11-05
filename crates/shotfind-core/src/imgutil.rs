//! Image decoding and resizing helpers.

use anyhow::{bail, Result};
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use image::{DynamicImage, ImageReader, Limits, RgbImage};
use std::io::Cursor;

/// Reads only the header to get dimensions (cheap, no pixel decode).
pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    Ok(ImageReader::new(Cursor::new(bytes)).with_guessed_format()?.into_dimensions()?)
}

/// Decodes with hard memory limits (decompression-bomb safe) and flattens any
/// transparency onto white, which is what OCR and CLIP expect for screenshots.
pub fn decode_rgb(bytes: &[u8], max_pixels: u64) -> Result<RgbImage> {
    let (w, h) = dimensions(bytes)?;
    if (w as u64) * (h as u64) > max_pixels {
        bail!("image is {w}x{h}, over the pixel limit");
    }
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(w.max(1));
    limits.max_image_height = Some(h.max(1));
    limits.max_alloc = Some(max_pixels.saturating_mul(8).max(64 << 20));
    reader.limits(limits);
    let img = reader.decode()?;
    Ok(flatten(img))
}

fn flatten(img: DynamicImage) -> RgbImage {
    if !img.color().has_alpha() {
        return img.into_rgb8();
    }
    let rgba = img.into_rgba8();
    let (w, h) = rgba.dimensions();
    let mut out = RgbImage::new(w, h);
    for (src, dst) in rgba.pixels().zip(out.pixels_mut()) {
        let a = src[3] as u32;
        for c in 0..3 {
            dst[c] = ((src[c] as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
        }
    }
    out
}

pub fn resize(img: &RgbImage, w: u32, h: u32, filter: FilterType) -> RgbImage {
    let (w, h) = (w.max(1), h.max(1));
    if img.dimensions() == (w, h) {
        return img.clone();
    }
    let mut dst = RgbImage::new(w, h);
    let mut r = Resizer::new();
    let opts = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(filter));
    r.resize(img, &mut dst, &opts).expect("same pixel type");
    dst
}

/// Scales so the longest side is at most `max_side` (never upscales).
pub fn fit_within(img: &RgbImage, max_side: u32) -> RgbImage {
    let (w, h) = img.dimensions();
    let long = w.max(h);
    if long <= max_side {
        return img.clone();
    }
    let s = max_side as f64 / long as f64;
    resize(img, ((w as f64 * s).round() as u32).max(1), ((h as f64 * s).round() as u32).max(1), FilterType::Bilinear)
}

pub fn encode_jpeg(img: &RgbImage, quality: u8) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(32 * 1024);
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
    img.write_with_encoder(enc)?;
    Ok(out)
}

/// Screen-like dimensions (desktop resolutions or phone screens).
pub fn looks_like_screen_dims(w: u32, h: u32) -> bool {
    const WIDTHS: &[u32] = &[
        1280, 1366, 1440, 1536, 1600, 1680, 1920, 2048, 2160, 2256, 2560, 2736, 2880, 2960, 3000, 3024, 3200, 3440,
        3456, 3840, 5120,
    ];
    const PHONE_WIDTHS: &[u32] =
        &[640, 720, 750, 828, 1080, 1125, 1170, 1179, 1206, 1242, 1260, 1284, 1290, 1320, 1440];
    let (long, short) = (w.max(h), w.min(h));
    let desktop = w > h && WIDTHS.contains(&w) && (h as f64 / w as f64 - 9.0 / 16.0).abs() < 0.13;
    let phone = h > w && PHONE_WIDTHS.contains(&w) && (long as f64 / short as f64) >= 1.7;
    desktop || phone
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatten_transparent_to_white() {
        let mut img = image::RgbaImage::new(2, 1);
        img.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        img.put_pixel(1, 0, image::Rgba([10, 20, 30, 255]));
        let out = flatten(DynamicImage::ImageRgba8(img));
        assert_eq!(out.get_pixel(0, 0).0, [255, 255, 255]);
        assert_eq!(out.get_pixel(1, 0).0, [10, 20, 30]);
    }

    #[test]
    fn screen_dims() {
        assert!(looks_like_screen_dims(1920, 1080));
        assert!(looks_like_screen_dims(2560, 1600));
        assert!(looks_like_screen_dims(1080, 2400));
        assert!(!looks_like_screen_dims(4032, 3024)); // camera photo
        assert!(!looks_like_screen_dims(512, 512));
    }

    #[test]
    fn bomb_guard() {
        let img = RgbImage::new(100, 100);
        let mut buf = Vec::new();
        DynamicImage::ImageRgb8(img).write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
        assert!(decode_rgb(&buf, 100 * 100).is_ok());
        assert!(decode_rgb(&buf, 100 * 99).is_err());
    }
}
