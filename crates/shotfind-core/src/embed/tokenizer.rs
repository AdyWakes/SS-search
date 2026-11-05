//! CLIP byte-level BPE tokenizer (port of openai/CLIP `simple_tokenizer.py`).
//! The merges file (MIT, from openai/CLIP) is compiled into the binary.

use regex::Regex;
use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

const VOCAB_GZ: &[u8] = include_bytes!("../../assets/bpe_simple_vocab_16e6.txt.gz");
pub const SOT: i64 = 49406;
pub const EOT: i64 = 49407;
pub const CONTEXT: usize = 77;

pub struct ClipTokenizer {
    encoder: HashMap<String, i64>,
    ranks: HashMap<(String, String), usize>,
    byte_enc: [char; 256],
    pat: Regex,
    cache: parking_lot::Mutex<HashMap<String, Vec<i64>>>,
}

/// byte -> char table, plus the chars in OpenAI's insertion order (which is
/// the order of the base vocabulary).
fn bytes_to_unicode() -> ([char; 256], Vec<char>) {
    let mut bs: Vec<u32> = (b'!' as u32..=b'~' as u32).chain(0xA1..=0xAC).chain(0xAE..=0xFF).collect();
    let mut cs = bs.clone();
    let mut n = 0;
    for b in 0..256u32 {
        if !bs.contains(&b) {
            bs.push(b);
            cs.push(256 + n);
            n += 1;
        }
    }
    let mut out = ['\0'; 256];
    let mut order = Vec::with_capacity(256);
    for (b, c) in bs.into_iter().zip(cs) {
        let ch = char::from_u32(c).unwrap();
        out[b as usize] = ch;
        order.push(ch);
    }
    (out, order)
}

impl ClipTokenizer {
    pub fn shared() -> &'static ClipTokenizer {
        static T: OnceLock<ClipTokenizer> = OnceLock::new();
        T.get_or_init(ClipTokenizer::new)
    }

    fn new() -> ClipTokenizer {
        let mut text = String::new();
        flate2::read::GzDecoder::new(VOCAB_GZ).read_to_string(&mut text).expect("bundled vocab");
        let merges: Vec<(String, String)> = text
            .lines()
            .skip(1)
            .take(49152 - 256 - 2)
            .map(|l| {
                let mut it = l.split(' ');
                (it.next().unwrap_or("").to_string(), it.next().unwrap_or("").to_string())
            })
            .collect();
        let (byte_enc, order) = bytes_to_unicode();
        let mut vocab: Vec<String> = order.iter().map(|c| c.to_string()).collect();
        vocab.extend(order.iter().map(|c| format!("{c}</w>")));
        vocab.extend(merges.iter().map(|(a, b)| format!("{a}{b}")));
        vocab.push("<|startoftext|>".into());
        vocab.push("<|endoftext|>".into());
        let encoder = vocab.into_iter().enumerate().map(|(i, s)| (s, i as i64)).collect();
        let ranks = merges.into_iter().enumerate().map(|(i, m)| (m, i)).collect();
        let pat =
            Regex::new(r"<\|startoftext\|>|<\|endoftext\|>|'s|'t|'re|'ve|'m|'ll|'d|\p{L}+|\p{N}|[^\s\p{L}\p{N}]+")
                .expect("valid regex");
        ClipTokenizer { encoder, ranks, byte_enc, pat, cache: Default::default() }
    }

    fn bpe(&self, token: &str) -> Vec<i64> {
        if let Some(v) = self.cache.lock().get(token) {
            return v.clone();
        }
        let chars: Vec<char> = token.chars().collect();
        let mut word: Vec<String> = chars.iter().map(|c| c.to_string()).collect();
        if let Some(last) = word.last_mut() {
            last.push_str("</w>");
        }
        loop {
            if word.len() < 2 {
                break;
            }
            let mut best: Option<(usize, usize)> = None;
            for i in 0..word.len() - 1 {
                if let Some(&r) = self.ranks.get(&(word[i].clone(), word[i + 1].clone())) {
                    if best.is_none_or(|(br, _)| r < br) {
                        best = Some((r, i));
                    }
                }
            }
            let Some((_, _)) = best else { break };
            let (first, second) = {
                let (_, i) = best.unwrap();
                (word[i].clone(), word[i + 1].clone())
            };
            let mut merged = Vec::with_capacity(word.len());
            let mut i = 0;
            while i < word.len() {
                if i + 1 < word.len() && word[i] == first && word[i + 1] == second {
                    merged.push(format!("{first}{second}"));
                    i += 2;
                } else {
                    merged.push(word[i].clone());
                    i += 1;
                }
            }
            word = merged;
        }
        let ids: Vec<i64> = word.iter().filter_map(|w| self.encoder.get(w).copied()).collect();
        self.cache.lock().insert(token.to_string(), ids.clone());
        ids
    }

    /// Token ids without SOT/EOT.
    pub fn encode(&self, text: &str) -> Vec<i64> {
        let clean = text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
        let mut out = Vec::new();
        for m in self.pat.find_iter(&clean) {
            let t: String = m.as_str().bytes().map(|b| self.byte_enc[b as usize]).collect();
            out.extend(self.bpe(&t));
        }
        out
    }

    /// SOT + tokens + EOT, truncated to the 77-token context. Returns
    /// (ids padded with zeros, attention mask).
    pub fn encode_padded(&self, text: &str) -> (Vec<i64>, Vec<i64>) {
        let mut ids = vec![SOT];
        let toks = self.encode(text);
        ids.extend(toks.into_iter().take(CONTEXT - 2));
        ids.push(EOT);
        let mut mask = vec![1i64; ids.len()];
        ids.resize(CONTEXT, 0);
        mask.resize(CONTEXT, 0);
        (ids, mask)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_reference_ids() {
        let t = ClipTokenizer::shared();
        // Reference ids from openai/CLIP's tokenizer.
        assert_eq!(t.encode("a photo of a cat"), vec![320, 1125, 539, 320, 2368]);
        assert_eq!(t.encode("a photo of a dog"), vec![320, 1125, 539, 320, 1929]);
        assert_eq!(t.encode("A   PHOTO of a CAT"), t.encode("a photo of a cat"));
        let (ids, mask) = t.encode_padded("a cat");
        assert_eq!(&ids[..4], &[SOT, 320, 2368, EOT]);
        assert_eq!(ids.len(), CONTEXT);
        assert_eq!(mask.iter().sum::<i64>(), 4);
    }

    #[test]
    fn long_text_truncates() {
        let long = "word ".repeat(200);
        let (ids, _) = ClipTokenizer::shared().encode_padded(&long);
        assert_eq!(ids.len(), CONTEXT);
        assert_eq!(ids[CONTEXT - 1], EOT);
    }
}
