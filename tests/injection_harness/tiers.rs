//! Difficulty-tier assignment for instrument 3.
//!
//! **Tier is computed, not declared.** A source file records where a plant came from and
//! what scheme it belongs to; it never records a difficulty. If it did, the same hand that
//! chose which entries to include could also choose which bucket to put them in, and the
//! headline "hard-tier recall" number would mean nothing more than "recall over whatever
//! we decided was hard". `tier_of` is a fixed, mechanical rule — the only thing that can
//! move an entry between tiers is a change to this function, and that change is reviewable
//! on its own, independent of any one plant.
//!
//! Provenance (the source file's header) is the audit trail — it lets a reader check where
//! an entry came from. It is not, on its own, proof the difficulty is fair; that's what
//! this rule is for.

use keyword_extraction_pipeline::{Config, Resources};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// Obviously coded: has digits, internal capitals, or a separator run.
    Easy,
    /// Word-like but distinctive: multi-word, internal caps, or simply absent from
    /// general English.
    Medium,
    /// A single, ordinary-looking English word that happens to be rare enough to be
    /// absent from the wordlist at typical cutoffs. Genuinely hard, and the only tier a
    /// `wordlist_size` sweep can move in principle.
    HardReachable,
    /// Cannot be emitted by Stage 1 at any configuration: a bare numeral, or a word
    /// common enough that no plausible `wordlist_size` would treat it as absent. See
    /// `shape::is_candidate`, which rejects both before scoring ever runs.
    Unreachable,
}

/// The wordlist depth tier assignment is evaluated at. Fixed, and deliberately NOT the
/// `wordlist_size` under test elsewhere in the harness — if tier assignment moved with the
/// sweep, a plant could migrate tiers mid-sweep and every reported curve would be
/// measuring a moving target instead of a fixed question.
const FULL_DEPTH: usize = 80_000;

/// The depth below which a word is common enough that no default in this crate's
/// plausible range would ever call it "absent". Mirrors the crate's own default
/// `acronym_wordlist_depth` (20 000) as the boundary between "shouted English" and
/// "acronym" — the same order of magnitude below which a word is unambiguously ordinary.
const CEILING_DEPTH: usize = 20_000;

/// Resources built once at the two fixed reference depths tier assignment needs.
pub struct TierResources {
    full: Resources,
    shallow: Resources,
}

impl TierResources {
    pub fn build() -> Self {
        Self {
            full: Resources::for_config(&Config { wordlist_size: FULL_DEPTH, ..Config::default() }),
            shallow: Resources::for_config(&Config { wordlist_size: CEILING_DEPTH, ..Config::default() }),
        }
    }
}

/// Local copy of `shape::has_internal_caps` — private to the library, and duplicating a
/// five-line predicate here is cheaper than widening the crate's public API for a test
/// harness that is not part of its contract.
fn has_internal_caps(text: &str) -> bool {
    let mut chars = text.chars();
    let _ = chars.next();
    chars.clone().any(char::is_uppercase) && chars.any(char::is_lowercase)
}

fn has_digit(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_digit())
}

