//! Scores Stage 1's output against a generated corpus's known plants.
//!
//! Two design choices carried through from the plan this instrument was built against:
//!
//! **Matching is span-exact, not string-equal.** A plant is recalled only when some
//! emission's span equals it (or, for a multi-word plant, is equal to or contained by it —
//! `MabSelect SuRe` may legitimately surface as just `MabSelect`). String matching would
//! credit a plant because its surface happened to occur elsewhere in the carrier's prose,
//! and would silently credit a distractor whose surface collided with a real plant. The
//! `matched_by_string_but_not_span` counter stays at the ready in [`Report`]; if it is ever
//! non-zero on a real run, that is a bug in this harness, not a Stage 1 finding.
//!
//! **Kind is reported, never gated on.** `shape::classify` is a pure orthographic function
//! of the surface — `Superdex` has no digits, so it can only ever come out `Technical`, and
//! penalising that as a miss would be measuring a two-line function instead of the
//! thresholds this instrument exists to tune. A plant counts as recalled under either kind,
//! with the split kept in [`TierStat`] so a classification drift is still visible.

use std::{collections::BTreeMap, ops::Range};

use keyword_extraction_pipeline::{stages::shape, Config, Keyword, Kind, Resources};

use super::carrier::Doc;
use super::tiers::Tier;

/// One emission from Stage 1, thresholds already applied.
#[derive(Debug, Clone)]
pub struct Emission {
    pub span: Range<usize>,
    pub kind: Kind,
    pub score: f32,
}

/// Run Stage 1 with both thresholds open (0.0), returning the full scored candidate set.
/// Threshold filtering is applied afterwards by [`emissions`] — free, since it only
/// touches values already computed. Only `wordlist_size` / `acronym_wordlist_depth`
/// changes require a fresh `Resources` and a fresh call here.
pub fn candidates(doc: &Doc, res: &Resources) -> Vec<Keyword> {
    let cfg = Config {
        thresholds: keyword_extraction_pipeline::config::Thresholds {
            identifier: 0.0,
            technical: 0.0,
            ..Config::default().thresholds
        },
        retain_features: true,
        ..Config::default()
    };
    shape::extract(&doc.text, &cfg, res)
}

/// Apply per-kind thresholds to a candidate set.
pub fn emissions(cands: &[Keyword], t_id: f32, t_tech: f32) -> Vec<Emission> {
    cands
        .iter()
        .filter(|k| k.score >= if k.kind == Kind::Identifier { t_id } else { t_tech })
        .flat_map(|k| k.offsets.iter().map(move |span| Emission { span: span.clone(), kind: k.kind, score: k.score }))
        .collect()
}

#[derive(Debug, Clone, Default)]
pub struct TierStat {
    pub planted: usize,
    pub recalled: usize,
    pub as_identifier: usize,
    pub as_technical: usize,
}

#[derive(Debug, Clone, Default)]
pub struct SchemeStat {
    pub planted: usize,
    pub emitted: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub tiers: BTreeMap<Tier, TierStat>,
    pub distractor_schemes: BTreeMap<String, SchemeStat>,
    /// Emissions that land at a span no plant or distractor occupies — the cost side of
    /// the ledger. See the module doc on `carrier.rs`: without this, a sweep that only
    /// maximised recall would recommend shrinking the wordlist to nothing.
    pub prose_false_emissions: usize,
    pub prose_tokens_scanned: usize,
    pub planted_identifiers_found: usize,
    pub unplanted_identifiers_found: usize,
    /// Diagnostic only. Non-zero here means a bug in this harness's span bookkeeping, not
    /// a finding about Stage 1.
    pub matched_by_string_but_not_span: usize,
}

impl Report {
    pub fn prose_false_per_1k(&self) -> f32 {
        if self.prose_tokens_scanned == 0 {
            0.0
        } else {
            1000.0 * self.prose_false_emissions as f32 / self.prose_tokens_scanned as f32
        }
    }

    pub fn unplanted_ids_per_1k(&self) -> f32 {
        if self.prose_tokens_scanned == 0 {
            0.0
        } else {
            1000.0 * self.unplanted_identifiers_found as f32 / self.prose_tokens_scanned as f32
        }
    }
}

fn span_covers(outer: &Range<usize>, inner: &Range<usize>) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

