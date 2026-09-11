//! Stage 2 — Schwartz–Hearst abbreviation definitions.
//!
//! Implemented directly from Schwartz & Hearst (2003) rather than pulled from a crate,
//! so behaviour pins to [`crate::PipelineVersion`]: a dependency that quietly improved
//! its matching would silently invalidate every stored keyword set.
//!
//! This stage answers a question Stage 1 structurally cannot. Stage 1 decides what a term is
//! from its orthography, which makes `SOP` indistinguishable from the ordinary word
//! "sop" without the case-run heuristic in [`super::shape::acronym_flags`]. A document
//! that writes `Standard Operating Procedure (SOP)` has *told* us, and evidence beats
//! inference. It also yields the canonical expansion, which nothing else here can.
//!
//! Offsets record the **definition site**, not every occurrence of the term. That is what
//! this stage observed. Where the same term also appears in Stage 1's output, that record
//! carries the full occurrence set — the two are complementary rather than redundant,
//! which is why they are emitted separately rather than merged.

use std::{collections::HashMap, ops::Range};

use crate::{
    config::Config,
    resources::Resources,
    tokenize,
    types::{Keyword, Kind, Origin},
};

/// A definition is categorical evidence rather than another weighted vote: the document
/// stated the term itself. Scoring it on Stage 1's feature scale would be inventing a
/// measurement — there is nothing to measure once the text has said so outright.
const DEFINITION_SCORE: f32 = 1.0;

/// Schwartz–Hearst bounds on a short-form candidate.
const MIN_SHORT: usize = 2;
const MAX_SHORT: usize = 10;
const MAX_SHORT_WORDS: usize = 2;

struct Pair {
    short_form: Range<usize>,
    long_form: Range<usize>,
}

pub fn extract(text: &str, _cfg: &Config, res: &Resources) -> Vec<Keyword> {
    let mut pairs: Vec<Pair> = Vec::new();
    for clause in tokenize::clauses(text) {
        collect_pairs(text, clause, res, &mut pairs);
    }

    // One record per distinct normalised form, unioning the sites that produced it. A
    // term defined twice in one document is one term with two pieces of evidence.
    let mut order: Vec<String> = Vec::new();
    let mut merged: HashMap<String, Keyword> = HashMap::new();

    for pair in &pairs {
        let short = &text[pair.short_form.clone()];
        let long = &text[pair.long_form.clone()];
        let expansion = collapse(long);
        for (surface, span) in
            [(short, pair.short_form.clone()), (long, pair.long_form.clone())]
        {
            let normalised = collapse(&surface.to_lowercase());
            let entry = merged.entry(normalised.clone()).or_insert_with(|| {
                order.push(normalised.clone());
                Keyword {
                    original_keyword: surface.to_string(),
                    normalised: normalised.clone(),
                    kind: Kind::Technical,
                    origin: Origin::Definition,
                    score: DEFINITION_SCORE,
                    rank: 0,
                    frequency: 0,
                    offsets: Vec::new(),
                    expansion: Some(expansion.clone()),
                    features: None,
                }
            });
            if !entry.offsets.contains(&span) {
                entry.offsets.push(span);
                entry.frequency += 1;
            }
        }
    }

    // Keyed traversal of an insertion-ordered list rather than the map, so output does
    // not depend on hash iteration order.
    order.into_iter().filter_map(|k| merged.remove(&k)).collect()
}

