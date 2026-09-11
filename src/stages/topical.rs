//! Stage 3 — YAKE topical keyphrase extraction.
//!
//! Implemented directly from Campos et al., *YAKE! Keyword extraction from single
//! documents using multiple local features* (Information Sciences, 2020), for the same
//! reason as Stage 2: behaviour has to pin to [`crate::PipelineVersion`], and a dependency
//! that quietly improved its scoring would invalidate every stored keyword set without
//! changing the stamp.
//!
//! YAKE is corpus-blind by construction — every feature is computed from the single
//! document — which is exactly the constraint this crate works under. No IDF, no
//! background corpus, no training.
//!
//! **Scores are lower-is-better.** This is the one place in the crate where that is true,
//! and it is why [`crate::config::Thresholds::topical`] is an upper bound rather than a
//! floor. Ranking still runs descending by score across the union, so topical keywords
//! are re-signed at emission: see `to_keyword_score`.

use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

use crate::{
    config::Config,
    resources::Resources,
    tokenize::{self, Token},
    types::{Keyword, Kind, Origin},
};

/// Window for the relatedness feature. The paper's value.
const CONTEXT_WINDOW: usize = 1;

/// Candidates whose surfaces are this similar are treated as the same phrase, keeping the
/// better-scoring one. The paper deduplicates with a string-similarity threshold; this is
/// its Levenshtein form.
const DEDUPE_SIMILARITY: f32 = 0.8;

/// A term's per-document statistics, gathered in one pass.
#[derive(Default, Clone)]
struct TermStats {
    frequency: usize,
    /// Occurrences beginning with a capital that are not sentence-initial.
    uppercase: usize,
    /// Occurrences that are entirely uppercase and longer than one character.
    acronym: usize,
    /// Indices of the sentences the term appears in.
    sentences: HashSet<usize>,
    /// Median-ish position: the sentence indices are kept and reduced later.
    first_positions: Vec<usize>,
    left: HashSet<String>,
    right: HashSet<String>,
}

pub fn extract(text: &str, cfg: &Config, res: &Resources) -> Vec<Keyword> {
    // Clauses rather than raw lines: `t_position` and `t_sentence` are both statements
    // about where a term sits in the document's *prose*, and canonical text carries one
    // newline per rendered line. Measured over lines, every wrapped PDF line counts as
    // its own sentence and dispersion becomes a measure of column width.
    //
    // Adjacency is deliberately left alone — `is_phrase` still refuses to cross a raw
    // newline, so joining clauses here cannot splice one table cell onto the next.
    let sentences = tokenize::clauses(text);
    if sentences.is_empty() {
        return Vec::new();
    }

    let (stats, sentence_tokens) = gather(text, &sentences, res);
    if stats.is_empty() {
        return Vec::new();
    }

    let term_scores = score_terms(&stats, &sentences);
    let candidates = candidates(text, &sentence_tokens, cfg, res);

    let mut scored: Vec<(f32, Candidate)> = candidates
        .into_iter()
        .filter_map(|c| score_candidate(&c, &term_scores).map(|s| (s, c)))
        .filter(|(s, _)| *s <= cfg.thresholds.topical)
        .collect();

    // Ascending: YAKE scores are lower-is-better. Ties resolved on the surface so the
    // ordering is total and reproducible.
    scored.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.normalised.cmp(&b.1.normalised)));
    deduplicate(&mut scored);

    scored
        .into_iter()
        .map(|(score, c)| Keyword {
            frequency: c.offsets.len() as u32,
            original_keyword: c.surface,
            normalised: c.normalised,
            kind: Kind::Topical,
            origin: Origin::Statistic,
            score: to_keyword_score(score),
            rank: 0,
            offsets: c.offsets,
            expansion: None,
            features: None,
        })
        .collect()
}

/// Re-sign a YAKE score for the shared ranking function.
///
/// [`super::shape::rank_within_kind`] sorts descending, because for every other stage a
/// higher score is a stronger claim. YAKE's is the reverse. Rather than special-casing
/// the sort — which would be a trap for anyone adding a fourth stage — the score is mapped
/// to `1 / (1 + s)`, which is monotonically decreasing in `s`, lands in `(0, 1]` like
/// every other score in the crate, and preserves the ordering exactly.
fn to_keyword_score(yake: f32) -> f32 {
    1.0 / (1.0 + yake.max(0.0))
}

