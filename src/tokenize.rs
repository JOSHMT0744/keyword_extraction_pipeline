//! Tokenisation over canonical text.
//!
//! Deliberately *not* UAX#29 word segmentation. The Unicode algorithm breaks on hyphens
//! and slashes, which would shred `DS-2291` and `SOP-114/rev3` into fragments — the exact
//! tokens the identifier lane exists to find. Internal separators between alphanumerics
//! are held together; only leading and trailing punctuation is trimmed.

use std::ops::Range;

/// Separators that join parts of a single token rather than ending one.
const INTERNAL: &[char] = &['-', '_', '/', '.', '+', '\'', '’'];

#[derive(Debug, Clone, PartialEq)]
pub struct Token<'a> {
    pub text: &'a str,
    /// Byte offsets into the canonical text.
    pub span: Range<usize>,
}

impl Token<'_> {
    pub fn lower(&self) -> String {
        self.text.to_lowercase()
    }
}

/// Split canonical text into candidate tokens with byte offsets.
pub fn tokens(text: &str) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    for (offset, chunk) in whitespace_chunks(text) {
        if let Some((start, end)) = trim_edges(chunk) {
            let span = (offset + start)..(offset + end);
            out.push(Token { text: &text[span.clone()], span });
        }
    }
    out
}

fn whitespace_chunks(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.split_whitespace()
        .map(move |c| (c.as_ptr() as usize - text.as_ptr() as usize, c))
}

/// Trim leading and trailing punctuation, keeping internal separators.
///
/// Returns `None` when nothing lexical remains. A trailing full stop is always trimmed —
/// `SOP-114.` at the end of a sentence must match `SOP-114` — while internal stops
/// survive, so `v2.14.3` stays whole.
fn trim_edges(chunk: &str) -> Option<(usize, usize)> {
    let is_edge_punct = |c: char| !c.is_alphanumeric();

    let start = chunk.find(|c: char| !is_edge_punct(c))?;
    let end = chunk.rfind(|c: char| !is_edge_punct(c)).map(|i| {
        i + chunk[i..].chars().next().map(char::len_utf8).unwrap_or(1)
    })?;

    (start < end).then_some((start, end))
}

/// Whether a token carries an internal separator joining alphanumeric runs.
///
/// Counts the resulting segments: `DS-2291` is two, `SOP-114/rev3` is three.
pub fn separator_segments(text: &str) -> usize {
    let segments: Vec<&str> = text
        .split(|c| INTERNAL.contains(&c))
        .filter(|s| !s.is_empty())
        .collect();
    if segments.len() > 1 && segments.iter().all(|s| s.chars().any(char::is_alphanumeric)) {
        segments.len()
    } else {
        1
    }
}

/// Split canonical text into sentences.
///
/// Used by the prose heuristic and the Schwartz–Hearst pass. A terminator only ends a
/// sentence when followed by whitespace and an uppercase or digit start, so `v2.14.3` and
/// `Fig. 4` do not fabricate boundaries.
pub fn sentences(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;

    for (i, c) in text.char_indices() {
        let terminator = matches!(c, '.' | '!' | '?' | '\n');
        if !terminator {
            continue;
        }
        let after = i + c.len_utf8();
        let ends = c == '\n'
            || text[after..]
                .chars()
                .next()
                .is_some_and(|n| n.is_whitespace())
                && text[after..]
                    .chars()
                    .find(|n| !n.is_whitespace())
                    .is_some_and(|n| n.is_uppercase() || n.is_ascii_digit());

        if ends && after > start {
            let _ = bytes;
            if text[start..after].trim().len() > 1 {
                out.push(start..after);
            }
            start = after;
        }
    }
    if start < text.len() && text[start..].trim().len() > 1 {
        out.push(start..text.len());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(s: &str) -> Vec<&str> {
        tokens(s).into_iter().map(|t| t.text).collect()
    }

    #[test]
    fn keeps_identifiers_whole() {
        assert_eq!(texts("Batch DS-2291 and SOP-114/rev3."), ["Batch", "DS-2291", "and", "SOP-114/rev3"]);
    }

    #[test]
    fn trims_sentence_punctuation_but_not_internal_stops() {
        assert_eq!(texts("(see v2.14.3), yes."), ["see", "v2.14.3", "yes"]);
    }

    #[test]
    fn offsets_point_at_the_token() {
        let text = "Batch DS-2291 released";
        let t = &tokens(text)[1];
        assert_eq!(&text[t.span.clone()], "DS-2291");
    }

    #[test]
    fn drops_pure_punctuation() {
        assert!(texts("--- *** ---").is_empty());
    }

    #[test]
    fn counts_separator_segments() {
        assert_eq!(separator_segments("DS-2291"), 2);
        assert_eq!(separator_segments("SOP-114/rev3"), 3);
        assert_eq!(separator_segments("ordinary"), 1);
        // A trailing separator leaves one lexical segment, not two.
        assert_eq!(separator_segments("word-"), 1);
    }

    #[test]
    fn splits_sentences_without_breaking_on_versions() {
        let text = "Run v2.14.3 completed. The next run failed.";
        let s = sentences(text);
        assert_eq!(s.len(), 2, "got {:?}", s.iter().map(|r| &text[r.clone()]).collect::<Vec<_>>());
        assert!(text[s[0].clone()].contains("v2.14.3"));
    }

    #[test]
    fn treats_line_breaks_as_sentence_boundaries() {
        let text = "Column Regeneration Report\nBatch DS-2291 was purified";
        assert_eq!(sentences(text).len(), 2);
    }
}
