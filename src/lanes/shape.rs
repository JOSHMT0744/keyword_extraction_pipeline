//! Lane 1 — shape and wordlist. The priority arm.
//!
//! Finds identifiers and technical vocabulary from orthography alone, with no curated
//! list of schemes. That is the point: recall must not be capped by a gazetteer, and the
//! lane has to work on a tenant nobody has onboarded and a scheme nobody has catalogued.
//!
//! Scoring is a transparent weighted sum over a retained feature vector, deliberately not
//! a classifier. There are no labels to train one on, and a learned model would forfeit
//! the reproducibility the whole crate exists to provide. Every component is stored, so
//! retuning is a matter of changing weights rather than re-deriving anything.

use std::{collections::HashMap, ops::Range};

use crate::{
    config::{Config, ShapeWeights},
    resources::Resources,
    tokenize::{self, Token},
    types::{Keyword, Kind, Origin},
};

/// The retained feature vector. Every value is in `[0, 1]`.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShapeFeatures {
    pub internal_caps: f32,
    pub digit_letter_mix: f32,
    pub separator_segments: f32,
    pub unusual_length: f32,
    pub absent_from_wordlist: f32,
    pub short_all_caps: f32,
    pub in_document_frequency: f32,
}

impl ShapeFeatures {
    /// Weighted sum. Weights sum to 1.0 by construction, so the score is in `[0, 1]`.
    pub fn score(&self, w: &ShapeWeights) -> f32 {
        self.internal_caps * w.internal_caps
            + self.digit_letter_mix * w.digit_letter_mix
            + self.separator_segments * w.separator_segments
            + self.unusual_length * w.unusual_length
            + self.absent_from_wordlist * w.absent_from_wordlist
            + self.short_all_caps * w.short_all_caps
            + self.in_document_frequency * w.in_document_frequency
    }
}

/// A candidate before scoring: one distinct normalised form and everywhere it occurs.
struct Candidate {
    surface: String,
    normalised: String,
    offsets: Vec<Range<usize>>,
    /// A short all-caps token sitting among ordinary text, rather than inside a run of
    /// capitals. See [`acronym_flags`].
    acronym: bool,
}

