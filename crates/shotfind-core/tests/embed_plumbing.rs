//! Exercises the CLIP loading/inference plumbing with tiny hand-built ONNX
//! models that have the same input/output names and shapes as the real ones.
//! Needs ONNX Runtime (ORT_DYLIB_PATH), so it is ignored by default.

use shotfind_core::embed::{ClipImage, ClipText};
use shotfind_core::pipeline::{ImageEmbedder, TextEmbedder};

// --- minimal protobuf writer -------------------------------------------------
fn varint(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
}
fn key(field: u32, wire: u32, out: &mut Vec<u8>) {
    varint(((field << 3) | wire) as u64, out);
}
fn int(field: u32, v: i64, out: &mut Vec<u8>) {
    key(field, 0, out);
    varint(v as u64, out);
}
fn bytes(field: u32, b: &[u8], out: &mut Vec<u8>) {
    key(field, 2, out);
    varint(b.len() as u64, out);
    out.extend_from_slice(b);
}
fn msg(field: u32, f: impl FnOnce(&mut Vec<u8>), out: &mut Vec<u8>) {
    let mut m = Vec::new();
    f(&mut m);
    bytes(field, &m, out);
}

const FLOAT: i64 = 1;
const INT64: i64 = 7;

/// ValueInfoProto with a tensor type; dims: Some(n) fixed, None symbolic.
fn value_info(field: u32, name: &str, elem: i64, dims: &[Option<i64>], out: &mut Vec<u8>) {
    msg(
        field,
        |vi| {
            bytes(1, name.as_bytes(), vi);
            msg(
                2,
                |tp| {
                    msg(
                        1,
                        |tt| {
                            int(1, elem, tt);
                            msg(
                                2,
                                |shape| {
                                    for (i, d) in dims.iter().enumerate() {
                                        msg(
                                            1,
                                            |dim| match d {
                                                Some(v) => int(1, *v, dim),
                                                None => bytes(2, format!("d{i}").as_bytes(), dim),
                                            },
                                            shape,
                                        );
                                    }
                                },
                                tt,
                            );
                        },
                        tp,
                    );
                },
                vi,
            );
        },
        out,
    );
}

fn node(op: &str, inputs: &[&str], outputs: &[&str], attrs: impl FnOnce(&mut Vec<u8>), out: &mut Vec<u8>) {
    msg(
        1,
        |n| {
            for i in inputs {
                bytes(1, i.as_bytes(), n);
            }
            for o in outputs {
                bytes(2, o.as_bytes(), n);
            }
            bytes(4, op.as_bytes(), n);
            attrs(n);
        },
        out,
    );
}

fn attr_ints(name: &str, vals: &[i64], out: &mut Vec<u8>) {
    msg(
        5,
        |a| {
            bytes(1, name.as_bytes(), a);
            for v in vals {
                int(8, *v, a);
            }
            int(20, 7, a); // INTS
        },
        out,
    );
}
fn attr_int(name: &str, v: i64, out: &mut Vec<u8>) {
    msg(
        5,
        |a| {
            bytes(1, name.as_bytes(), a);
            int(3, v, a);
            int(20, 2, a); // INT
        },
        out,
    );
}

fn initializer(name: &str, dims: &[i64], data: &[f32], out: &mut Vec<u8>) {
    msg(
        5,
        |t| {
            for d in dims {
                int(1, *d, t);
            }
            int(2, FLOAT, t);
            bytes(8, name.as_bytes(), t);
            let raw: Vec<u8> = data.iter().flat_map(|f| f.to_le_bytes()).collect();
            bytes(9, &raw, t);
        },
        out,
    );
}

fn model(graph: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut m = Vec::new();
    int(1, 8, &mut m); // ir_version
    bytes(2, b"shotfind-test", &mut m);
    msg(7, graph, &mut m);
    msg(8, |o| int(2, 13, o), &mut m); // opset 13
    m
}

const D: i64 = 8;

/// pixel_values [N,3,224,224] -> mean over H,W -> [N,3] x W[3,D] -> image_embeds [N,D]
fn vision_model() -> Vec<u8> {
    model(|g| {
        node(
            "ReduceMean",
            &["pixel_values"],
            &["pooled"],
            |n| {
                attr_ints("axes", &[2, 3], n);
                attr_int("keepdims", 0, n);
            },
            g,
        );
        node("MatMul", &["pooled", "W"], &["image_embeds"], |_| {}, g);
        bytes(2, b"vision", g);
        let w: Vec<f32> = (0..3 * D).map(|i| ((i * 7 % 11) as f32 - 5.0) / 5.0).collect();
        initializer("W", &[3, D], &w, g);
        value_info(11, "pixel_values", FLOAT, &[None, Some(3), Some(224), Some(224)], g);
        value_info(12, "image_embeds", FLOAT, &[None, Some(D)], g);
    })
}

/// input_ids [N,S] int64 -> float -> mean over S -> [N,1] x W[1,D] (+bias) -> text_embeds [N,D]
fn text_model() -> Vec<u8> {
    model(|g| {
        node("Cast", &["input_ids"], &["ids_f"], |n| attr_int("to", FLOAT, n), g);
        node(
            "ReduceMean",
            &["ids_f"],
            &["m"],
            |n| {
                attr_ints("axes", &[1], n);
                attr_int("keepdims", 1, n);
            },
            g,
        );
        node("MatMul", &["m", "W"], &["mw"], |_| {}, g);
        node("Add", &["mw", "B"], &["text_embeds"], |_| {}, g);
        bytes(2, b"text", g);
        let w: Vec<f32> = (0..D).map(|i| (i as f32 + 1.0) / 1000.0).collect();
        let b: Vec<f32> = (0..D).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect();
        initializer("W", &[1, D], &w, g);
        initializer("B", &[D], &b, g);
        value_info(11, "input_ids", INT64, &[None, None], g);
        value_info(11, "attention_mask", INT64, &[None, None], g);
        value_info(12, "text_embeds", FLOAT, &[None, Some(D)], g);
    })
}

#[test]
#[ignore]
fn clip_plumbing_with_tiny_models() {
    shotfind_core::runtime::init(&[]).expect("set ORT_DYLIB_PATH");
    let dir = tempfile::tempdir().unwrap();
    let (v, t) = (dir.path().join("vision.onnx"), dir.path().join("text.onnx"));
    std::fs::write(&v, vision_model()).unwrap();
    std::fs::write(&t, text_model()).unwrap();

    let mut vis = ClipImage::load(&v, 1).unwrap();
    let red = image::RgbImage::from_pixel(1920, 1080, image::Rgb([250, 10, 10]));
    let blue = image::RgbImage::from_pixel(400, 400, image::Rgb([10, 10, 250]));
    let (er, eb) = (vis.embed_image(&red).unwrap(), vis.embed_image(&blue).unwrap());
    assert_eq!(er.len(), D as usize);
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm(&er) - 1.0).abs() < 1e-4, "unit length");
    let cos: f32 = er.iter().zip(&eb).map(|(a, b)| a * b).sum();
    assert!(cos < 0.99, "different images give different vectors (cos={cos})");

    let mut txt = ClipText::load(&t, 1).unwrap();
    let a = txt.embed_text("a photo of a cat").unwrap();
    let b = txt.embed_text("a photo of a cat").unwrap();
    assert_eq!(a, b, "deterministic");
    assert_eq!(a.len(), D as usize);
    assert!((norm(&a) - 1.0).abs() < 1e-4);
}
