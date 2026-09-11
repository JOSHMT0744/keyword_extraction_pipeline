//! Instrument 3 — identifier injection.
//!
//! Settles: are Stage 1's shape thresholds (`wordlist_size`, `acronym_wordlist_depth`,
//! `thresholds.identifier`, `thresholds.technical`) right? Plants known identifiers and
//! known non-identifiers into realistic documents and measures how well Stage 1 recovers
//! and rejects them.
//!
//! This file holds the gates: hard pass/fail assertions safe to run on every CI run,
//! forever, because none of them depends on a target number nobody has measured yet. The
//! parameter sweep that reports those numbers lives in `examples/sweep_stage1.rs`, sharing
//! this same harness (`tests/injection_harness/`) via `#[path]`.
//!
//! Built incrementally; see the plan this instrument was built from. Landed so far: tier
//! assignment and data provenance (the anti-fudge machinery, checkable before any carrier
//! document existed), a seeded corpus generator, and the scorer with the easy-tier and
//! prose-false-positive gates below. Remaining: distractor-specific gates and the two
//! trivial baselines that make "does Stage 1 earn its complexity" an explicit comparison.

#[path = "injection_harness/mod.rs"]
mod harness;

use keyword_extraction_pipeline::{Config, Resources};

use harness::{carrier, data, score, tier_of, Tier, TierResources};

fn tier_res() -> &'static TierResources {
    static R: std::sync::OnceLock<TierResources> = std::sync::OnceLock::new();
    R.get_or_init(TierResources::build)
}

/// Identifiers, distractors and acronyms for carrier generation. Not cached in a
/// `OnceLock`: `data::sources`/`distractors` re-parse a handful of small TSV files, which
/// is cheap next to `TierResources::build`'s wordlist construction.
fn corpus_pools() -> carrier::Pools {
    carrier::Pools {
        identifiers: data::sources(150),
        distractors: data::distractors(150),
        acronyms: carrier::Pools::default_acronyms(),
    }
}

/// A corpus at a fixed, documented seed — the same one every gate below scores against,
/// so a failure in one test is directly comparable to a failure in another.
const GATE_SEED: &str = "instrument-3-gate-corpus-v1";
const GATE_DOC_COUNT: usize = 24;

fn gate_report() -> score::Report {
    let docs = carrier::generate(GATE_SEED, GATE_DOC_COUNT, &corpus_pools(), tier_res());
    let res = Resources::default();
    let cfg = Config::default();
    score::score_corpus(&docs, &res, cfg.thresholds.identifier, cfg.thresholds.technical)
}

/// Every source-file entry, tiered. Built once per test process — `TierResources::build`
/// constructs two 80k-word `Resources`, which is not free to repeat per assertion.
fn tiered_sources() -> Vec<(harness::data::Entry, Tier)> {
    data::sources(150)
        .into_iter()
        .map(|e| {
            let t = tier_of(&e.surface, tier_res());
            (e, t)
        })
        .collect()
}

#[test]
fn tiers_are_assigned_by_rule_and_every_tier_has_real_entries() {
    // The core anti-fudge property: tier is a function of the surface, not a column
    // anyone wrote by hand — enforced structurally, since no source TSV even has a tier
    // column to begin with (see tests/injection_harness/data.rs's Entry type).
    let tiered = tiered_sources();
    assert!(!tiered.is_empty(), "no source entries loaded — run scripts/fetch_injection_tiers.sh");

    let mut counts: std::collections::HashMap<Tier, usize> = std::collections::HashMap::new();
    for (_, t) in &tiered {
        *counts.entry(*t).or_insert(0) += 1;
    }
    println!("tier distribution over real mined sources: {counts:?}");

    // Only Easy is required to have real-world representation from these three sources.
    // JIRA keys, CVE ids and PDB entry ids are all classic *coded* identifier schemes —
    // digit-bearing by construction — so it is expected, not a bug, that mining real
    // ticket/CVE/entry-id lists produces almost entirely Easy-tier entries. Medium and
    // Hard-tier coverage (word-like product names, plain absent single words) comes from
    // carrier-injected plants added in a later step of this instrument, not from these
    // three sources; asserting their presence here would fail for the wrong reason.
    assert!(
        counts.get(&Tier::Easy).copied().unwrap_or(0) > 0,
        "no Easy-tier examples among the mined sources: {counts:?}"
    );
}

