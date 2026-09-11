//! Parameter sweep for instrument 3 (identifier injection).
//!
//! Reports, never asserts — this is a diagnostic tool, not a test. `tests/injection.rs`
//! holds the actual gates; this program reuses the same harness (`tests/injection_harness/`,
//! included below the same way, via `#[path]`) purely to print tables a human reads to
//! decide whether `wordlist_size`, `acronym_wordlist_depth` or the thresholds should move.
//! Moving any of them is a separate decision, taken against this output — this program
//! makes no recommendation on its own.
//!
//! Run with `cargo run --release --example sweep_stage1` — `Resources::build` over an 80k
//! word list is painful to repeat ~50 times in a debug build.
//!
//! Cost structure: `wordlist_size` and `acronym_wordlist_depth` both require rebuilding
//! `Resources`, because they change the *candidate set* Stage 1 considers
//! (`is_candidate`/`acronym_flags` both consult `Resources`), not just which of a fixed
//! set of scores clears a threshold. Thresholds are free once a candidate set exists:
//! `score::candidates` runs Stage 1 with both thresholds open, and `score::emissions`
//! filters the result post hoc — one extraction per document per `Resources` build, not
//! one per grid point.

#[path = "../tests/injection_harness/mod.rs"]
mod harness;

use std::collections::BTreeMap;

use keyword_extraction_pipeline::{Config, Resources};

use harness::{carrier, data, score, tiers::TierResources, Tier};

const SEED: &str = "sweep-stage1-corpus-v1";
const DOC_COUNT: usize = 40;

const WORDLIST_SIZES: &[usize] = &[20_000, 30_000, 40_000, 50_000, 55_000, 60_000, 65_000, 70_000, 75_000, 80_000];
const TECH_THRESHOLDS: &[f32] = &[0.30, 0.34, 0.38, 0.42, 0.46];
const ID_THRESHOLDS: &[f32] = &[0.45, 0.50, 0.55, 0.60, 0.65];
const ACRONYM_DEPTHS: &[usize] = &[5_000, 10_000, 20_000, 30_000, 40_000];

const DEFAULT_WORDLIST_SIZE: usize = 65_000;
const DEFAULT_TECH_THRESHOLD: f32 = 0.38;
const DEFAULT_ID_THRESHOLD: f32 = 0.55;
const DEFAULT_ACRONYM_DEPTH: usize = 20_000;

struct Point {
    wordlist_size: usize,
    t_tech: f32,
    medium: f32,
    hard: f32,
    distractor_rate: f32,
    prose_fp_per_1k: f32,
    unplanted_ids_per_1k: f32,
}

fn main() {
    // Fixed 80k-depth resources for tier assignment only — never the wordlist_size under
    // sweep. See tiers.rs's module doc: if tier assignment moved with the sweep, a plant
    // could migrate tiers mid-sweep and every curve below would measure a moving target.
    let tier_res = TierResources::build();
    let pools = carrier::Pools {
        identifiers: data::sources(150),
        distractors: data::distractors(150),
        acronyms: carrier::Pools::default_acronyms(),
    };
    let docs = carrier::generate(SEED, DOC_COUNT, &pools, &tier_res);

    println!("instrument 3 sweep — {DOC_COUNT} documents, seed {SEED:?}\n");

    let mut points: Vec<Point> = Vec::new();
    for &wordlist_size in WORDLIST_SIZES {
        let cfg = Config { wordlist_size, ..Config::default() };
        let res = Resources::for_config(&cfg);

        // One candidate extraction per document at this wordlist_size — everything below
        // reuses it, filtering post hoc.
        let per_doc_candidates: Vec<_> = docs.iter().map(|d| score::candidates(d, &res)).collect();

        for &t_tech in TECH_THRESHOLDS {
            let mut report = score::Report::default();
            for (doc, cands) in docs.iter().zip(&per_doc_candidates) {
                let ems = score::emissions(cands, DEFAULT_ID_THRESHOLD, t_tech);
                score::score_doc(doc, &ems, &mut report);
            }
            points.push(Point {
                wordlist_size,
                t_tech,
                medium: recall(&report, Tier::Medium),
                hard: recall(&report, Tier::HardReachable),
                distractor_rate: distractor_rate(&report),
                prose_fp_per_1k: report.prose_false_per_1k(),
                unplanted_ids_per_1k: report.unplanted_ids_per_1k(),
            });
        }
    }

    print_table("T1: MEDIUM-TIER RECALL", &points, |p| p.medium);
    print_table("T2: HARD-REACHABLE-TIER RECALL (the informative number)", &points, |p| p.hard);
    print_table("T3: DISTRACTOR EMISSION RATE (reported classes, not gated)", &points, |p| p.distractor_rate);
    print_table("T4: PROSE FALSE-EMISSION PER 1000 TOKENS (the cost axis)", &points, |p| p.prose_fp_per_1k);
    print_table("T5: UNPLANTED IDENTIFIERS PER 1000 TOKENS (volume signal)", &points, |p| p.unplanted_ids_per_1k);

    sweep_identifier_threshold(&docs, &tier_res, &pools);
    sweep_acronym_depth(&docs, &tier_res);
    print_frontier(&points);
}