/// Distinguish acronyms from shouting.
///
/// The wordlist is lowercase general English, so a case-blind membership test suppresses
/// `SOP` because "sop" is an ordinary word. Treating every all-caps token as absent
/// instead admits every word of a shouted heading. The discriminator is context: a
/// genuine acronym appears among lowercase prose, while shouting comes in runs. A token
/// is treated as an acronym only when it is short, all-caps, and *not* part of a run of
/// three or more consecutive all-caps tokens.
fn acronym_flags(toks: &[Token<'_>]) -> Vec<bool> {
    const RUN: usize = 3;
    let caps: Vec<bool> = toks.iter().map(|t| is_short_all_caps(t.text) || is_long_all_caps(t.text)).collect();
    let mut out = vec![false; toks.len()];

    let mut i = 0;
    while i < toks.len() {
        if !caps[i] {
            i += 1;
            continue;
        }
        let mut end = i;
        while end < toks.len() && caps[end] {
            end += 1;
        }
        if end - i < RUN {
            for j in i..end {
                out[j] = is_short_all_caps(toks[j].text);
            }
        }
        i = end;
    }
    out
}

pub fn extract(text: &str, cfg: &Config, res: &Resources) -> Vec<Keyword> {
    let toks = tokenize::tokens(text);
    let acronyms = acronym_flags(&toks);
    let mut candidates: Vec<Candidate> = Vec::new();

    collect_unigrams(&toks, &acronyms, res, &mut candidates);
    collect_phrases(text, &toks, res, &mut candidates);

    let mut out: Vec<Keyword> = Vec::new();
    for c in candidates {
        let features = features_for(&c.surface, c.offsets.len(), c.acronym, res);
        let score = features.score(&cfg.shape_weights);
        let kind = classify(&c.surface);

        let threshold = match kind {
            Kind::Identifier => cfg.thresholds.identifier,
            _ => cfg.thresholds.technical,
        };
        if score < threshold {
            continue;
        }

        out.push(Keyword {
            frequency: c.offsets.len() as u32,
            surface: c.surface,
            normalised: c.normalised,
            kind,
            origin: Origin::Shape,
            score,
            rank: 0,
            offsets: c.offsets,
            expansion: None,
            features: cfg.retain_features.then_some(features),
        });
    }

    rank_within_kind(&mut out);
    out
}

/// Group single tokens by normalised form, keeping every occurrence.
fn collect_unigrams(
    toks: &[Token<'_>],
    acronyms: &[bool],
    res: &Resources,
    out: &mut Vec<Candidate>,
) {
    let mut seen: HashMap<String, usize> = HashMap::new();

    for (i, t) in toks.iter().enumerate() {
        let lower = t.lower();
        let acronym = acronyms[i];
        if !is_candidate(t.text, &lower, acronym, res) {
            continue;
        }
        match seen.get(&lower) {
            Some(&idx) => {
                out[idx].offsets.push(t.span.clone());
                // One unshouted occurrence is enough to call it an acronym.
                out[idx].acronym |= acronym;
            }
            None => {
                seen.insert(lower.clone(), out.len());
                out.push(Candidate {
                    surface: t.text.to_string(),
                    normalised: lower,
                    offsets: vec![t.span.clone()],
                    acronym,
                });
            }
        }
    }
}

/// Merge adjacent distinctive tokens into multi-word technical terms.
///
/// `MabSelect SuRe` and `Capto S` are single names split across tokens; emitting only the
/// parts loses the entity. The merge is deliberately narrow — at least one component must
/// carry internal capitalisation or digits — because a looser rule turns every title-case
/// heading into a spurious term. `Column Regeneration Report` is three ordinary words with
/// only sentence-position capitals, and must not merge.
fn collect_phrases(text: &str, toks: &[Token<'_>], res: &Resources, out: &mut Vec<Candidate>) {
    const MAX_LEN: usize = 3;
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut i = 0;

    while i < toks.len() {
        if !is_phrase_head(toks[i].text, res) {
            i += 1;
            continue;
        }

        let mut end = i + 1;
        while end < toks.len()
            && end - i < MAX_LEN
            && is_phrase_tail(toks[end].text, res)
            && same_line(text, &toks[end - 1], &toks[end])
        {
            end += 1;
        }
        if end == i + 1 {
            i += 1;
            continue;
        }

        let span = toks[i].span.start..toks[end - 1].span.end;
        let surface: String = toks[i..end].iter().map(|t| t.text).collect::<Vec<_>>().join(" ");
        let lower = surface.to_lowercase();

        // Require a strong orthographic signal somewhere in the phrase.
        if !toks[i..end].iter().any(|t| has_internal_caps(t.text)) {
            i += 1;
            continue;
        }

        match seen.get(&lower) {
            Some(&idx) => out[idx].offsets.push(span),
            None => {
                seen.insert(lower.clone(), out.len());
                out.push(Candidate { surface, normalised: lower, offsets: vec![span], acronym: false });
            }
        }
        i = end;
    }
}

/// Whether two adjacent tokens are on the same line of canonical text.
///
/// Adjacency in the token vector is not adjacency in the document: spreadsheet rows,
/// slide bullets and table cells all become separate lines. Without this check the merger
/// splices unrelated cells together — an xlsx of batch codes produced the phantom term
/// `DS-2291 HEK293T DS-2292` by running off the end of one row into the next.
fn same_line(text: &str, left: &Token<'_>, right: &Token<'_>) -> bool {
    !text[left.span.end..right.span.start].contains('\n')
}

/// A phrase head must be name-like, not a code.
///
/// Digit-bearing tokens are excluded from merging entirely: a run of identifiers is a
/// list, not a name, and merging them fabricates a term that appears nowhere.
fn is_phrase_head(text: &str, res: &Resources) -> bool {
    !has_digit(text)
        && text.chars().count() >= 2
        && text.chars().next().is_some_and(char::is_alphabetic)
        && (has_internal_caps(text) || !res.is_common_word(&text.to_lowercase()))
        && text.chars().next().is_some_and(char::is_uppercase)
}

/// A continuation may be a single capital letter — `Capto S` and `Nuvia HR-S` both end
/// in one, and requiring two characters would truncate the name.
fn is_phrase_tail(text: &str, res: &Resources) -> bool {
    let lower = text.to_lowercase();
    !has_digit(text)
        && text.chars().next().is_some_and(char::is_uppercase)
        && (has_internal_caps(text) || !res.is_common_word(&lower))
}

/// Whether a token is worth scoring at all.
///
/// Rejects stopwords, pure numerics and single characters. Bare numbers are excluded
/// because a number alone carries no identifying power and a table of measurements would
/// otherwise flood the lane.
fn is_candidate(text: &str, lower: &str, acronym: bool, res: &Resources) -> bool {
    if text.chars().count() < 2 || res.is_stopword(lower) {
        return false;
    }
    if !text.chars().any(char::is_alphabetic) {
        return false;
    }
    if acronym {
        return true;
    }
    // Ordinary English with no orthographic signal is not a Lane 1 candidate. Topical
    // relevance is Lane 3's job, and emitting these here would drown the lane.
    if res.is_common_word(lower)
        && !has_internal_caps(text)
        && !has_digit(text)
        && tokenize::separator_segments(text) == 1
        && !is_short_all_caps(text)
    {
        return false;
    }
    true
}

fn features_for(
    surface: &str,
    occurrences: usize,
    acronym: bool,
    res: &Resources,
) -> ShapeFeatures {
    let lower = surface.to_lowercase();
    let len = surface.chars().count();
    let segments = tokenize::separator_segments(surface);

    ShapeFeatures {
        internal_caps: has_internal_caps(surface) as u8 as f32,
        digit_letter_mix: (has_digit(surface) && surface.chars().any(char::is_alphabetic)) as u8
            as f32,
        separator_segments: match segments {
            0 | 1 => 0.0,
            2 => 0.6,
            _ => 1.0,
        },
        // English word lengths cluster between 2 and 12. Beyond that, length itself is
        // weak evidence of something other than an ordinary word.
        unusual_length: match len {
            0..=12 => 0.0,
            13..=17 => 0.5,
            _ => 1.0,
        },
        // Case-aware: `SOP` is not the word "sop", so a confirmed acronym counts as
        // absent regardless of its lowercase form.
        absent_from_wordlist: (acronym || !res.is_common_word(&lower)) as u8 as f32,
        short_all_caps: is_short_all_caps(surface) as u8 as f32,
        // Repetition is weak evidence of significance and saturates quickly: the third
        // occurrence says much less than the second.
        in_document_frequency: ((occurrences.saturating_sub(1)) as f32 / 3.0).min(1.0),
    }
}

/// Identifier versus technical term.
///
/// Digit-bearing or coded tokens are identifiers — the exact-matchable arm. Everything
/// else that survives scoring is technical vocabulary.
fn classify(surface: &str) -> Kind {
    let has_digit = has_digit(surface);
    if has_digit && surface.chars().any(char::is_alphabetic) {
        return Kind::Identifier;
    }
    if has_digit && tokenize::separator_segments(surface) > 1 {
        return Kind::Identifier;
    }
    Kind::Technical
}

/// Rank descending by score within each kind.
///
/// Never across kinds: shape scores and topical scores are on unrelated scales, and a
/// single ranked list would be a fabricated comparison.
pub fn rank_within_kind(keywords: &mut [Keyword]) {
    use std::collections::HashMap;

    // Ties broken by normalised form so ordering is total and therefore reproducible.
    keywords.sort_by(|a, b| {
        (a.kind as u8)
            .cmp(&(b.kind as u8))
            .then(b.score.total_cmp(&a.score))
            .then(a.normalised.cmp(&b.normalised))
    });

    let mut next: HashMap<u8, u32> = HashMap::new();
    for k in keywords.iter_mut() {
        let n = next.entry(k.kind as u8).or_insert(0);
        k.rank = *n;
        *n += 1;
    }
}

fn has_internal_caps(text: &str) -> bool {
    let mut chars = text.chars();
    let _ = chars.next();
    chars.clone().any(char::is_uppercase) && chars.any(char::is_lowercase)
}

fn has_digit(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_digit())
}

/// Short runs of capitals are acronyms — `SOP`, `HPLC`, `PO`. Longer runs are usually
/// shouting or a heading, not a term.
fn is_long_all_caps(text: &str) -> bool {
    text.chars().count() > 6 && is_all_caps(text)
}

fn is_all_caps(text: &str) -> bool {
    text.chars().all(|c| c.is_uppercase() || !c.is_alphabetic())
        && text.chars().any(char::is_alphabetic)
}

fn is_short_all_caps(text: &str) -> bool {
    let len = text.chars().count();
    (2..=6).contains(&len) && is_all_caps(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> Vec<Keyword> {
        extract(text, &Config::default(), &Resources::default())
    }

    fn surfaces(text: &str, kind: Kind) -> Vec<String> {
        run(text).into_iter().filter(|k| k.kind == kind).map(|k| k.surface).collect()
    }

    const SAMPLE: &str = "Column Regeneration Report. Batch DS-2291 was purified on the \
        MabSelect SuRe column. Cell line HEK293T was cultured according to SOP-114 \
        revision 3. The chromatography step achieved the expected yield.";

    #[test]
    fn finds_coded_identifiers() {
        let ids = surfaces(SAMPLE, Kind::Identifier);
        for want in ["DS-2291", "HEK293T", "SOP-114"] {
            assert!(ids.contains(&want.to_string()), "missing {want} in {ids:?}");
        }
    }

    #[test]
    fn finds_domain_vocabulary_as_technical() {
        let tech = surfaces(SAMPLE, Kind::Technical);
        assert!(tech.contains(&"chromatography".to_string()), "got {tech:?}");
    }

    #[test]
    fn merges_multiword_product_names() {
        let tech = surfaces(SAMPLE, Kind::Technical);
        assert!(
            tech.contains(&"MabSelect SuRe".to_string()),
            "the name was split into its parts: {tech:?}"
        );
    }

    #[test]
    fn does_not_merge_across_a_line_break() {
        // Spreadsheet rows, slide bullets and table cells are separate lines. Token
        // adjacency is not text adjacency, and merging across the gap invents terms.
        let all: Vec<String> = run("Batch Cell\nDS-2291\tHEK293T\nDS-2292\tHEK293T")
            .into_iter().map(|k| k.surface).collect();
        assert!(
            !all.iter().any(|s| s.split_whitespace().count() > 1),
            "merged across rows: {all:?}"
        );
    }

    #[test]
    fn does_not_merge_runs_of_identifiers_into_a_phantom_name() {
        let all: Vec<String> = run("Samples DS-2291 DS-2292 DS-2293 were shipped.")
            .into_iter().map(|k| k.surface).collect();
        assert!(
            !all.iter().any(|s| s.contains(' ')),
            "a list of codes is not a name: {all:?}"
        );
    }

    #[test]
    fn does_not_merge_ordinary_title_case_headings() {
        // "Column Regeneration Report" is three ordinary words carrying only positional
        // capitals. Merging it would fabricate a term that means nothing.
        let all: Vec<String> = run(SAMPLE).into_iter().map(|k| k.surface).collect();
        assert!(
            !all.iter().any(|s| s.contains("Column Regeneration")),
            "title-case heading was merged: {all:?}"
        );
    }

    #[test]
    fn ordinary_english_is_not_emitted() {
        let all: Vec<String> =
            run(SAMPLE).into_iter().map(|k| k.normalised).collect();
        for common in ["column", "report", "revision", "step", "expected", "yield", "the"] {
            assert!(!all.contains(&common.to_string()), "emitted ordinary word {common}: {all:?}");
        }
    }

    #[test]
    fn bare_numbers_are_never_emitted() {
        let all: Vec<String> = run("Yield was 91.4 and 2291 units over 3 runs")
            .into_iter().map(|k| k.surface).collect();
        assert!(all.is_empty(), "bare numerics carry no identifying power: {all:?}");
    }

    #[test]
    fn acronyms_are_kept_but_shouting_is_not() {
        let ids: Vec<String> = run("The SOP was reviewed. THIS IS A VERY LOUD HEADING LINE")
            .into_iter().map(|k| k.surface).collect();
        assert!(ids.contains(&"SOP".to_string()), "got {ids:?}");
        assert!(!ids.contains(&"HEADING".to_string()), "long all-caps is shouting: {ids:?}");
    }

    #[test]
    fn a_word_inside_a_shouted_run_is_not_promoted_to_an_acronym() {
        // The precision cost of the case-aware rule, contained. "LOUD" would otherwise
        // count as absent from the wordlist purely for being capitalised.
        let shouted: Vec<String> = run("PLEASE READ THIS LOUD NOTICE NOW carefully")
            .into_iter().map(|k| k.surface).collect();
        assert!(!shouted.contains(&"LOUD".to_string()), "shouting leaked through: {shouted:?}");
    }

    #[test]
    fn an_acronym_among_lowercase_prose_survives_a_wordlist_collision() {
        // "sop" is an ordinary English word; "SOP" in running text is not.
        let out: Vec<String> = run("The batch was released under SOP control by the team.")
            .into_iter().map(|k| k.surface).collect();
        assert!(out.contains(&"SOP".to_string()), "got {out:?}");
    }

    #[test]
    fn repeated_occurrences_are_all_recorded() {
        let out = run("Batch DS-2291 shipped. Batch DS-2291 was later recalled.");
        let id = out.iter().find(|k| k.surface == "DS-2291").expect("identifier missing");
        assert_eq!(id.frequency, 2);
        assert_eq!(id.offsets.len(), 2);
    }

    #[test]
    fn every_offset_resolves_to_the_normalised_form() {
        // Deliberately not `== surface`. One keyword covers one normalised form, so its
        // offsets may point at differently-cased variants; `surface` is only the first
        // one seen. Asserting against `surface` passes only while no fixture varies
        // case, and would break the day one did.
        let text = "Batch DS-2291 was purified on the MabSelect SuRe column.";
        for k in run(text) {
            for span in &k.offsets {
                assert_eq!(text[span.clone()].to_lowercase(), k.normalised, "offset does not resolve");
            }
        }
    }

    #[test]
    fn case_variants_are_one_finding_rather_than_several() {
        let text = "Chromatography was used. The chromatography step ran with chromatography.";
        let out = run(text);
        let matches: Vec<&Keyword> =
            out.iter().filter(|k| k.normalised == "chromatography").collect();
        assert_eq!(matches.len(), 1, "case split one finding into several: {out:?}");
        assert_eq!(matches[0].frequency, 3, "occurrences were lost: {:?}", matches[0]);
        assert_eq!(matches[0].offsets.len(), 3);
    }

    #[test]
    fn ranks_are_dense_and_scoped_to_each_kind() {
        let out = run(SAMPLE);
        for kind in [Kind::Identifier, Kind::Technical] {
            let mut ranks: Vec<u32> =
                out.iter().filter(|k| k.kind == kind).map(|k| k.rank).collect();
            ranks.sort_unstable();
            assert_eq!(ranks, (0..ranks.len() as u32).collect::<Vec<_>>(), "{kind:?}");
        }
    }

    #[test]
    fn output_is_deterministic_including_tie_order() {
        assert_eq!(run(SAMPLE), run(SAMPLE));
    }

    #[test]
    fn features_are_retained_and_sum_within_range() {
        let res = Resources::default();
        for t in ["DS-2291", "chromatography", "SOP", "MabSelect"] {
            let s = features_for(t, 1, false, &res).score(&ShapeWeights::default());
            assert!((0.0..=1.0).contains(&s), "{t} scored {s}");
        }
    }
}