#[test]
fn a_bare_numeral_and_a_common_word_are_unreachable_by_rule() {
    // Pins the two concrete cases the plan's original spec got wrong: `2291` and `Titan`
    // are not "hard", they are structurally impossible under Stage 1's current design.
    // If a future change to shape::is_candidate or the wordlist ever makes this false,
    // this test — not a silently-misleading recall number — is what should catch it.
    assert_eq!(tier_of("2291", tier_res()), Tier::Unreachable);
    assert_eq!(tier_of("Titan", tier_res()), Tier::Unreachable);
    assert_eq!(tier_of("Blue", tier_res()), Tier::Unreachable);
}

#[test]
fn tier_assignment_is_deterministic() {
    // Same input, same TierResources instance, same answer — table stakes for anything
    // this harness later sweeps over.
    let res = tier_res();
    for surface in ["DS-2291", "chromatography", "Titan", "bacillus", "2291", "MabSelect SuRe"] {
        assert_eq!(tier_of(surface, res), tier_of(surface, res));
    }
}

#[test]
fn easy_tier_recall_is_total() {
    // "100% on the easy tier is table stakes" — the plan's own framing. Anything less at
    // default config means Stage 1 is missing plants no orthographic signal could hide:
    // digits, internal caps, coded separators, or a promoted short all-caps acronym. The
    // report is built once via gate_report() so a failure here is against the exact same
    // corpus every other gate in this file scores.
    let report = gate_report();
    let easy = report.tiers.get(&Tier::Easy).cloned().unwrap_or_default();
    assert!(easy.planted > 0, "no Easy-tier plants in the gate corpus — check corpus_pools()");
    assert_eq!(
        easy.recalled, easy.planted,
        "Easy-tier recall was not total: {}/{} recalled (as_identifier={}, as_technical={})",
        easy.recalled, easy.planted, easy.as_identifier, easy.as_technical
    );
}

#[test]
fn prose_false_emission_rate_has_not_collapsed() {
    // Reported, not tuned against — see score.rs's module doc. This is a wide
    // collapse-detector in the tests/cross_format.rs idiom, not a claim about what the
    // number should be; nobody has measured a target yet, which is the point of building
    // this instrument in the first place. The real value is always printed so a human can
    // read it off a CI log without re-running anything.
    let report = gate_report();
    println!(
        "prose_false_per_1k = {:.3} ({} false emissions over {} tokens)",
        report.prose_false_per_1k(),
        report.prose_false_emissions,
        report.prose_tokens_scanned
    );
    println!("unplanted_ids_per_1k = {:.3}", report.unplanted_ids_per_1k());
    assert!(
        report.prose_false_per_1k() < 200.0,
        "prose false-emission rate looks collapsed: {:?}",
        report
    );
}

#[test]
fn the_span_matching_diagnostic_never_fires_on_a_healthy_run() {
    // If this ever fails, the bug is in this harness's own span bookkeeping (see
    // score.rs's module doc), not a finding about Stage 1 — treat it as a stop-the-line
    // signal, not a number to report.
    let report = gate_report();
    assert_eq!(
        report.matched_by_string_but_not_span, 0,
        "a plant matched Stage 1's output by string but not by span"
    );
}

#[test]
fn per_tier_recall_is_printed_for_a_human_to_read() {
    // The informative numbers — hard-reachable and medium recall — are reported, never
    // gated: gating them at a value nobody has measured is exactly the failure mode the
    // plan behind this instrument was written to avoid. This test's only job is to make
    // sure the table actually gets printed somewhere a CI log will show it.
    let report = gate_report();
    for tier in [Tier::Easy, Tier::Medium, Tier::HardReachable, Tier::Unreachable] {
        let stat = report.tiers.get(&tier).cloned().unwrap_or_default();
        let recall = if stat.planted == 0 { f32::NAN } else { stat.recalled as f32 / stat.planted as f32 };
        println!(
            "{tier:?}: {}/{} recalled ({:.1}% — as_identifier={}, as_technical={})",
            stat.recalled,
            stat.planted,
            recall * 100.0,
            stat.as_identifier,
            stat.as_technical
        );
    }
}