/// Assign a difficulty tier to a surface, by rule.
///
/// Mirrors (but does not call — those functions are private) the exact structural checks
/// `shape::is_candidate` and `shape::classify` apply, so that `Unreachable` really does
/// mean "Stage 1's own code rejects this before scoring", not an approximation of it.
pub fn tier_of(surface: &str, res: &TierResources) -> Tier {
    let has_letter = surface.chars().any(char::is_alphabetic);
    if !has_letter {
        // Bare numeral. `shape::is_candidate` rejects any token with no alphabetic
        // character outright — this can never be emitted, at any configuration.
        return Tier::Unreachable;
    }

    // Checked per word, not over the whole surface: `shape::has_internal_caps` is applied
    // per-token in the real pipeline (a phrase merges only when at least one of its
    // component tokens has internal caps), so a multi-word surface must be judged the
    // same way here or a phrase like `MabSelect SuRe` — which the real pipeline would
    // call Easy on `MabSelect` alone — could be misjudged as Medium by this rule instead.
    let any_word_has_digit_or_caps = surface
        .split_whitespace()
        .any(|w| has_digit(w) || has_internal_caps(w));
    if any_word_has_digit_or_caps || keyword_extraction_pipeline::tokenize::separator_segments(surface) > 1 {
        return Tier::Easy;
    }

    // Everything below here is wholly alphabetic, single-case, no separators: the
    // orthography carries no signal at all, and the entire question is whether the word
    // is common enough to look ordinary.
    let multi_word = surface.contains(' ');
    let lower = surface.to_lowercase();

    if res.shallow.is_common_word(&lower) {
        // Common enough that even the shallowest plausible wordlist calls it ordinary.
        // No wordlist_size in this crate's plausible sweep range reaches down this far —
        // this is the `Titan` / `Blue` case, a ceiling `is_candidate` enforces via
        // `absent_from_wordlist` never clearing the technical threshold in practice.
        return Tier::Unreachable;
    }

    if multi_word || !res.full.is_common_word(&lower) {
        // Absent even at the deepest embedded depth (80k): domain vocabulary with no
        // ordinary-English signal at all, or a multi-word name. Distinctive on its own.
        return Tier::Medium;
    }

    // Present somewhere in the 80k list, but only past the shallow ceiling: a real,
    // ordinary-looking single word that a `wordlist_size` sweep can plausibly reach.
    Tier::HardReachable
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res() -> &'static TierResources {
        static R: std::sync::OnceLock<TierResources> = std::sync::OnceLock::new();
        R.get_or_init(TierResources::build)
    }

    #[test]
    fn bare_numerals_are_unreachable() {
        assert_eq!(tier_of("2291", res()), Tier::Unreachable);
    }

    #[test]
    fn common_word_collisions_are_unreachable() {
        // titan rank ~10 625, blue rank ~791 — both well inside CEILING_DEPTH.
        assert_eq!(tier_of("Titan", res()), Tier::Unreachable);
        assert_eq!(tier_of("Blue", res()), Tier::Unreachable);
    }

    #[test]
    fn digit_bearing_or_internally_capped_tokens_are_easy() {
        assert_eq!(tier_of("DS-2291", res()), Tier::Easy);
        assert_eq!(tier_of("HEK293T", res()), Tier::Easy);
        assert_eq!(tier_of("MabSelect", res()), Tier::Easy);
    }

    #[test]
    fn multi_word_terms_with_no_internal_caps_are_medium() {
        // `MabSelect SuRe` is NOT the right example here: `MabSelect` alone has internal
        // caps, so per the per-token rule above the phrase is Easy, matching how the real
        // pipeline would score it. `Capto Adhere` has no internal caps in either word.
        assert_eq!(tier_of("Capto Adhere", res()), Tier::Medium);
    }

    #[test]
    fn a_word_absent_at_any_depth_is_medium() {
        // Absent from the embedded list even at its full 80k depth: sepharose, superdex
        // and elution are the standing examples elsewhere in this crate of domain
        // vocabulary with no wordlist entry at all, at any cutoff.
        for w in ["sepharose", "superdex", "elution"] {
            assert_eq!(tier_of(w, res()), Tier::Medium, "{w} should be absent-at-any-depth");
        }
    }

    #[test]
    fn a_word_present_only_deep_in_the_list_is_hard_but_reachable() {
        // `chromatography` is rank 78 504 — inside the embedded list's 80k depth, so it
        // is NOT "absent at any depth" (that's the Medium case above). It IS the
        // canonical example, repeated throughout this crate's own tests and docs, of a
        // single technical word invisible at the 65k default and visible again only if
        // wordlist_size is pushed close to the full 80k — exactly the case a
        // `wordlist_size` sweep can move, which is what HardReachable means.
        assert_eq!(tier_of("chromatography", res()), Tier::HardReachable);
        assert_eq!(tier_of("bacillus", res()), Tier::HardReachable);
    }
}

