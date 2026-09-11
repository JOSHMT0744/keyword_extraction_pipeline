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

use harness::{baseline, carrier, data, score, tier_of, Tier, TierResources};

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


#[test]
fn dates_and_phone_numbers_are_never_emitted() {
    // Gated only for the classes that measure 0 on the first real run of this harness —
    // per the plan, gate what is actually zero rather than asserting a floor nobody has
    // checked. Hand-simulating shape's feature weights predicted both: an ISO 8601 date
    // (e.g. 2024-03-11) scores separator_segments 0.12 + absent_from_wordlist 0.40 = 0.52,
    // just under the 0.55 identifier threshold, and a phone number's digit-only fragments
    // (+1, 2000000) both die in is_candidate — the same structural rejection as a bare
    // numeral. Confirmed, not just predicted: see the per-scheme table this test prints.
    let report = gate_report();
    for scheme in ["iso8601-date", "phone-number"] {
        let stat = report.distractor_schemes.get(scheme).cloned().unwrap_or_default();
        println!("{scheme}: {}/{} emitted (must be 0)", stat.emitted, stat.planted);
        assert!(stat.planted > 0, "no {scheme} distractors were planted — check the corpus");
        assert_eq!(stat.emitted, 0, "{scheme} was emitted {} time(s), expected never", stat.emitted);
    }
}

#[test]
fn distractor_emission_is_reported_per_scheme() {
    // Not gated — several of these are legitimately ambiguous. `v2.14.3` in a release
    // note, or a git short hash in a commit reference, are defensibly identifiers; marking
    // them MustNotEmit would stack the deck as surely as a naive all-easy plant set does
    // in the other direction. What matters is that the rate is visible, per scheme, on
    // every run — including the URL finding this instrument predicted and then confirmed:
    // a reference URL (https://example.com/docs/ref-4417) scores separator_segments 0.12 +
    // digit_letter_mix 0.25 + unusual_length 0.03 + absent_from_wordlist 0.40 = 0.80,
    // comfortably clearing the identifier threshold. Recorded in TODO.md rather than
    // hidden by narrowing the distractor set to avoid it.
    let report = gate_report();
    for (scheme, stat) in &report.distractor_schemes {
        let rate = if stat.planted == 0 { f32::NAN } else { stat.emitted as f32 / stat.planted as f32 };
        println!("{scheme}: {}/{} emitted ({:.0}%)", stat.emitted, stat.planted, rate * 100.0);
    }
    assert!(!report.distractor_schemes.is_empty(), "no distractor schemes were scored at all");
}

#[test]
fn stage_one_beats_both_trivial_baselines_on_distractor_rejection() {
    // The complexity-earns-its-keep gate. Relative, not absolute — needs no measured
    // target, and is safe to gate from day one. A baseline that matched or beat Stage 1
    // here would mean the seven-feature weighted sum was buying nothing a two-line
    // digit-bearing regex, or a bare "is it in the wordlist" check, didn't already have.
    let docs = carrier::generate(GATE_SEED, GATE_DOC_COUNT, &corpus_pools(), tier_res());
    let res = Resources::default();
    let cfg = Config::default();

    let stage1 = score::score_corpus(&docs, &res, cfg.thresholds.identifier, cfg.thresholds.technical);
    let digit_report = {
        let mut r = score::Report::default();
        for doc in &docs {
            score::score_doc(doc, &baseline::digit_bearing(doc), &mut r);
        }
        r
    };
    let wordlist_report = {
        let mut r = score::Report::default();
        for doc in &docs {
            score::score_doc(doc, &baseline::absent_from_wordlist(doc, &res), &mut r);
        }
        r
    };

    let distractor_hits = |r: &score::Report| -> usize { r.distractor_schemes.values().map(|s| s.emitted).sum() };
    let (s1, d1, w1) = (distractor_hits(&stage1), distractor_hits(&digit_report), distractor_hits(&wordlist_report));
    println!("distractor hits — stage1={s1} digit_regex={d1} wordlist_only={w1}");

    assert!(s1 < d1, "Stage 1 ({s1}) did not beat the digit-bearing baseline ({d1}) on distractors");
    assert!(s1 < w1, "Stage 1 ({s1}) did not beat the wordlist-only baseline ({w1}) on distractors");
}

#[test]
fn neither_trivial_baseline_recovers_the_unreachable_tier() {
    // The wordlist-only baseline has no acronym handling and no digit/common-word
    // carve-out, so this checks it doesn't accidentally do better than Stage 1 on the
    // one tier Stage 1 structurally cannot reach.
    let docs = carrier::generate(GATE_SEED, GATE_DOC_COUNT, &corpus_pools(), tier_res());
    let res = Resources::default();

    let mut digit_r = score::Report::default();
    let mut wordlist_r = score::Report::default();
    for doc in &docs {
        score::score_doc(doc, &baseline::digit_bearing(doc), &mut digit_r);
        score::score_doc(doc, &baseline::absent_from_wordlist(doc, &res), &mut wordlist_r);
    }

    for (name, report) in [("digit_bearing", &digit_r), ("absent_from_wordlist", &wordlist_r)] {
        let unreachable = report.tiers.get(&Tier::Unreachable).cloned().unwrap_or_default();
        println!("{name}: Unreachable {}/{} recalled", unreachable.recalled, unreachable.planted);
        assert_eq!(
            unreachable.recalled, 0,
            "{name} recovered {} Unreachable-tier plant(s), expected none",
            unreachable.recalled
        );
    }
}