/// Score one document's emissions against its own known plants.
pub fn score_doc(doc: &Doc, emissions: &[Emission], report: &mut Report) {
    let tokens = keyword_extraction_pipeline::tokenize::tokens(&doc.text);
    report.prose_tokens_scanned += tokens.len();

    // Which spans are "claimed" by a plant or distractor, so anything else is prose.
    let mut claimed: Vec<Range<usize>> = Vec::new();

    for plant in &doc.plants {
        claimed.push(plant.span.clone());

        if plant.is_distractor {
            let stat = report.distractor_schemes.entry(plant.scheme.clone()).or_default();
            stat.planted += 1;
            let hit = emissions.iter().any(|e| e.span == plant.span || span_covers(&e.span, &plant.span));
            if hit {
                stat.emitted += 1;
            }
            continue;
        }

        let stat = report.tiers.entry(plant.tier).or_default();
        stat.planted += 1;

        let exact = emissions.iter().find(|e| e.span == plant.span || span_covers(&e.span, &plant.span));
        if let Some(e) = exact {
            stat.recalled += 1;
            match e.kind {
                Kind::Identifier => stat.as_identifier += 1,
                Kind::Technical => stat.as_technical += 1,
                Kind::Topical => {}
            }
            report.planted_identifiers_found += 1;
        } else {
            // Diagnostic: did the surface appear anywhere else, unmatched by span? If so
            // and nothing above caught it, the counter below should have been incremented
            // by the false-emission pass instead — this check exists purely so that gap
            // can never open silently.
            let string_hit = emissions.iter().any(|e| doc.text.get(e.span.clone()) == Some(plant.surface.as_str()));
            if string_hit {
                report.matched_by_string_but_not_span += 1;
            }
        }
    }

    for e in emissions {
        let is_claimed = claimed.iter().any(|c| span_covers(c, &e.span) || span_covers(&e.span, c));
        if !is_claimed {
            report.prose_false_emissions += 1;
            if e.kind == Kind::Identifier {
                report.unplanted_identifiers_found += 1;
            }
        }
    }
}

pub fn score_corpus(docs: &[Doc], res: &Resources, t_id: f32, t_tech: f32) -> Report {
    let mut report = Report::default();
    for doc in docs {
        let cands = candidates(doc, res);
        let ems = emissions(&cands, t_id, t_tech);
        score_doc(doc, &ems, &mut report);
    }
    report
}

#[cfg(test)]
mod tests {
    use super::super::{carrier, data, tiers::TierResources};
    use super::*;

    fn tier_res() -> &'static TierResources {
        static R: std::sync::OnceLock<TierResources> = std::sync::OnceLock::new();
        R.get_or_init(TierResources::build)
    }

    fn pools() -> carrier::Pools {
        carrier::Pools {
            identifiers: data::sources(150),
            distractors: data::distractors(150),
            acronyms: carrier::Pools::default_acronyms(),
        }
    }

    #[test]
    fn easy_tier_plants_are_recalled_at_default_thresholds() {
        let docs = carrier::generate("score-easy-seed", 12, &pools(), tier_res());
        let res = Resources::default();
        let report = score_corpus(&docs, &res, Config::default().thresholds.identifier, Config::default().thresholds.technical);

        let easy = report.tiers.get(&Tier::Easy).cloned().unwrap_or_default();
        assert!(easy.planted > 0, "no Easy-tier plants were generated");
        assert_eq!(
            easy.recalled, easy.planted,
            "Easy-tier recall was not total: {}/{} recalled",
            easy.recalled, easy.planted
        );
    }

    #[test]
    fn the_string_only_match_diagnostic_stays_at_zero() {
        let docs = carrier::generate("score-diag-seed", 12, &pools(), tier_res());
        let res = Resources::default();
        let report = score_corpus(&docs, &res, Config::default().thresholds.identifier, Config::default().thresholds.technical);
        assert_eq!(
            report.matched_by_string_but_not_span, 0,
            "a plant matched by string but not by span — a bug in this harness's bookkeeping"
        );
    }

    #[test]
    fn prose_false_emission_rate_is_computed_and_printed() {
        let docs = carrier::generate("score-prose-seed", 12, &pools(), tier_res());
        let res = Resources::default();
        let report = score_corpus(&docs, &res, Config::default().thresholds.identifier, Config::default().thresholds.technical);
        println!(
            "prose_false_per_1k = {:.3} ({} false emissions over {} tokens)",
            report.prose_false_per_1k(),
            report.prose_false_emissions,
            report.prose_tokens_scanned
        );
        // Wide collapse-detector only, in the tests/cross_format.rs idiom — nobody has
        // measured a target number for this yet, which is the point of this instrument.
        assert!(report.prose_false_per_1k() < 200.0, "prose false-emission rate looks collapsed: {report:?}");
    }
}
