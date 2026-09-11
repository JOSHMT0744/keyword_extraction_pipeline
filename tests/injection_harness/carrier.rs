//! Generates a plain-text corpus by filling typed slots in the templates under
//! `tests/data/injection/carriers/*.txt` with plants drawn from the mined identifier and
//! distractor data.
//!
//! Carriers are fed directly to `shape::extract`, never through `canonicalise`. Instrument
//! 3 measures Stage 1's scoring; parsing correctness is instrument 1's job
//! (`tests/parsing.rs`), and routing through the parser would make every recorded plant
//! span depend on canonicalisation behaviour as well — which is exactly the kind of
//! "nearly the same, but not quite" gap this instrument exists to have zero tolerance for.
//!
//! A template's slots sit in five distinct regions on purpose — a title, body prose
//! mid-sentence, a line-per-row semi-tabular block, a shouted all-caps heading, and a
//! closing paragraph — so that "natural-position injection" is a structural property of
//! the templates rather than a claim about them. The tabular block specifically exercises
//! `same_line` (the guard against the `DS-2291 HEK293T DS-2292` row-splicing phantom
//! documented in `shape::collect_phrases`), and the shouted heading is the only place
//! `acronym_wordlist_depth` has anything to bite on — without one, sweeping that setting
//! would have nothing to measure.
//!
//! The tension worth naming rather than hiding: these templates are hand-authored, so
//! "natural position" is a claim about text we wrote, not text mined from a document.
//! Fully mechanical slot placement would be more obviously neutral, but it would also
//! destroy the paragraph structure that `same_line`, phrase merging and
//! `in_document_frequency` all depend on to mean anything. The structural gate in
//! `tests/injection.rs` (multiple distinct regions, plants not appended as a list) is the
//! check against that risk; it does not eliminate the tension, only bounds it.

use std::ops::Range;

use super::data::Entry;
use super::rng::Rng;
use super::tiers::{tier_of, Tier, TierResources};

pub struct Template {
    pub register: &'static str,
    pub text: &'static str,
}

/// Slot markers recognised in a template body.
const SLOT_ID: &str = "{{ID}}";
const SLOT_DIS: &str = "{{DIS}}";
const SLOT_ACR: &str = "{{ACR}}";

fn templates() -> Vec<Template> {
    macro_rules! carrier {
        ($register:literal, $path:literal) => {
            Template { register: $register, text: strip_header(include_str!($path)) }
        };
    }
    vec![
        carrier!("lab", "../data/injection/carriers/lab_report.txt"),
        carrier!("invoice", "../data/injection/carriers/invoice.txt"),
        carrier!("it", "../data/injection/carriers/it_ticket.txt"),
        carrier!("minutes", "../data/injection/carriers/meeting_minutes.txt"),
    ]
}

/// Templates carry a small `#`-prefixed header of their own (for a human reading the raw
/// file) ending in a `---` line; strip it so only the document body reaches the generator.
fn strip_header(text: &'static str) -> &'static str {
    match text.find("\n---\n") {
        Some(i) => &text[i + 5..],
        None => text,
    }
}

/// What was substituted at one slot, and where it landed in the generated document.
#[derive(Debug, Clone)]
pub struct Plant {
    pub surface: String,
    pub span: Range<usize>,
    pub tier: Tier,
    pub scheme: String,
    /// `true` for a distractor (must not be counted as a hit), `false` for a genuine
    /// planted identifier.
    pub is_distractor: bool,
}

#[derive(Debug, Clone)]
pub struct Doc {
    pub register: String,
    pub text: String,
    pub plants: Vec<Plant>,
}

pub struct Pools {
    pub identifiers: Vec<Entry>,
    pub distractors: Vec<Entry>,
    /// Short all-caps acronym-shaped strings for the `{{ACR}}` slot specifically — drawn
    /// separately from the general identifier pool because the shouted-heading region
    /// only means something for a token `is_short_all_caps` would actually consider.
    pub acronyms: Vec<&'static str>,
}

impl Pools {
    pub fn default_acronyms() -> Vec<&'static str> {
        // A mix of real short-all-caps schemes this crate's own tests already reference
        // (SOP, HPLC, ELN, LIMS, QMS) plus generic ones for the non-lab registers.
        vec!["SOP", "HPLC", "ELN", "LIMS", "QMS", "PO", "KYC", "SLA", "NDA", "VAT"]
    }
}

/// Generate a deterministic corpus of `n` documents from `seed`.
///
/// One document per call into `templates()`, cycled, so every register appears roughly
/// equally rather than being left to chance — a corpus that happened to skip the `minutes`
/// template would silently under-test that register's structure.
pub fn generate(seed: &str, n: usize, pools: &Pools, tier_res: &TierResources) -> Vec<Doc> {
    let templates = templates();
    let mut rng = Rng::new(seed, "carrier-choice");
    let mut docs = Vec::with_capacity(n);

    for i in 0..n {
        let template = &templates[i % templates.len()];
        docs.push(fill(seed, i, template, pools, tier_res, &mut rng));
    }
    docs
}