/// One pass over the document collecting every statistic the five features need.
fn gather<'a>(
    text: &'a str,
    sentences: &[Range<usize>],
    res: &Resources,
) -> (HashMap<String, TermStats>, Vec<Vec<Token<'a>>>) {
    let mut stats: HashMap<String, TermStats> = HashMap::new();
    let mut per_sentence: Vec<Vec<Token<'a>>> = Vec::with_capacity(sentences.len());

    for (index, span) in sentences.iter().enumerate() {
        let toks: Vec<Token<'a>> = tokenize::tokens(&text[span.clone()])
            .into_iter()
            .map(|t| Token {
                text: t.text,
                span: (span.start + t.span.start)..(span.start + t.span.end),
            })
            .collect();

        for (i, tok) in toks.iter().enumerate() {
            let lower = tok.lower();
            if !is_countable(tok.text, &lower, res) {
                continue;
            }
            let entry = stats.entry(lower).or_default();
            entry.frequency += 1;
            entry.sentences.insert(index);
            entry.first_positions.push(index);

            let first_char = tok.text.chars().next();
            if i > 0 && first_char.is_some_and(char::is_uppercase) {
                entry.uppercase += 1;
            }
            if tok.text.chars().count() > 1 && tok.text.chars().all(|c| !c.is_lowercase())
                && tok.text.chars().any(char::is_alphabetic)
            {
                entry.acronym += 1;
            }

            // Relatedness looks at distinct neighbours: a term surrounded by many
            // different words behaves like a stopword, whatever the stopword list says.
            for offset in 1..=CONTEXT_WINDOW {
                if let Some(left) = i.checked_sub(offset).and_then(|j| toks.get(j)) {
                    entry.left.insert(left.lower());
                }
                if let Some(right) = toks.get(i + offset) {
                    entry.right.insert(right.lower());
                }
            }
        }
        per_sentence.push(toks);
    }
    (stats, per_sentence)
}

/// The five term features, combined as the paper specifies.
fn score_terms(
    stats: &HashMap<String, TermStats>,
    sentences: &[Range<usize>],
) -> HashMap<String, f32> {
    let frequencies: Vec<f32> = stats.values().map(|s| s.frequency as f32).collect();
    let mean = frequencies.iter().sum::<f32>() / frequencies.len().max(1) as f32;
    let variance = frequencies.iter().map(|f| (f - mean).powi(2)).sum::<f32>()
        / frequencies.len().max(1) as f32;
    let deviation = variance.sqrt();
    let max_frequency = frequencies.iter().cloned().fold(1.0_f32, f32::max);
    let total_sentences = sentences.len().max(1) as f32;

    stats
        .iter()
        .map(|(term, s)| {
            let frequency = s.frequency as f32;

            // Casing: a term written in caps or capitalised mid-sentence is being
            // marked as significant by its author.
            let casing = s.uppercase.max(s.acronym) as f32;
            let t_case = casing / (1.0 + frequency.ln());

            // Position: terms introduced early carry more of a document's subject.
            let median = median(&s.first_positions);
            let t_position = (3.0 + median).ln().ln();

            // Frequency, normalised against the document's own spread rather than a
            // corpus — the corpus-blind substitute for IDF.
            let tf_norm = frequency / (mean + deviation).max(1.0);

            // Relatedness: many distinct neighbours means the term behaves like a
            // function word regardless of whether it is on the stopword list.
            let left = s.left.len() as f32 / frequency.max(1.0);
            let right = s.right.len() as f32 / frequency.max(1.0);
            let t_rel = 1.0 + (left + right) * (frequency / max_frequency);

            // Dispersion: a term spread across the document is topical; one confined to
            // a single sentence is incidental.
            let t_sentence = s.sentences.len() as f32 / total_sentences;

            let score = (t_rel * t_position)
                / (t_case + (tf_norm / t_rel) + (t_sentence / t_rel));
            (term.clone(), if score.is_finite() { score.max(0.0) } else { f32::MAX })
        })
        .collect()
}

struct Candidate {
    surface: String,
    normalised: String,
    terms: Vec<String>,
    offsets: Vec<Range<usize>>,
}