/// Find `long form (short form)` and `short form (long form)` within one clause.
///
/// Sentence scope is the point: a parenthetical takes its meaning from the clause it sits
/// in, and a window that ran past a full stop would happily define an acronym out of the
/// previous sentence's words.
fn collect_pairs(text: &str, sentence: Range<usize>, res: &Resources, out: &mut Vec<Pair>) {
    debug_assert!(text.is_char_boundary(sentence.start) && text.is_char_boundary(sentence.end));
    let span = &text[sentence.clone()];
    let mut search = 0usize;

    while let Some(rel_open) = span[search..].find('(') {
        let open = sentence.start + search + rel_open;
        let after_open = open + 1;
        let Some(rel_close) = text[after_open..sentence.end].find(')') else { break };
        let close = after_open + rel_close;
        search = (close + 1) - sentence.start;

        let inner = trimmed(text, after_open..close);
        let before = sentence.start..open;

        if inner.is_empty() {
            continue;
        }

        // `long form (short form)` — overwhelmingly the common case.
        let inner_text = &text[inner.clone()];
        if is_short_form_candidate(inner_text, res) {
            if let Some(long) = long_form_before(text, before.clone(), inner_text) {
                out.push(Pair { short_form: inner, long_form: long });
                continue;
            }
        }

        // `short form (long form)` — the parenthetical carries the expansion instead.
        if let Some(short) = last_word(text, before) {
            let short_text = &text[short.clone()];
            if is_short_form_candidate(short_text, res)
                && inner_text.split_whitespace().count() > MAX_SHORT_WORDS
            {
                if let Some(offset) = best_long_form(short_text, inner_text) {
                    let long = (inner.start + offset)..inner.end;
                    if is_plausible_expansion(short_text, &text[long.clone()]) {
                        out.push(Pair { short_form: short, long_form: long });
                    }
                }
            }
        }
    }
}

/// Search the window of words preceding the parenthesis for the expansion.
///
/// The window is bounded exactly as the paper specifies — `min(|SF| + 5, |SF| * 2)`
/// words. Unbounded search would let a three-letter acronym reach back across a whole
/// clause and assemble an expansion out of unrelated words.
fn long_form_before(text: &str, before: Range<usize>, short: &str) -> Option<Range<usize>> {
    let limit = window_words(short);
    let words: Vec<Range<usize>> = tokenize::tokens(&text[before.clone()])
        .into_iter()
        .map(|t| (before.start + t.span.start)..(before.start + t.span.end))
        .collect();
    if words.is_empty() {
        return None;
    }
    let start = words[words.len().saturating_sub(limit)].start;
    let window = start..before.end;
    let window_text = text[window.clone()].trim_end();
    if window_text.is_empty() {
        return None;
    }

    let offset = best_long_form(short, window_text)?;
    let long = (window.start + offset)..(window.start + window_text.len());
    is_plausible_expansion(short, &text[long.clone()]).then_some(long)
}

/// Schwartz–Hearst's matching rule, right to left.
///
/// Every alphanumeric character of the short form must appear in the long form in order,
/// and the short form's *first* character must land at the start of a word. That last
/// condition is what stops `SOP` matching the tail of "workshops".
///
/// Returns the byte offset into `long` where the matched expansion begins.
fn best_long_form(short: &str, long: &str) -> Option<usize> {
    let s: Vec<char> = short.chars().collect();
    let l: Vec<(usize, char)> = long.char_indices().collect();
    if s.is_empty() || l.is_empty() {
        return None;
    }

    let lower = |c: char| c.to_lowercase().next().unwrap_or(c);
    let mut si = s.len() as isize - 1;
    let mut li = l.len() as isize - 1;

    while si >= 0 {
        let curr = lower(s[si as usize]);
        if !curr.is_alphanumeric() {
            si -= 1;
            continue;
        }
        loop {
            if li < 0 {
                return None;
            }
            let (_, lc) = l[li as usize];
            let matches = lower(lc) == curr;
            // The first character of the short form must begin a word.
            let mid_word =
                si == 0 && li > 0 && l[(li - 1) as usize].1.is_alphanumeric();
            if matches && !mid_word {
                break;
            }
            li -= 1;
        }
        li -= 1;
        si -= 1;
    }

    let first_matched = (li + 1).max(0) as usize;
    let byte = l[first_matched].0;
    // Back up to the start of the word the match landed in.
    Some(long[..byte].rfind(char::is_whitespace).map(|p| p + 1).unwrap_or(0))
}

