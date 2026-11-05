//! Text normalisation shared by indexing-side scoring and query parsing.
//! Mirrors FTS5's `unicode61 remove_diacritics 2`: lowercase, strip accents,
//! split on anything that is not a letter or digit.

use unicode_normalization::UnicodeNormalization;

pub fn fold(s: &str) -> String {
    s.nfd().filter(|c| !unicode_normalization::char::is_combining_mark(*c)).flat_map(|c| c.to_lowercase()).collect()
}

/// Lowercased, accent-free word tokens.
pub fn tokens(s: &str) -> Vec<String> {
    fold(s).split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).map(|t| t.to_string()).collect()
}

/// Allowed edit distance for a query token of this length (typo tolerance).
pub fn max_edits(token: &str) -> usize {
    if token.chars().all(|c| c.is_ascii_digit()) {
        return 0; // "404" must not match "401"
    }
    match token.chars().count() {
        0..=3 => 0,
        4..=6 => 1,
        _ => 2,
    }
}

/// Collapses characters OCR commonly confuses, so "UPl" ~ "UPI", "C0de" ~
/// "Code", "rnodern" ~ "modern". Applied to already-folded tokens.
pub fn ocr_fold(t: &str) -> String {
    let mapped: String = t
        .chars()
        .map(|c| match c {
            'i' | 'l' | '1' | '|' | '!' => 'l',
            'o' | '0' => 'o',
            '5' => 's',
            '8' => 'b',
            c => c,
        })
        .collect();
    mapped.replace("rn", "m").replace("vv", "w").replace("cl", "d")
}

/// Optimal string alignment distance (Levenshtein + adjacent transpositions),
/// bailing out early once `limit` is exceeded.
pub fn edit_distance(a: &str, b: &str, limit: usize) -> Option<usize> {
    let (la, lb) = (a.chars().count(), b.chars().count());
    if la.abs_diff(lb) > limit {
        return None;
    }
    let d = strsim::osa_distance(a, b);
    (d <= limit).then_some(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding_and_tokens() {
        assert_eq!(tokens("Café — UPI-Payment, Rahul!"), vec!["cafe", "upi", "payment", "rahul"]);
        assert_eq!(tokens("Größe ÉCOLE"), vec!["große", "ecole"]);
    }

    #[test]
    fn ocr_confusions() {
        assert_eq!(ocr_fold("upl"), ocr_fold("upi"));
        assert_eq!(ocr_fold("c0de"), ocr_fold("code"));
        assert_eq!(ocr_fold("rnodern"), ocr_fold("modern"));
        assert_ne!(ocr_fold("cat"), ocr_fold("car"));
    }

    #[test]
    fn edits() {
        assert_eq!(max_edits("404"), 0);
        assert_eq!(max_edits("cat"), 0);
        assert_eq!(max_edits("rahul"), 1);
        assert_eq!(max_edits("payment"), 2);
        assert_eq!(edit_distance("paymnet", "payment", 2), Some(1)); // transposition = 1
        assert_eq!(edit_distance("eror", "error", 1), Some(1));
        assert_eq!(edit_distance("upl", "upi", 1), Some(1));
        assert_eq!(edit_distance("kitten", "sitting", 2), None);
    }
}