/// Contiguous n-grams up to `yake_ngram_max`, never crossing a sentence, a line, a
/// stopword or punctuation.
///
/// Stopwords bound a phrase rather than joining it: `regeneration of the column` is two
/// candidates, not one four-word phrase. Line breaks bound it for the same reason Stage 1
/// refuses to merge across them — token adjacency in the vector is not adjacency on the
/// page. Punctuation bounds it because a comma is a clause boundary: `resumed, since`
/// is two clauses touching, not a phrase, and joining them also makes the candidate's
/// offsets cover text the candidate does not contain.
fn candidates(
    text: &str,
    sentence_tokens: &[Vec<Token<'_>>],
    cfg: &Config,
    res: &Resources,
) -> Vec<Candidate> {
    let mut by_form: HashMap<String, Candidate> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let max = cfg.yake_ngram_max.max(1);

    for toks in sentence_tokens {
        for start in 0..toks.len() {
            for len in 1..=max {
                let Some(window) = toks.get(start..start + len) else { break };
                if !is_phrase(window, text, res) {
                    break;
                }
                let surface = window
                    .iter()
                    .map(|t| t.text)
                    .collect::<Vec<_>>()
                    .join(" ");
                let normalised = surface.to_lowercase();
                let span = window[0].span.start..window[len - 1].span.end;
                debug_assert_eq!(
                    text[span.clone()].to_lowercase(),
                    normalised,
                    "candidate offsets must cover exactly the candidate"
                );

                let entry = by_form.entry(normalised.clone()).or_insert_with(|| {
                    order.push(normalised.clone());
                    Candidate {
                        surface,
                        normalised: normalised.clone(),
                        terms: window.iter().map(|t| t.lower()).collect(),
                        offsets: Vec::new(),
                    }
                });
                entry.offsets.push(span);
            }
        }
    }

    order.into_iter().filter_map(|k| by_form.remove(&k)).collect()
}

fn is_phrase(window: &[Token<'_>], text: &str, res: &Resources) -> bool {
    for (i, tok) in window.iter().enumerate() {
        let lower = tok.lower();
        if !is_countable(tok.text, &lower, res) {
            return false;
        }
        if i > 0 {
            // Only whitespace may separate the words of one phrase. Anything else — a
            // comma, a bracket, a line break — is a boundary, and spanning it would put
            // characters inside the candidate's offsets that are not in the candidate.
            let gap = &text[window[i - 1].span.end..tok.span.start];
            if gap.is_empty() || !gap.chars().all(|c| c == ' ' || c == '\t') {
                return false;
            }
        }
    }
    true
}

/// `S(kw) = Π S(w) / (TF(kw) · (1 + Σ S(w)))`, the paper's combination.
fn score_candidate(c: &Candidate, terms: &HashMap<String, f32>) -> Option<f32> {
    let mut product = 1.0_f32;
    let mut sum = 0.0_f32;
    for term in &c.terms {
        let s = *terms.get(term)?;
        product *= s;
        sum += s;
    }
    let tf = c.offsets.len() as f32;
    let score = product / (tf * (1.0 + sum));
    score.is_finite().then_some(score.max(0.0))
}

/// Drop near-duplicate surfaces, keeping the better-scoring one.
///
/// Without this the output is `column regeneration`, `column regenerations` and
/// `Column Regeneration` as three separate findings, which is one finding reported three
/// times. Input must already be sorted best-first.
fn deduplicate(scored: &mut Vec<(f32, Candidate)>) {
    let mut kept: Vec<String> = Vec::new();
    scored.retain(|(_, c)| {
        if kept.iter().any(|k| similarity(k, &c.normalised) >= DEDUPE_SIMILARITY) {
            return false;
        }
        kept.push(c.normalised.clone());
        true
    });
}

fn similarity(a: &str, b: &str) -> f32 {
    let longest = a.chars().count().max(b.chars().count());
    if longest == 0 {
        return 1.0;
    }
    1.0 - (levenshtein(a, b) as f32 / longest as f32)
}

/// Two-row Levenshtein. Inline rather than a dependency: it is fifteen lines, and a
/// crate that changed its tie-breaking would move `pipeline_version` for no reason.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];

    for (i, ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            current[j + 1] = (previous[j] + cost)
                .min(previous[j + 1] + 1)
                .min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

/// Whether a token participates in topical scoring at all.
///
/// Stopwords, punctuation and bare numerals are excluded: they are the vocabulary of
/// every document and so distinguish none of them.
fn is_countable(surface: &str, lower: &str, res: &Resources) -> bool {
    if surface.chars().count() < 2 || res.is_stopword(lower) {
        return false;
    }
    surface.chars().any(char::is_alphabetic)
}

fn median(values: &[usize]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) as f32 / 2.0
    } else {
        sorted[mid] as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Long enough for the statistics to mean something, and about one thing.
    const DOC: &str = "Column regeneration and resin lifetime.\n\
        The regeneration of a chromatography column is a routine but consequential \
        operation, and the sequence in which it is performed determines how much of the \
        resin capacity survives into the next cycle. Operators are expected to confirm \
        that the buffer has equilibrated before any sample is introduced, because a \
        column that has not equilibrated will bind inconsistently.\n\
        Where a run has been interrupted, the whole regeneration sequence should be \
        restarted rather than resumed, since a partial regeneration leaves the resin bed \
        in a state that no later step corrects. Resin lifetime is reported in cycles \
        rather than in months, because a column used heavily for six weeks has aged more \
        than one used occasionally for a year.";

    /// Built once. `Resources::default()` parses an eighty-thousand-word list, which in
    /// a debug build costs far more than the stage under test.
    fn resources() -> &'static Resources {
        static RESOURCES: std::sync::OnceLock<Resources> = std::sync::OnceLock::new();
        RESOURCES.get_or_init(Resources::default)
    }

    fn run(text: &str) -> Vec<Keyword> {
        // The default topical threshold is an unvalidated guess pending the keyphrase
        // instrument; these tests are about ordering and shape, so they open it up.
        let cfg = Config {
            thresholds: crate::config::Thresholds { topical: 100.0, ..Default::default() },
            ..Config::default()
        };
        extract(text, &cfg, resources())
    }

    fn originals(text: &str) -> Vec<String> {
        run(text).into_iter().map(|k| k.normalised).collect()
    }

    #[test]
    fn the_documents_subject_ranks_above_its_incidental_vocabulary() {
        let out = originals(DOC);
        let rank = |s: &str| out.iter().position(|k| k == s);
        let subject = rank("regeneration").or(rank("column regeneration")).expect("no subject");
        let incidental = rank("months").or(rank("weeks")).unwrap_or(usize::MAX);
        assert!(subject < incidental, "subject ranked below incidental vocabulary: {out:?}");
    }

    #[test]
    fn stopwords_are_never_emitted_and_never_join_a_phrase() {
        for s in originals(DOC) {
            for word in s.split_whitespace() {
                assert!(
                    !resources().is_stopword(word),
                    "stopword {word} inside candidate {s:?}"
                );
            }
        }
    }

    #[test]
    fn phrases_do_not_cross_punctuation() {
        // `resumed, since` is two clauses touching, not a phrase — and a candidate whose
        // span covers a comma it does not contain has broken offsets as well.
        let out = originals(DOC);
        assert!(
            !out.iter().any(|s| s == "resumed since"),
            "built a phrase across a comma: {out:?}"
        );
    }

    #[test]
    fn phrases_do_not_cross_a_line_break() {
        // The same rule as Stage 1: token adjacency is not adjacency on the page.
        let out = originals("Column regeneration\nResin lifetime report follows here now.");
        assert!(
            !out.iter().any(|s| s.contains("regeneration resin")),
            "merged across a line break: {out:?}"
        );
    }

    #[test]
    fn near_duplicate_phrasings_are_reported_once() {
        // `resin` and `Resin` are one finding, not two. Distinct phrases that merely
        // share a word — `regeneration` and `regeneration sequence` — are not duplicates
        // and must both survive.
        let out = originals(DOC);
        let mut seen = std::collections::HashSet::new();
        for s in &out {
            assert!(seen.insert(s.clone()), "the same finding reported twice: {s:?}");
        }
        for a in &out {
            for b in &out {
                if a != b {
                    assert!(
                        similarity(a, b) < DEDUPE_SIMILARITY,
                        "near-duplicates both survived: {a:?} and {b:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn scores_are_re_signed_so_a_better_keyword_scores_higher() {
        // YAKE is lower-is-better; every other stage is higher-is-better, and one shared
        // ranking function sorts them all.
        let out = run(DOC);
        for pair in out.windows(2) {
            assert!(pair[0].score >= pair[1].score, "not descending: {out:?}");
        }
        for k in &out {
            assert!((0.0..=1.0).contains(&k.score), "score out of range: {k:?}");
        }
    }

    #[test]
    fn every_offset_resolves_to_the_normalised_form() {
        for k in run(DOC) {
            for span in &k.offsets {
                assert_eq!(DOC[span.clone()].to_lowercase(), k.normalised, "offset does not resolve");
            }
        }
    }

    #[test]
    fn ngrams_are_bounded_by_the_configured_maximum() {
        for k in run(DOC) {
            assert!(
                k.original_keyword.split_whitespace().count() <= Config::default().yake_ngram_max,
                "phrase too long: {k:?}"
            );
        }
    }

    #[test]
    fn output_is_deterministic_including_tie_order() {
        assert_eq!(run(DOC), run(DOC));
    }

    #[test]
    fn a_document_with_nothing_to_say_yields_nothing_rather_than_panicking() {
        for degenerate in ["", "   ", "a a a a a", "the the the"] {
            let _ = run(degenerate);
        }
    }

    #[test]
    fn levenshtein_agrees_with_worked_examples() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("same", "same"), 0);
    }
}