fn recall(report: &score::Report, tier: Tier) -> f32 {
    let stat = report.tiers.get(&tier).cloned().unwrap_or_default();
    if stat.planted == 0 {
        f32::NAN
    } else {
        stat.recalled as f32 / stat.planted as f32
    }
}

fn distractor_rate(report: &score::Report) -> f32 {
    let (planted, emitted): (usize, usize) =
        report.distractor_schemes.values().fold((0, 0), |(p, e), s| (p + s.planted, e + s.emitted));
    if planted == 0 {
        f32::NAN
    } else {
        emitted as f32 / planted as f32
    }
}

fn print_table(title: &str, points: &[Point], value: impl Fn(&Point) -> f32) {
    println!("== {title} ==");
    print!("{:>12}", "wordlist");
    for &t in TECH_THRESHOLDS {
        print!(" {t:>8.2}");
    }
    println!();

    for &wordlist_size in WORDLIST_SIZES {
        let marker = if wordlist_size == DEFAULT_WORDLIST_SIZE { "*" } else { " " };
        print!("{marker}{wordlist_size:>11}");
        for &t_tech in TECH_THRESHOLDS {
            let v = points
                .iter()
                .find(|p| p.wordlist_size == wordlist_size && (p.t_tech - t_tech).abs() < 1e-6)
                .map(&value)
                .unwrap_or(f32::NAN);
            print!(" {v:>8.3}");
        }
        println!();
    }
    println!("(* marks the current default wordlist_size = {DEFAULT_WORDLIST_SIZE}, columns are thresholds.technical)\n");
}

/// `thresholds.identifier` barely interacts with `wordlist_size` — digit-bearing tokens
/// are absent from the wordlist regardless of its size, since the list is alphabetic-only
/// — so it gets its own small table at the default wordlist_size rather than bloating the
/// main grid with an axis that mostly doesn't move.
fn sweep_identifier_threshold(docs: &[carrier::Doc], tier_res: &TierResources, _pools: &carrier::Pools) {
    println!("== IDENTIFIER THRESHOLD (at default wordlist_size = {DEFAULT_WORDLIST_SIZE}) ==");
    let cfg = Config { wordlist_size: DEFAULT_WORDLIST_SIZE, ..Config::default() };
    let res = Resources::for_config(&cfg);
    let per_doc_candidates: Vec<_> = docs.iter().map(|d| score::candidates(d, &res)).collect();

    println!("{:>10} {:>10} {:>10}", "t_id", "easy_recall", "medium_recall");
    for &t_id in ID_THRESHOLDS {
        let mut report = score::Report::default();
        for (doc, cands) in docs.iter().zip(&per_doc_candidates) {
            let ems = score::emissions(cands, t_id, DEFAULT_TECH_THRESHOLD);
            score::score_doc(doc, &ems, &mut report);
        }
        let marker = if (t_id - DEFAULT_ID_THRESHOLD).abs() < 1e-6 { "*" } else { " " };
        println!(
            "{marker}{t_id:>9.2} {:>10.3} {:>10.3}",
            recall(&report, Tier::Easy),
            recall(&report, Tier::Medium)
        );
    }
    let _ = tier_res;
    println!();
}

