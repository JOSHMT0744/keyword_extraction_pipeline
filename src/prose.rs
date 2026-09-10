//! The deterministic prose gate.
//!
//! Lane 3 assumes running text. Applied to a spreadsheet it emits column headers as
//! topics; applied to an email footer it emits the disclaimer. Both are confident,
//! plausible and wrong — a failure worse than emitting nothing, because nothing is
//! visibly nothing while a wrong topic looks like a right one.
//!
//! The verdict is carried on [`crate::DocumentResult`] rather than consumed and dropped,
//! so that "no topical keywords" can be reported with its reason attached. A blank where
//! keywords should be is the same silent omission [`crate::DocumentStatus`] exists to
//! prevent, one level down; `is_prose: false` with no explanation would reintroduce it.

use serde::{Deserialize, Serialize};

use crate::{config::Config, parse::SourceKind, resources::Resources, tokenize};

/// Why the gate ruled as it did. Exactly one reason, the first that applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProseReason {
    /// Running text. Lane 3 ran.
    Prose,
    /// A spreadsheet or CSV. Rejected on format, not on measurement: a wide enough
    /// sheet can pass any ratio test, and the format already tells us the answer.
    InherentlyTabular,
    /// Not enough text for statistical keyphrase extraction to mean anything.
    TooFewTokens,
    /// Sentences too short on average — headings, bullets, form fields.
    SentencesTooShort,
    /// Too few function words. Prose is full of them; a list of values is not.
    TooFewStopwords,
    /// Too many lines that look like table or field rows.
    TooManyTableLines,
    /// Lane 3's statistics are tuned to English, and the document is not English.
    NotEnglish,
}

impl ProseReason {
    /// A phrase that completes "no topical keywords: …".
    pub fn explain(&self) -> &'static str {
        match self {
            ProseReason::Prose => "the document is running text",
            ProseReason::InherentlyTabular => "the format is inherently tabular",
            ProseReason::TooFewTokens => "too few tokens for statistical extraction",
            ProseReason::SentencesTooShort => "sentences are too short to be prose",
            ProseReason::TooFewStopwords => "too few function words to be prose",
            ProseReason::TooManyTableLines => "too many lines look like table rows",
            ProseReason::NotEnglish => "the topical lane is English-only",
        }
    }
}

/// The measurements behind the ruling, reported whether or not they decided it.
///
/// Kept even when the format short-circuits the decision, so a threshold can be moved
/// against real numbers rather than guessed at.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProseVerdict {
    pub is_prose: bool,
    pub reason: ProseReason,
    pub mean_sentence_len: f32,
    pub stopword_ratio: f32,
    pub table_line_ratio: f32,
    pub tokens: usize,
}

impl ProseVerdict {
    /// The gate's ruling as one line, for a person rather than a machine.
    pub fn summary(&self) -> String {
        format!(
            "{} (mean sentence {:.1} tokens, {:.0}% stopwords, {:.0}% table-like lines, {} tokens)",
            self.reason.explain(),
            self.mean_sentence_len,
            self.stopword_ratio * 100.0,
            self.table_line_ratio * 100.0,
            self.tokens
        )
    }
}

pub fn assess(
    text: &str,
    source: SourceKind,
    english: bool,
    cfg: &Config,
    res: &Resources,
) -> ProseVerdict {
    let tokens = tokenize::tokens(text);
    let sentences = tokenize::sentences(text);

    let stopwords = tokens.iter().filter(|t| res.is_stopword(&t.lower())).count();
    let stopword_ratio = ratio(stopwords, tokens.len());
    let mean_sentence_len = if sentences.is_empty() {
        0.0
    } else {
        tokens.len() as f32 / sentences.len() as f32
    };

    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let table_like = lines.iter().filter(|l| looks_like_a_table_row(l)).count();
    let table_line_ratio = ratio(table_like, lines.len());

    let verdict = |reason: ProseReason| ProseVerdict {
        is_prose: reason == ProseReason::Prose,
        reason,
        mean_sentence_len,
        stopword_ratio,
        table_line_ratio,
        tokens: tokens.len(),
    };

    // Order matters: the cheapest and most certain rejections come first, and the first
    // reason that applies is the one reported. Reporting the *last* failed check would
    // send someone tuning a ratio on a document rejected for its format.
    if source.is_inherently_tabular() {
        return verdict(ProseReason::InherentlyTabular);
    }
    if !english {
        return verdict(ProseReason::NotEnglish);
    }
    if tokens.len() < cfg.prose.min_tokens {
        return verdict(ProseReason::TooFewTokens);
    }
    if mean_sentence_len < cfg.prose.min_mean_sentence_len {
        return verdict(ProseReason::SentencesTooShort);
    }
    if stopword_ratio < cfg.prose.min_stopword_ratio {
        return verdict(ProseReason::TooFewStopwords);
    }
    if table_line_ratio > cfg.prose.max_table_line_ratio {
        return verdict(ProseReason::TooManyTableLines);
    }
    verdict(ProseReason::Prose)
}

