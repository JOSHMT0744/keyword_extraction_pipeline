//! Two trivial detectors, scored through the same [`super::score`] machinery as Stage 1,
//! so "does the seven-feature weighted sum earn its complexity" is an explicit measured
//! comparison rather than an assumption. Neither baseline is a strawman built to lose:
//! both are real, commonly reached-for heuristics — a digit-bearing-token regex and a
//! bare wordlist-absence check are exactly what someone would write first, before
//! building anything like Stage 1.

use std::ops::Range;

use keyword_extraction_pipeline::{tokenize, Kind, Resources};

use super::carrier::Doc;
use super::score::Emission;

/// Flag any token containing an ASCII digit. This crate has no `regex` dependency and
/// none is added for a baseline; the check is trivial enough not to need one.
pub fn digit_bearing(doc: &Doc) -> Vec<Emission> {
    tokenize::tokens(&doc.text)
        .into_iter()
        .filter(|t| t.text.chars().any(|c| c.is_ascii_digit()))
        .map(|t| to_emission(t.span))
        .collect()
}

/// Flag any token absent from the general English wordlist and not a stopword — the
/// single strongest feature in Stage 1's own weighting (`absent_from_wordlist`, 0.40),
/// isolated and run alone with no other signal, no acronym handling, no phrase merging.
pub fn absent_from_wordlist(doc: &Doc, res: &Resources) -> Vec<Emission> {
    tokenize::tokens(&doc.text)
        .into_iter()
        .filter(|t| {
            let lower = t.lower();
            !res.is_stopword(&lower) && !res.is_common_word(&lower)
        })
        .map(|t| to_emission(t.span))
        .collect()
}

fn to_emission(span: Range<usize>) -> Emission {
    // Baselines have no per-kind threshold of their own; `Kind::Technical` is a
    // placeholder that plays no role in scoring — `score_doc` only ever compares spans.
    Emission { span, kind: Kind::Technical, score: 1.0 }
}

#[cfg(test)]
mod tests {
    use super::super::{score, tiers::TierResources, Tier};
    use super::*;
    use keyword_extraction_pipeline::Config;

    fn tier_res() -> &'static TierResources {
        static R: std::sync::OnceLock<TierResources> = std::sync::OnceLock::new();
        R.get_or_init(TierResources::build)
    }

    fn pools() -> super::super::carrier::Pools {
        super::super::carrier::Pools {
            identifiers: super::super::data::sources(150),
            distractors: super::super::data::distractors(150),
            acronyms: super::super::carrier::Pools::default_acronyms(),
        }
    }

    /// Score one baseline over a fixed corpus, the same way `score::score_corpus` does
    /// for Stage 1, but with the baseline's own emission function in place of
    /// `score::emissions`.
    fn score_baseline(docs: &[Doc], emit: impl Fn(&Doc) -> Vec<Emission>) -> score::Report {
        let mut report = score::Report::default();
        for doc in docs {
            let ems = emit(doc);
            score::score_doc(doc, &ems, &mut report);
        }
        report
    }

    #[test]
    fn stage_one_emits_strictly_fewer_distractors_than_either_baseline() {
        // Relative, not absolute — needs no measured target, and is safe to gate from
        // day one. This is what makes Stage 1's complexity earn its keep: a baseline
        // that matched or beat it on distractor rejection would mean the seven-feature
        // weighted sum was buying nothing a two-line regex didn't already have.
        let docs = super::super::carrier::generate("baseline-seed-v1", 24, &pools(), tier_res());
        let res = Resources::default();
        let cfg = Config::default();

        let stage1 = score::score_corpus(&docs, &res, cfg.thresholds.identifier, cfg.thresholds.technical);
        let digit_report = score_baseline(&docs, digit_bearing);
        let wordlist_report = score_baseline(&docs, |d| absent_from_wordlist(d, &res));

        let distractor_hits = |r: &score::Report| -> usize {
            r.distractor_schemes.values().map(|s| s.emitted).sum()
        };
        let (s1, d1, w1) = (distractor_hits(&stage1), distractor_hits(&digit_report), distractor_hits(&wordlist_report));
        println!("distractor hits — stage1={s1} digit_regex={d1} wordlist_only={w1}");

        assert!(s1 < d1, "Stage 1 ({s1}) did not beat the digit-bearing baseline ({d1}) on distractors");
        assert!(s1 < w1, "Stage 1 ({s1}) did not beat the wordlist-only baseline ({w1}) on distractors");
    }

    #[test]
    fn neither_baseline_recovers_the_unreachable_tier() {
        // The wordlist-only baseline in particular has no reason to respect the ceiling
        // shape::is_candidate enforces — it has no acronym handling and no digit/common-
        // word carve-out — so this is the check that it doesn't accidentally do better
        // than Stage 1 on the one tier Stage 1 structurally cannot reach.
        let docs = super::super::carrier::generate("baseline-unreachable-seed", 24, &pools(), tier_res());
        let res = Resources::default();

        for (name, report) in [
            ("digit_bearing", score_baseline(&docs, digit_bearing)),
            ("absent_from_wordlist", score_baseline(&docs, |d| absent_from_wordlist(d, &res))),
        ] {
            let unreachable = report.tiers.get(&Tier::Unreachable).cloned().unwrap_or_default();
            println!("{name}: Unreachable {}/{} recalled", unreachable.recalled, unreachable.planted);
            assert_eq!(
                unreachable.recalled, 0,
                "{name} recovered {} Unreachable-tier plant(s), expected none",
                unreachable.recalled
            );
        }
    }
}