/// `acronym_wordlist_depth` touches nothing but short (2-6 char) all-caps tokens — see
/// `tiers.rs`'s acronym branch — so it is swept separately and narrowly, not folded into
/// the main grid above. The cost axis here (shouted-heading false emissions) comes from
/// carrier text this program authored, not real-world prose, so this table is weaker
/// evidence than the identifier tiers above and should not borrow their credibility.
fn sweep_acronym_depth(docs: &[carrier::Doc], tier_res: &TierResources) {
    println!("== T6: ACRONYM_WORDLIST_DEPTH (narrow — see module doc) ==");
    println!("{:>10} {:>14} {:>18}", "depth", "acronym_recall", "shouted_fp_per_1k");

    for &depth in ACRONYM_DEPTHS {
        let cfg = Config {
            wordlist_size: DEFAULT_WORDLIST_SIZE,
            acronym_wordlist_depth: depth,
            ..Config::default()
        };
        let res = Resources::for_config(&cfg);
        let mut report = score::Report::default();
        for doc in docs {
            let cands = score::candidates(doc, &res);
            let ems = score::emissions(&cands, DEFAULT_ID_THRESHOLD, DEFAULT_TECH_THRESHOLD);
            score::score_doc(doc, &ems, &mut report);
        }
        // Acronym-shaped plants are Easy-tier under tiers.rs's own rule (see its module
        // doc), so "acronym recall" here is exactly the Easy-tier recall at this depth.
        let marker = if depth == DEFAULT_ACRONYM_DEPTH { "*" } else { " " };
        println!(
            "{marker}{depth:>9} {:>14.3} {:>18.3}",
            recall(&report, Tier::Easy),
            report.prose_false_per_1k()
        );
    }
    let _ = tier_res;
    println!();
}

/// The deliverable an operator actually reads: for each false-positive budget, the best
/// hard-reachable recall available and where it sits, with today's default located on the
/// curve. Not a recommendation — a printed trade-off, so the first question answered is
/// "is the current default on the frontier at all".
fn print_frontier(points: &[Point]) {
    println!("== FRONTIER (max hard-reachable recall at each prose-false-emission budget) ==");
    let budgets = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, f32::INFINITY];
    let mut seen: BTreeMap<u64, ()> = BTreeMap::new();
    for &budget in &budgets {
        let best = points
            .iter()
            .filter(|p| p.prose_fp_per_1k <= budget && !p.hard.is_nan())
            .max_by(|a, b| a.hard.total_cmp(&b.hard));
        let Some(p) = best else { continue };
        // Skip a budget that lands on the same point as the previous one printed.
        let key = ((p.wordlist_size as u64) << 32) | (p.t_tech * 1000.0) as u64;
        if seen.insert(key, ()).is_some() {
            continue;
        }
        let budget_label = if budget.is_finite() { format!("{budget:>5.1}") } else { "  inf".to_string() };
        println!(
            "  prose_fp/1k <= {budget_label} : wordlist={:<6} tech={:.2}  hard={:.3} medium={:.3} distractor={:.3} prose_fp/1k={:.2}",
            p.wordlist_size, p.t_tech, p.hard, p.medium, p.distractor_rate, p.prose_fp_per_1k
        );
    }

    if let Some(default) = points
        .iter()
        .find(|p| p.wordlist_size == DEFAULT_WORDLIST_SIZE && (p.t_tech - DEFAULT_TECH_THRESHOLD).abs() < 1e-6)
    {
        println!(
            "  * current default   : wordlist={:<6} tech={:.2}  hard={:.3} medium={:.3} distractor={:.3} prose_fp/1k={:.2}",
            default.wordlist_size, default.t_tech, default.hard, default.medium, default.distractor_rate, default.prose_fp_per_1k
        );
    }
}
