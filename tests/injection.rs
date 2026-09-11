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
//! Built incrementally; see the plan this instrument was built from. This first slice
//! covers only the anti-fudge machinery — tier assignment and data provenance — which is
//! checkable before any carrier document or scorer exists. Later slices add the corpus
//! generator, the scorer, and the easy-tier / prose-false-positive gates.

#[path = "injection_harness/mod.rs"]
mod harness;

use harness::{data, tier_of, Tier, TierResources};

fn tier_res() -> &'static TierResources {
    static R: std::sync::OnceLock<TierResources> = std::sync::OnceLock::new();
    R.get_or_init(TierResources::build)
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