/// Reject matches that are technically valid but say nothing.
fn is_plausible_expansion(short: &str, long: &str) -> bool {
    let words: Vec<&str> = long.split_whitespace().collect();
    if words.is_empty() || words.len() > window_words(short) {
        return false;
    }
    // `X (X)` is a restatement, not a definition.
    if words.iter().any(|w| w.eq_ignore_ascii_case(short)) {
        return false;
    }
    // An expansion no longer than the abbreviation has expanded nothing.
    long.chars().count() > short.chars().count()
}

/// Collapse internal whitespace runs to single spaces.
///
/// A long form that straddles a line wrap contains a newline. `original_keyword` keeps
/// it, because that field is defined as the form *as it appears* and its offsets resolve back
/// to the canonical text. `normalised` and `expansion` must not: they exist to be matched
/// and displayed, and `high performance liquid\nchromatography` would never match a
/// consumer's query for the same phrase written on one line.
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn window_words(short: &str) -> usize {
    let n = short.chars().count();
    (n + 5).min(n * 2)
}

fn is_short_form_candidate(s: &str, res: &Resources) -> bool {
    let chars = s.chars().count();
    if !(MIN_SHORT..=MAX_SHORT).contains(&chars) {
        return false;
    }
    if s.split_whitespace().count() > MAX_SHORT_WORDS {
        return false;
    }
    if !s.chars().next().is_some_and(char::is_alphanumeric) {
        return false;
    }
    // A parenthetical with no letters is a date, a figure number or a measurement.
    if !s.chars().any(char::is_alphabetic) {
        return false;
    }
    !res.is_stopword(&s.to_lowercase())
}

fn trimmed(text: &str, span: Range<usize>) -> Range<usize> {
    let slice = &text[span.clone()];
    let start = slice.len() - slice.trim_start().len();
    let end = slice.trim_end().len();
    (span.start + start)..(span.start + end)
}