/// A line that reads as a record rather than a sentence.
///
/// Two shapes, both common in extracted text: a tab-separated row, and a short line with
/// no terminal punctuation. The second is what a slide bullet, a form field and a table
/// cell all look like once the layout is gone.
fn looks_like_a_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.contains('\t') {
        return true;
    }
    let words = trimmed.split_whitespace().count();
    words <= 6 && !trimmed.ends_with(['.', '!', '?'])
}

fn ratio(part: usize, whole: usize) -> f32 {
    if whole == 0 {
        0.0
    } else {
        part as f32 / whole as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROSE: &str = "The regeneration of a chromatography column is a routine but \
        consequential operation, and the sequence in which it is performed determines how \
        much of the resin capacity survives into the next cycle. Operators are expected to \
        confirm that the buffer has equilibrated before any sample is introduced, because a \
        column that has not equilibrated will bind inconsistently and the resulting peak \
        will be difficult to interpret. Where a run has been interrupted, the whole sequence \
        should be restarted rather than resumed, since a partial regeneration leaves the bed \
        in a state that no later step corrects. The record of each cycle is retained so that \
        a drift in capacity can be seen developing rather than discovered at failure. None of \
        this is unusual, and none of it is difficult, but it is the kind of procedure that is \
        followed carelessly once and then blamed on the equipment for the rest of the quarter.";

    /// Built once. `Resources::default()` parses an eighty-thousand-word list, which in
    /// a debug build costs far more than the code under test.
    fn resources() -> &'static Resources {
        static RESOURCES: std::sync::OnceLock<Resources> = std::sync::OnceLock::new();
        RESOURCES.get_or_init(Resources::default)
    }

    fn assess_default(text: &str, source: SourceKind) -> ProseVerdict {
        assess(text, source, true, &Config::default(), resources())
    }

    #[test]
    fn running_text_is_prose() {
        let v = assess_default(PROSE, SourceKind::Plain);
        assert!(v.is_prose, "{}", v.summary());
        assert_eq!(v.reason, ProseReason::Prose);
    }

    #[test]
    fn a_spreadsheet_is_rejected_on_its_format_rather_than_a_ratio() {
        // A wide enough sheet passes any ratio test. The format already answers this.
        let v = assess_default(PROSE, SourceKind::Spreadsheet);
        assert!(!v.is_prose);
        assert_eq!(v.reason, ProseReason::InherentlyTabular);
    }

    #[test]
    fn a_table_is_not_prose() {
        let table = "Batch\tCell\tYield\nDS-2291\tHEK293T\t91.4\nDS-2292\tHEK293T\t88.1\n"
            .repeat(40);
        let v = assess_default(&table, SourceKind::Plain);
        assert!(!v.is_prose, "{}", v.summary());
    }

    #[test]
    fn non_english_is_rejected_with_its_own_reason() {
        let v = assess(PROSE, SourceKind::Plain, false, &Config::default(), resources());
        assert_eq!(v.reason, ProseReason::NotEnglish);
    }

    #[test]
    fn a_short_document_is_rejected_for_length_not_for_shape() {
        let v = assess_default("Batch DS-2291 was purified today.", SourceKind::Plain);
        assert_eq!(v.reason, ProseReason::TooFewTokens);
    }

    #[test]
    fn the_measurements_are_reported_even_when_the_format_decided() {
        // So a threshold can be moved against real numbers rather than guessed at.
        let v = assess_default(PROSE, SourceKind::Spreadsheet);
        assert!(v.tokens > 0, "measurements were skipped: {v:?}");
        assert!(v.stopword_ratio > 0.0);
    }

    #[test]
    fn the_reason_reads_as_an_explanation() {
        let v = assess_default("Short.", SourceKind::Plain);
        assert!(v.summary().contains("too few tokens"), "{}", v.summary());
    }
}
