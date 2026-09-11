//! Tokenisation over canonical text.
//!
//! Deliberately *not* UAX#29 word segmentation. The Unicode algorithm breaks on hyphens
//! and slashes, which would shred `DS-2291` and `SOP-114/rev3` into fragments — the exact
//! tokens the identifier stage exists to find. Internal separators between alphanumerics
//! are held together; only leading and trailing punctuation is trimmed.
//!
//! The apostrophe is deliberately *not* one of those separators. No identifier scheme
//! uses one, and counting `anyone’s` as a two-segment coded token paid it the same
//! orthographic bonus as `DS-2291` — which put a possessive at the top of a real
//! document's keyword list.

use std::ops::Range;

/// Separators that join parts of a single token rather than ending one.
const INTERNAL: &[char] = &['-', '_', '/', '.', '+'];

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

/// Split canonical text into sentences with soft line wraps healed.
///
/// [`sentences`] terminates on every newline, which is right when a line genuinely is its
/// own unit — a spreadsheet row, a slide bullet, a form field. It is wrong for wrapped
/// prose, which breaks lines mid-clause constantly: in a PDF, `high performance liquid\n
/// chromatography (HPLC)` is one clause, not two fragments, and canonical text carries one
/// newline per *rendered* line. Taken at its word, [`sentences`] measures column width.
///
/// The distinguishing rule is the one dehyphenation already uses: a break is a *soft wrap*
/// when the text before it did not end in sentence punctuation **and** the text after it
/// begins lowercase. Anything else — a blank line, a new capitalised line, a full stop —
/// stays a boundary.
///
/// This is the unit for anything asking "are these words in the same sentence": the prose
/// gate, Stage 2's definition scope, and Stage 3's dispersion feature. It is emphatically
/// *not* the unit for "are these words adjacent on the page" — that question is answered
/// by looking for a newline in the raw gap between two tokens, and joining clauses here
/// does not disturb it. Keeping both views available is why canonical text retains its
/// line breaks rather than being reflowed.
pub fn clauses(text: &str) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();

    for span in sentences(text) {
        let Some(previous) = out.last_mut() else {
            out.push(span);
            continue;
        };
        let ended = text[previous.clone()]
            .trim_end()
            .ends_with(['.', '!', '?', ':', ';']);
        let continues = text[span.clone()]
            .trim_start()
            .chars()
            .next()
            .is_some_and(char::is_lowercase);

        if !ended && continues {
            previous.end = span.end;
        } else {
            out.push(span);
        }
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
    fn a_wrapped_prose_block_is_one_clause() {
        // The corpus failure in miniature: canonical PDF text carries one newline per
        // rendered line, and read as sentences that is a measurement of column width.
        let text = "Armature gives every scientist a single\n\
                    workspace where data capture, analysis and\n\
                    reporting live together.";
        assert_eq!(sentences(text).len(), 3);
        assert_eq!(clauses(text).len(), 1, "soft wraps were not healed");
    }

    #[test]
    fn a_heading_does_not_absorb_the_line_below_it() {
        // Both conditions must hold. `ARMATURE CODE` ends without punctuation, but the
        // next line starts uppercase, so it is a heading rather than a wrapped clause.
        let text = "ARMATURE CODE\nStart today.";
        assert_eq!(clauses(text).len(), 2, "a heading swallowed the line after it");
    }

    #[test]
    fn table_rows_are_never_joined_to_each_other() {
        // Rows begin with their own capitalised cell, so the lowercase condition fails
        // and each stays its own unit — which is what the prose gate needs to see.
        let text = "Batch\tCell\tYield\nDS-2291\tHEK293T\t91.4\nDS-2292\tHEK293T\t88.1";
        assert_eq!(clauses(text).len(), 3, "spliced table rows into one clause");
    }

    #[test]
    fn a_clause_covers_exactly_the_text_it_spans() {
        // Clauses are extended by moving an end offset, so a bug here would silently
        // misplace every offset Stage 2 and Stage 3 derive from them.
        let text = "one two three\nfour five six.\nSeven eight nine.";
        for span in clauses(text) {
            assert!(text.is_char_boundary(span.start) && text.is_char_boundary(span.end));
        }
        let spans = clauses(text);
        assert_eq!(spans[0].start, 0);
        assert_eq!(spans.last().unwrap().end, text.len());
    }

    #[test]
    fn an_apostrophe_is_not_an_identifier_separator() {
        // `anyone’s` was scoring the same two-segment bonus as `DS-2291`.
        assert_eq!(separator_segments("anyone\u{2019}s"), 1);
        assert_eq!(separator_segments("don't"), 1);
        assert_eq!(separator_segments("DS-2291"), 2);
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