fn last_word(text: &str, span: Range<usize>) -> Option<Range<usize>> {
    let toks = tokenize::tokens(&text[span.clone()]);
    toks.last().map(|t| (span.start + t.span.start)..(span.start + t.span.end))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Built once. `Resources::default()` parses an eighty-thousand-word list, which in
    /// a debug build costs far more than the code under test.
    fn resources() -> &'static Resources {
        static RESOURCES: std::sync::OnceLock<Resources> = std::sync::OnceLock::new();
        RESOURCES.get_or_init(Resources::default)
    }

    fn run(text: &str) -> Vec<Keyword> {
        extract(text, &Config::default(), resources())
    }

    fn originals(text: &str) -> Vec<String> {
        run(text).into_iter().map(|k| k.original_keyword).collect()
    }

    #[test]
    fn a_definition_yields_both_the_abbreviation_and_its_expansion() {
        let out = originals("The batch was released under Standard Operating Procedure (SOP) 114.");
        assert!(out.contains(&"SOP".to_string()), "got {out:?}");
        assert!(
            out.contains(&"Standard Operating Procedure".to_string()),
            "the expansion is the other half of the evidence: {out:?}"
        );
    }

    #[test]
    fn both_halves_carry_the_canonical_expansion() {
        for k in run("We used Standard Operating Procedure (SOP) throughout.") {
            assert_eq!(k.expansion.as_deref(), Some("Standard Operating Procedure"), "{k:?}");
            assert_eq!(k.origin, Origin::Definition);
            assert_eq!(k.kind, Kind::Technical);
        }
    }

    #[test]
    fn a_definition_split_by_a_line_wrap_is_still_found() {
        // Wrapped prose breaks mid-clause. Treating that break as a sentence boundary
        // would lose most real definitions in a PDF.
        let out = originals("The instrument uses high performance liquid\nchromatography (HPLC) daily.");
        assert!(out.contains(&"HPLC".to_string()), "lost to a line wrap: {out:?}");
        // The surface stays faithful to the text, newline and all, so its offsets
        // resolve; the expansion is the canonical one-line form.
        let hplc = run("The instrument uses high performance liquid\nchromatography (HPLC) daily.")
            .into_iter()
            .find(|k| k.original_keyword == "HPLC")
            .expect("HPLC missing");
        assert_eq!(
            hplc.expansion.as_deref(),
            Some("high performance liquid chromatography"),
            "expansion kept a line wrap"
        );
    }

    #[test]
    fn a_wrapped_expansion_normalises_to_one_line_while_its_surface_stays_faithful() {
        let text = "It uses high performance liquid\nchromatography (HPLC) daily.";
        let long = run(text)
            .into_iter()
            .find(|k| k.normalised.contains("chromatography") && k.original_keyword != "HPLC")
            .expect("expansion missing");
        assert_eq!(long.normalised, "high performance liquid chromatography");
        assert!(long.original_keyword.contains('\n'), "surface should mirror the text");
        for span in &long.offsets {
            assert_eq!(collapse(&text[span.clone()].to_lowercase()), long.normalised);
        }
    }

    #[test]
    fn a_table_row_is_not_healed_into_the_row_above_it() {
        // The mirror image: a new capitalised line is a real boundary, and merging
        // across it would invent expansions out of neighbouring cells.
        let out = originals("Sample Operating Pressure\nDS-2291 reviewed it (SOP).");
        assert!(
            !out.iter().any(|s| s.contains("Sample Operating Pressure")),
            "merged across a row boundary: {out:?}"
        );
    }

    #[test]
    fn the_window_does_not_reach_back_across_a_sentence_boundary() {
        // Without sentence scope, `SOP` would happily assemble itself out of
        // "Shipping. Order Processing" from the preceding clause.
        let out = originals("Sales Order Processing was slow. The team reviewed it (SOP).");
        assert!(
            !out.iter().any(|s| s.contains("Sales Order Processing")),
            "the window ran past a full stop: {out:?}"
        );
    }

    #[test]
    fn an_aside_is_not_a_definition() {
        for aside in [
            "The yield is plotted (see Fig. 4) for each run.",
            "The column was replaced (2024-03-11) before the run.",
            "The release was tagged (v2.14.3) that morning.",
        ] {
            assert!(originals(aside).is_empty(), "treated an aside as a definition: {aside}");
        }
    }

    #[test]
    fn a_restatement_is_not_an_expansion() {
        // `X (X)` matches trivially and defines nothing.
        assert!(originals("The report on SOP (SOP) was filed.").is_empty());
    }

    #[test]
    fn the_first_letter_must_begin_a_word() {
        // Otherwise `SOP` would match the tail of "workshops" plus any later o and p.
        let out = originals("Attendees ran workshops on process (SOP) design.");
        assert!(
            !out.iter().any(|s| s.contains("workshops")),
            "matched mid-word: {out:?}"
        );
    }

    #[test]
    fn an_expansion_inside_the_parentheses_is_found_too() {
        let out = originals("The instrument uses HPLC (high performance liquid chromatography).");
        assert!(out.contains(&"HPLC".to_string()), "got {out:?}");
        assert!(
            out.contains(&"high performance liquid chromatography".to_string()),
            "got {out:?}"
        );
    }

    #[test]
    fn every_offset_resolves_to_the_normalised_form() {
        // Not `== surface`: one keyword covers one normalised form, and `normalised`
        // additionally collapses the whitespace a line wrap leaves behind.
        let text = "Released under Standard Operating Procedure (SOP) today.";
        for k in run(text) {
            for span in &k.offsets {
                assert_eq!(collapse(&text[span.clone()].to_lowercase()), k.normalised);
            }
        }
    }

    #[test]
    fn a_term_defined_twice_is_one_record_with_both_sites() {
        let out = run("Standard Operating Procedure (SOP) applies. \
             Standard Operating Procedure (SOP) was revised.");
        let sop = out.iter().find(|k| k.original_keyword == "SOP").expect("SOP missing");
        assert_eq!(sop.frequency, 2, "sites were not merged: {sop:?}");
        assert_eq!(sop.offsets.len(), 2);
    }

    #[test]
    fn output_is_deterministic_including_tie_order() {
        let text = "Standard Operating Procedure (SOP) and Quality Control Unit (QCU) apply.";
        assert_eq!(run(text), run(text));
    }
}
