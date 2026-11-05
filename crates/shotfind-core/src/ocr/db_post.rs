//! DBNet post-processing: probability map -> text boxes.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxF {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub score: f32,
}

impl BoxF {
    pub fn area(&self) -> f32 {
        (self.x1 - self.x0).max(0.0) * (self.y1 - self.y0).max(0.0)
    }
    /// Intersection over the smaller box's area.
    pub fn overlap_ratio(&self, o: &BoxF) -> f32 {
        let iw = (self.x1.min(o.x1) - self.x0.max(o.x0)).max(0.0);
        let ih = (self.y1.min(o.y1) - self.y0.max(o.y0)).max(0.0);
        let small = self.area().min(o.area());
        if small <= 0.0 {
            0.0
        } else {
            iw * ih / small
        }
    }
}

/// Thresholds the map, dilates by one pixel, finds 8-connected components,
/// scores each by mean probability, and expands ("unclips") the shrunken
/// region back to the full text extent. Coordinates are in map pixels.
pub fn boxes_from_probmap(prob: &[f32], w: usize, h: usize, thresh: f32, box_thresh: f32, unclip: f32) -> Vec<BoxF> {
    assert_eq!(prob.len(), w * h);
    // Binary mask with 2x2 dilation (PaddleOCR `use_dilation`).
    let mut mask = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            if prob[y * w + x] > thresh {
                mask[y * w + x] = true;
                if x + 1 < w {
                    mask[y * w + x + 1] = true;
                }
                if y + 1 < h {
                    mask[(y + 1) * w + x] = true;
                    if x + 1 < w {
                        mask[(y + 1) * w + x + 1] = true;
                    }
                }
            }
        }
    }
    let mut seen = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    let mut boxes = Vec::new();
    for start in 0..w * h {
        if !mask[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        let (mut sum, mut n) = (0f32, 0usize);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            sum += prob[i];
            n += 1;
            let xs = x.saturating_sub(1)..=(x + 1).min(w - 1);
            for ny in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                for nx in xs.clone() {
                    let j = ny * w + nx;
                    if mask[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        let (bw, bh) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32);
        if bw.min(bh) < 3.0 || n < 6 {
            continue;
        }
        let score = sum / n as f32;
        if score < box_thresh {
            continue;
        }
        // Offset distance from the Vatti-clipping unclip: area * ratio / perimeter.
        let d = bw * bh * unclip / (2.0 * (bw + bh));
        boxes.push(BoxF {
            x0: (x0 as f32 - d).max(0.0),
            y0: (y0 as f32 - d).max(0.0),
            x1: (x1 as f32 + 1.0 + d).min(w as f32),
            y1: (y1 as f32 + 1.0 + d).min(h as f32),
            score,
        });
    }
    boxes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_two_separate_regions() {
        let (w, h) = (40, 20);
        let mut p = vec![0f32; w * h];
        for y in 3..7 {
            for x in 2..20 {
                p[y * w + x] = 0.9;
            }
        }
        for y in 12..16 {
            for x in 5..35 {
                p[y * w + x] = 0.8;
            }
        }
        p[0] = 0.95; // single-pixel noise is dropped
        let b = boxes_from_probmap(&p, w, h, 0.3, 0.5, 1.6);
        assert_eq!(b.len(), 2, "{b:?}");
        assert!(b[0].y0 + b[0].y1 < b[1].y0 + b[1].y1, "top region first");
        assert!(b[0].x0 < 2.0 && b[0].x1 > 20.0, "unclipped: {:?}", b[0]);
    }
}
