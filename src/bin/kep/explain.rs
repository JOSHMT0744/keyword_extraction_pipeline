//! `kep explain` — account for one document's scores.
//!
//! The plan describes Lane 1 as "a transparent weighted sum with stored components".
//! Transparency that is computed and discarded is not transparency, so this command is
//! the other half of that claim: for every candidate it prints each feature's raw value,
//! the weight applied to it, the contribution that produced, and whether the total
//! cleared its kind's threshold.
//!
//! Near-misses matter more than hits. A hit tells you the threshold is low enough; only
//! the rejected candidates tell you what a lower one would have admitted.

use std::io::{self, Write};

use keyword_extraction_pipeline::{
    canonicalise,
    config::ShapeWeights,
    lanes::shape::{self, ShapeFeatures},
    Config, DocumentStatus, FormatHint, Keyword, Kind, Resources,
};

use crate::{cli::ExplainArgs, render};

pub fn run(args: &ExplainArgs, cfg: &Config, out: &mut impl Write) -> io::Result<i32> {
    let bytes = match std::fs::read(&args.path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{}: {e}", args.path.display());
            return Ok(1);
        }
    };

    // Features are forced on regardless of the resolved config: this command exists to
    // show them, and requiring a flag to make its own output non-empty would be absurd.
    let cfg_kept = Config { retain_features: true, ..cfg.clone() };
    let res = Resources::for_config(&cfg_kept);

    let result = keyword_extraction_pipeline::extract(&bytes, FormatHint::Sniff, &cfg_kept, &res);
    writeln!(out, "{}", args.path.display())?;
    writeln!(
        out,
        "  status {}   pipeline_version {}",
        render::status_label(&result.status),
        result.pipeline_version.short()
    )?;

    if result.status != DocumentStatus::Ok {
        writeln!(out, "\n  Nothing to account for: the document never reached scoring.")?;
        return Ok(0);
    }

    writeln!(out, "\n  Weights: {}", weights_line(&cfg_kept.shape_weights))?;
    writeln!(
        out,
        "  Thresholds: identifier {:.2}, technical {:.2}\n",
        cfg_kept.thresholds.identifier, cfg_kept.thresholds.technical
    )?;

    writeln!(out, "  EMITTED")?;
    if result.keywords.is_empty() {
        writeln!(out, "    (none)")?;
    }
    for k in &result.keywords {
        write_candidate(out, k, &cfg_kept.shape_weights, true)?;
    }

    if args.near_misses {
        write_near_misses(out, &bytes, &cfg_kept, args.near_miss_limit)?;
    }

    Ok(0)
}

/// Re-run Lane 1 with both thresholds at zero, then subtract what was emitted.
///
/// Re-running rather than instrumenting the lane keeps the explain path and the extract
/// path the same code: anything shown here was produced by the scorer that actually runs.
fn write_near_misses(
    out: &mut impl Write,
    bytes: &[u8],
    cfg: &Config,
    limit: usize,
) -> io::Result<()> {
    let mut open = cfg.clone();
    open.thresholds.identifier = 0.0;
    open.thresholds.technical = 0.0;
    let res = Resources::for_config(&open);

    let Ok(text) = canonicalise(bytes, FormatHint::Sniff, &open) else {
        return Ok(());
    };
    let all = shape::extract(&text, &open, &res);

    let mut rejected: Vec<&Keyword> = all
        .iter()
        .filter(|k| k.score < threshold_for(k.kind, cfg))
        .collect();
    rejected.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.normalised.cmp(&b.normalised)));

    writeln!(out, "\n  BELOW THRESHOLD (what a lower threshold would admit, best first)")?;
    if rejected.is_empty() {
        writeln!(out, "    (none — every candidate cleared its threshold)")?;
        return Ok(());
    }
    for k in rejected.iter().take(limit) {
        write_candidate(out, k, &cfg.shape_weights, false)?;
    }
    if rejected.len() > limit {
        writeln!(out, "    … {} more (raise --near-miss-limit)", rejected.len() - limit)?;
    }
    Ok(())
}

fn write_candidate(
    out: &mut impl Write,
    k: &Keyword,
    w: &ShapeWeights,
    emitted: bool,
) -> io::Result<()> {
    let mark = if emitted { "+" } else { "-" };
    writeln!(
        out,
        "    {mark} {:<28} {:?}  score {:.3}  x{}",
        k.surface, k.kind, k.score, k.frequency
    )?;
    let Some(f) = &k.features else { return Ok(()) };
    for (name, value, weight) in components(f, w) {
        if value == 0.0 {
            continue;
        }
        writeln!(out, "        {name:<22} {value:>5.2} x {weight:.2} = {:.3}", value * weight)?;
    }
    Ok(())
}

fn components(f: &ShapeFeatures, w: &ShapeWeights) -> [(&'static str, f32, f32); 7] {
    [
        ("absent_from_wordlist", f.absent_from_wordlist, w.absent_from_wordlist),
        ("digit_letter_mix", f.digit_letter_mix, w.digit_letter_mix),
        ("internal_caps", f.internal_caps, w.internal_caps),
        ("separator_segments", f.separator_segments, w.separator_segments),
        ("short_all_caps", f.short_all_caps, w.short_all_caps),
        ("unusual_length", f.unusual_length, w.unusual_length),
        ("in_document_frequency", f.in_document_frequency, w.in_document_frequency),
    ]
}

fn threshold_for(kind: Kind, cfg: &Config) -> f32 {
    match kind {
        Kind::Identifier => cfg.thresholds.identifier,
        _ => cfg.thresholds.technical,
    }
}

fn weights_line(w: &ShapeWeights) -> String {
    components(&ShapeFeatures::default(), w)
        .iter()
        .map(|(name, _, weight)| format!("{name} {weight:.2}"))
        .collect::<Vec<_>>()
        .join(", ")
}
