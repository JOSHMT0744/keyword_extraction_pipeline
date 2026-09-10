//! Canonicalisation: raw text to the single canonical form everything else indexes into.
//!
//! This function's behaviour *is* a large part of [`crate::PipelineVersion`], and every
//! keyword offset in the system is relative to its output. Changing it invalidates every
//! stored keyword set, so changes belong in a deliberate version bump.
//!
//! Exposed via [`crate::canonicalise`] so a caller can regenerate the exact text on demand
//! to resolve offsets. `extract` never returns the text itself: the consumer holding full
//! document text is a retention decision, not an API convenience.

use unicode_normalization::UnicodeNormalization;

use crate::{config::Config, parse::SourceKind};

/// Discretionary hyphen: a rendering hint with no lexical content. Left in place it
/// splits tokens invisibly, so `DS-2291` and `DS\u{ad}-2291` would not match.
const SOFT_HYPHEN: char = '\u{00ad}';

pub fn canonicalise(raw: &str, source: SourceKind, cfg: &Config) -> String {
    let text = raw.replace("\r\n", "\n").replace('\r', "\n");
    // PDF page separator. A page break is a line break for our purposes.
    let text = text.replace('\x0c', "\n");

    let text = if source == SourceKind::Email && cfg.strip_quoted_blocks {
        strip_quoted_and_signature(&text)
    } else {
        text
    };

    let text = normalise_unicode(&text);
    let text = dehyphenate_line_breaks(&text);
    collapse_whitespace(&text)
}

/// NFKC, then fold the typographic hyphens that are lexically identical to ASCII.
///
/// Only true hyphens are folded. En and em dashes are punctuation between words, not
/// parts of tokens, and folding them would splice unrelated words into false compounds.
fn normalise_unicode(text: &str) -> String {
    text.nfkc()
        .filter(|&c| c != SOFT_HYPHEN)
        .map(|c| match c {
            '\u{2010}' | '\u{2011}' => '-', // hyphen, non-breaking hyphen
            '\u{00a0}' | '\u{2007}' | '\u{202f}' => ' ', // non-breaking spaces
            c => c,
        })
        .collect()
}

/// Rejoin words split across a line break by justification.
///
/// Only joins when the next line starts lowercase. `MabSelect SuRe-\nCapto` is a genuine
/// hyphenated pair, not a split word, and joining it would fabricate a token that appears
/// nowhere in the document.
fn dehyphenate_line_breaks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut lines = text.split('\n').peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim_end();
        let joins = trimmed.ends_with('-')
            && trimmed.len() > 1
            && trimmed[..trimmed.len() - 1].ends_with(|c: char| c.is_alphabetic())
            && lines
                .peek()
                .is_some_and(|n| n.trim_start().starts_with(|c: char| c.is_lowercase()));

        if joins {
            out.push_str(&trimmed[..trimmed.len() - 1]);
        } else {
            out.push_str(line);
            if lines.peek().is_some() {
                out.push('\n');
            }
        }
    }
    out
}

/// Collapse horizontal runs to one space and vertical runs to one blank line, then trim.
///
/// Paragraph boundaries survive because the prose heuristic and the Schwartz–Hearst pass
/// both need sentence and block structure to mean something.
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0usize;

    for line in text.split('\n') {
        let mut collapsed = String::with_capacity(line.len());
        let mut in_space = false;
        for c in line.chars() {
            if c.is_whitespace() {
                in_space = true;
            } else {
                if in_space && !collapsed.is_empty() {
                    collapsed.push(' ');
                }
                in_space = false;
                collapsed.push(c);
            }
        }

        if collapsed.is_empty() {
            blank_run += 1;
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
            if blank_run > 0 {
                out.push('\n');
            }
        }
        blank_run = 0;
        out.push_str(&collapsed);
    }
    out
}