fn fill(
    seed: &str,
    doc_index: usize,
    template: &Template,
    pools: &Pools,
    tier_res: &TierResources,
    template_choice_rng: &mut Rng,
) -> Doc {
    // Domain-separated per document index, so adding a slot to one document's fill order
    // cannot perturb another document's draws — see rng.rs's module doc.
    let domain = format!("slot-fill-{doc_index}");
    let mut rng = Rng::new(seed, &domain);
    let _ = template_choice_rng; // reserved: template order is currently deterministic (cycled), not drawn.

    let mut text = String::new();
    let mut plants = Vec::new();
    let mut rest = template.text;

    loop {
        let next_id = rest.find(SLOT_ID);
        let next_dis = rest.find(SLOT_DIS);
        let next_acr = rest.find(SLOT_ACR);

        let candidates = [
            next_id.map(|i| (i, SLOT_ID)),
            next_dis.map(|i| (i, SLOT_DIS)),
            next_acr.map(|i| (i, SLOT_ACR)),
        ];
        let Some((pos, marker)) = candidates.into_iter().flatten().min_by_key(|(i, _)| *i) else {
            text.push_str(rest);
            break;
        };

        text.push_str(&rest[..pos]);
        let start = text.len();

        let (surface, tier, scheme, is_distractor) = match marker {
            SLOT_ID => {
                let e = rng.choose(&pools.identifiers);
                let t = tier_of(&e.surface, tier_res);
                (e.surface.clone(), t, e.scheme.clone(), false)
            }
            SLOT_DIS => {
                let e = rng.choose(&pools.distractors);
                let t = tier_of(&e.surface, tier_res);
                (e.surface.clone(), t, e.scheme.clone(), true)
            }
            SLOT_ACR => {
                let a = *rng.choose(&pools.acronyms);
                let t = tier_of(a, tier_res);
                (a.to_string(), t, "acronym".to_string(), false)
            }
            _ => unreachable!(),
        };

        text.push_str(&surface);
        let end = text.len();
        plants.push(Plant { surface, span: start..end, tier, scheme, is_distractor });

        rest = &rest[pos + marker.len()..];
    }

    Doc { register: template.register.to_string(), text, plants }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::data;

    fn test_pools() -> Pools {
        Pools {
            identifiers: data::sources(150),
            distractors: data::distractors(150),
            acronyms: Pools::default_acronyms(),
        }
    }

    fn tier_res() -> &'static TierResources {
        static R: std::sync::OnceLock<TierResources> = std::sync::OnceLock::new();
        R.get_or_init(TierResources::build)
    }

    #[test]
    fn generation_is_byte_identical_for_a_fixed_seed() {
        let pools = test_pools();
        let a = generate("fixed-seed-1", 8, &pools, tier_res());
        let b = generate("fixed-seed-1", 8, &pools, tier_res());
        assert_eq!(a.len(), b.len());
        for (da, db) in a.iter().zip(b.iter()) {
            assert_eq!(da.text, db.text, "text differs for register {}", da.register);
            assert_eq!(
                da.plants.iter().map(|p| (p.surface.clone(), p.span.clone())).collect::<Vec<_>>(),
                db.plants.iter().map(|p| (p.surface.clone(), p.span.clone())).collect::<Vec<_>>(),
            );
        }
    }

    #[test]
    fn different_seeds_produce_different_documents() {
        let pools = test_pools();
        let a = generate("seed-a", 4, &pools, tier_res());
        let b = generate("seed-b", 4, &pools, tier_res());
        assert_ne!(
            a.iter().map(|d| &d.text).collect::<Vec<_>>(),
            b.iter().map(|d| &d.text).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn every_plant_span_is_the_recorded_surface() {
        let pools = test_pools();
        for doc in generate("span-check-seed", 12, &pools, tier_res()) {
            for p in &doc.plants {
                assert_eq!(
                    &doc.text[p.span.clone()],
                    p.surface,
                    "recorded span does not match its own surface in register {}",
                    doc.register
                );
            }
        }
    }

    #[test]
    fn every_register_appears_in_a_large_enough_corpus() {
        let pools = test_pools();
        let docs = generate("coverage-seed", 12, &pools, tier_res());
        let registers: std::collections::HashSet<&str> =
            docs.iter().map(|d| d.register.as_str()).collect();
        assert_eq!(registers.len(), 4, "expected all 4 carrier registers, got {registers:?}");
    }

    #[test]
    fn plants_land_in_at_least_three_distinct_regions_per_document() {
        // Structural check for "natural-position injection, never appended blocks": a
        // document is split into blank-line-separated blocks, and plants must not all
        // cluster into one of them.
        let pools = test_pools();
        for doc in generate("region-check-seed", 8, &pools, tier_res()) {
            let mut block_starts: Vec<usize> = vec![0];
            let mut pos = 0;
            for blank in doc.text.match_indices("\n\n") {
                pos = blank.0 + 2;
                block_starts.push(pos);
            }
            let _ = pos;

            let block_of = |offset: usize| -> usize {
                block_starts.iter().rposition(|&s| s <= offset).unwrap_or(0)
            };
            let regions: std::collections::HashSet<usize> =
                doc.plants.iter().map(|p| block_of(p.span.start)).collect();
            assert!(
                regions.len() >= 3,
                "register {} plants landed in only {} distinct regions",
                doc.register,
                regions.len()
            );
        }
    }

    #[test]
    fn plants_are_not_all_appended_after_the_last_prose_paragraph() {
        // A appended-block injection scheme would put every plant span at or after some
        // fixed cutoff near the end of the text. Assert at least one plant sits in the
        // first half of the document — a direct check against "appended blocks".
        let pools = test_pools();
        for doc in generate("appended-check-seed", 8, &pools, tier_res()) {
            let midpoint = doc.text.len() / 2;
            assert!(
                doc.plants.iter().any(|p| p.span.start < midpoint),
                "register {}: every plant landed in the second half of the document",
                doc.register
            );
        }
    }
}
