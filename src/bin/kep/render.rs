//! Human and machine rendering.
//!
//! The rule that makes the table worth reading: **an empty kind states why it is empty.**
//! "no identifiers" and "no identifiers above 0.55" are different claims, and only the
//! second tells you whether to move a threshold. A blank line where keywords should be
//! is the same silent-omission failure `DocumentStatus` exists to prevent, one level down.

use std::io::{self, Write};

use keyword_extraction_pipeline::{Config, DocumentResult, DocumentStatus, Keyword, Kind};

use crate::record::DocumentRecord;

/// Widest surface form shown before truncation, so the columns stay aligned on a
/// terminal without wrapping.
const SURFACE_WIDTH: usize = 28;

pub fn jsonl(out: &mut impl Write, record: &DocumentRecord<'_>) -> io::Result<()> {
    let line = serde_json::to_string(record).expect("record is serialisable");
    writeln!(out, "{line}")
}

pub fn csv_header(out: &mut impl Write) -> io::Result<()> {
    writeln!(out, "path,status,kind,rank,score,frequency,surface,normalised,origin,expansion")
}

pub fn csv(out: &mut impl Write, record: &DocumentRecord<'_>) -> io::Result<()> {
    let status = status_label(record.status);
    // A document with no keywords still gets a row. Dropping it would make an empty
    // result indistinguishable from a document that was never scanned.
    if record.by_kind().iter().all(|(_, ks)| ks.is_empty()) {
        return writeln!(out, "{},{status},,,,,,,,", quote(&record.path));
    }
    for (kind, keywords) in record.by_kind() {
        for k in keywords {
            writeln!(
                out,
                "{},{status},{:?},{},{:.4},{},{},{},{:?},{}",
                quote(&record.path),
                kind,
                k.rank,
                k.score,
                k.frequency,
                quote(&k.surface),
                quote(&k.normalised),
                k.origin,
                quote(k.expansion.as_deref().unwrap_or("")),
            )?;
        }
    }
    Ok(())
}

pub fn table(
    out: &mut impl Write,
    record: &DocumentRecord<'_>,
    result: &DocumentResult,
    cfg: &Config,
) -> io::Result<()> {
    let language = match &record.language {
        Some(l) if l.fully_supported => format!("{} {:.2}", l.code, record.language_confidence),
        Some(l) => format!("{} {:.2} (degraded)", l.code, record.language_confidence),
        None => "lang?".to_string(),
    };

    writeln!(
        out,
        "\n{}\n  {}  {}  {} chars",
        record.path,
        status_label(record.status),
        language,
        record.own_content_length
    )?;

    if !record.status.is_ok() {
        // The status already is the explanation; listing three empty kinds under it
        // would just be noise.
        return writeln!(out, "    {}", why_status(record.status));
    }

    for (kind, keywords) in record.by_kind() {
        if keywords.is_empty() {
            writeln!(out, "  {:<11} —      {}", label(kind), empty_reason(kind, result, cfg))?;
            continue;
        }
        for (i, k) in keywords.iter().enumerate() {
            let head = if i == 0 { label(kind) } else { "" };
            writeln!(
                out,
                "  {:<11} {:>5.3}  {:<width$}  x{:<3} {}",
                head,
                k.score,
                truncate(&k.surface, SURFACE_WIDTH),
                k.frequency,
                origin_note(k),
                width = SURFACE_WIDTH
            )?;
        }
    }
    Ok(())
}

/// Why this kind produced nothing. Extended as lanes land — the prose gate's verdict
/// replaces the placeholder below once Lane 3 exists.
fn empty_reason(kind: Kind, _result: &DocumentResult, cfg: &Config) -> String {
    match kind {
        Kind::Identifier => format!(
            "no candidate reached the identifier threshold of {:.2} (`kep explain` shows near-misses)",
            cfg.thresholds.identifier
        ),
        Kind::Technical => format!(
            "no candidate reached the technical threshold of {:.2} (`kep explain` shows near-misses)",
            cfg.thresholds.technical
        ),
        Kind::Topical => "lane not built yet".to_string(),
    }
}

fn why_status(status: &DocumentStatus) -> &'static str {
    match status {
        DocumentStatus::Ok => "",
        DocumentStatus::NoTextLayer => {
            "parsed cleanly but carries no text layer — almost certainly scanned; queue for OCR"
        }
        DocumentStatus::Encrypted => "encrypted; no password handling in this crate",
        DocumentStatus::UnsupportedFormat => "no parser for this format",
        DocumentStatus::ParseError(_) => "the parser rejected this file",
        DocumentStatus::TooShort => "no meaningful content at all",
    }
}

fn origin_note(k: &Keyword) -> String {
    let origin = format!("{:?}", k.origin).to_lowercase();
    match &k.expansion {
        Some(e) => format!("{origin} = {e}"),
        None => origin,
    }
}

pub fn status_label(status: &DocumentStatus) -> String {
    match status {
        DocumentStatus::ParseError(e) => format!("ParseError({e})"),
        other => format!("{other:?}"),
    }
}

fn label(kind: Kind) -> &'static str {
    match kind {
        Kind::Identifier => "Identifier",
        Kind::Technical => "Technical",
        Kind::Topical => "Topical",
    }
}

/// Fit a surface into its column.
///
/// Whitespace is collapsed first: a long form that straddled a line wrap keeps the
/// newline in its `surface`, faithfully, and printing that raw would tear the table in
/// half. The JSON keeps the real thing.
fn truncate(s: &str, width: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= width {
        return s;
    }
    let keep: String = s.chars().take(width.saturating_sub(1)).collect();
    format!("{keep}…")
}

/// Minimal RFC 4180 quoting. Surfaces contain commas and quotes often enough that
/// emitting them raw would produce a file no spreadsheet reads back correctly.
fn quote(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_keeps_the_column_width() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("aaaaaaaaaaaa", 5).chars().count(), 5);
    }

    #[test]
    fn a_wrapped_surface_does_not_tear_the_table_in_half() {
        assert_eq!(truncate("liquid\nchromatography", 40), "liquid chromatography");
    }

    #[test]
    fn csv_quoting_survives_a_surface_containing_a_comma() {
        assert_eq!(quote("a,b"), "\"a,b\"");
        assert_eq!(quote("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(quote("plain"), "plain");
    }
}