/// Remove quoted blocks, their attribution lines, and the trailing signature.
///
/// Mandatory rather than an optimisation. A term appearing once in a forty-message thread
/// otherwise reads as occurring forty times, and every footer reads as ubiquitous — which
/// distorts in-document frequency, the one statistic this corpus-blind crate has.
fn strip_quoted_and_signature(text: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();

    for line in text.split('\n') {
        let t = line.trim_start();

        // RFC 3676 §4.3 signature delimiter: "-- " alone on a line. Everything after it
        // is the sender's boilerplate, repeated across every message they ever sent.
        if t == "--" || t == "-- " {
            break;
        }
        if t.starts_with('>') {
            continue;
        }
        if is_attribution(t) {
            continue;
        }
        kept.push(line);
    }

    kept.join("\n")
}

/// Recognise the attribution line a client writes above a quoted block.
///
/// Deliberately narrow: it must both open like an attribution and end in `wrote:`.
/// A looser rule silently eats real sentences, and a missing sentence is invisible.
fn is_attribution(line: &str) -> bool {
    let lower = line.trim().to_ascii_lowercase();
    if !lower.ends_with("wrote:") {
        return false;
    }
    lower.starts_with("on ") || lower.starts_with("at ") || lower.contains(" wrote:")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canon(s: &str) -> String {
        canonicalise(s, SourceKind::Plain, &Config::default())
    }

    #[test]
    fn collapses_runs_but_keeps_paragraphs() {
        assert_eq!(canon("a   b\n\n\n\nc  d"), "a b\n\nc d");
    }

    #[test]
    fn rejoins_words_split_across_lines() {
        assert_eq!(canon("the column was regen-\nerated today"), "the column was regenerated today");
    }

    #[test]
    fn does_not_join_a_genuine_trailing_hyphen() {
        // Next line starts uppercase, so the hyphen is a real one.
        assert_eq!(canon("MabSelect SuRe-\nCapto S"), "MabSelect SuRe-\nCapto S");
    }

    #[test]
    fn removes_soft_hyphens_that_would_split_identifiers() {
        assert_eq!(canon("DS\u{ad}-2291"), "DS-2291");
    }

    #[test]
    fn folds_typographic_hyphens_but_not_dashes() {
        assert_eq!(canon("DS\u{2010}2291"), "DS-2291");
        assert_eq!(canon("a \u{2014} b"), "a \u{2014} b");
    }

    #[test]
    fn page_breaks_become_line_breaks() {
        assert_eq!(canon("page one\x0cpage two"), "page one\npage two");
    }

    #[test]
    fn strips_quotes_attribution_and_signature() {
        let raw = "Confirmed, regenerated per SOP-114.\n\
                   \n\
                   On Tue, Bob wrote:\n\
                   > Has batch DS-2291 been released?\n\
                   > > Original question.\n\
                   \n\
                   -- \n\
                   Alice Example | Process Development";
        let out = canonicalise(raw, SourceKind::Email, &Config::default());

        assert!(out.contains("Confirmed, regenerated per SOP-114."));
        assert!(!out.contains("Has batch"), "quoted block survived: {out:?}");
        assert!(!out.contains("Bob wrote"), "attribution survived: {out:?}");
        assert!(!out.contains("Process Development"), "signature survived: {out:?}");
    }

    #[test]
    fn attribution_rule_does_not_eat_ordinary_sentences() {
        // A missing sentence is an invisible failure, so the rule must be narrow.
        let raw = "On the whole the results were good.\nWe wrote: the yield was high.";
        let out = canonicalise(raw, SourceKind::Email, &Config::default());
        assert!(out.contains("On the whole the results were good."));
    }

    #[test]
    fn quotes_are_kept_when_stripping_is_disabled() {
        let cfg = Config { strip_quoted_blocks: false, ..Config::default() };
        let out = canonicalise("hello\n> quoted", SourceKind::Email, &cfg);
        assert!(out.contains("quoted"));
    }

    #[test]
    fn is_idempotent() {
        // Canonical text must be a fixed point, or re-canonicalising to resolve an offset
        // would produce a different string from the one the offsets refer to.
        let once = canon("a  b\u{ad}c\x0c\n\n\nd-\ne");
        assert_eq!(canon(&once), once);
    }
}
