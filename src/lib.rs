//! Deterministic tier-1 keyword and identifier extraction.
//!
//! Raw file in, ranked keywords out. The crate is **corpus-blind**: every score derives
//! from the document itself plus pinned static resources. There is no document-frequency
//! table, no IDF, and no cross-document state anywhere, so a document's keyword set is a
//! pure function of `(bytes, config, resources)` — summarised by [`PipelineVersion`].
//!
//! Corpus-relative ranking, storage, triage scoring and blurb scheduling all belong to
//! the consuming system.
//!
//! # What this crate cannot tell you
//!
//! Nothing measured here is evidence that keyword-based shortlisting reduces blurb spend.
//! That is an extrinsic property of the consuming system, measurable only with real
//! documents and a real query matcher. This crate is evaluated intrinsically: did it
//! extract the right things, reproducibly.

pub mod canonical;
pub mod config;
pub mod error;
pub mod language;
pub mod parse;
pub mod prose;
pub mod resources;
pub mod stages;
pub mod tokenize;
pub mod types;
pub mod version;

mod serde_hex;

pub use config::Config;
pub use error::ExtractError;
pub use resources::Resources;
pub use types::{
    DocumentResult, DocumentStatus, FormatHint, Keyword, Kind, Language, Origin,
};
pub use version::PipelineVersion;

/// Extract keywords from a document.
///
/// Pure function of `(bytes, cfg, res)`. The same bytes under the same
/// [`PipelineVersion`] always produce the same keyword set — that guarantee is what the
/// version stamp exists to make checkable, and what the snapshot suite enforces.
///
/// Never returns an `Err` for a document that merely cannot be read: an unreadable
/// document is a *result* with a [`DocumentStatus`] explaining why, so that the consumer
/// records it rather than losing it. `Err` is reserved for a caller mistake.
pub fn extract(bytes: &[u8], hint: FormatHint, cfg: &Config, res: &Resources) -> DocumentResult {
    let version = PipelineVersion::compute(cfg, res);
    let hash_exact = *blake3::hash(bytes).as_bytes();

    let raw = match parse::parse(bytes, hint, cfg) {
        Ok(raw) => raw,
        Err(e) => {
            let status = match e {
                ExtractError::Encrypted => DocumentStatus::Encrypted,
                ExtractError::UnsupportedFormat(_) => DocumentStatus::UnsupportedFormat,
                other => DocumentStatus::ParseError(other.to_string()),
            };
            return DocumentResult::failed(status, version, hash_exact);
        }
    };

    // Asked before length, because a scanned PDF is short *and* has no text layer, and
    // reporting it as merely short would hide the fact that OCR could recover it.
    if raw.text_layer_present == Some(false) {
        return DocumentResult::failed(DocumentStatus::NoTextLayer, version, hash_exact);
    }

    let canonical = canonical::canonicalise(&raw.text, raw.source, cfg);
    let own_content_length = canonical.chars().count();
    let hash_canonical = Some(*blake3::hash(canonical.as_bytes()).as_bytes());

    if own_content_length < cfg.min_content_length {
        let mut out = DocumentResult::failed(DocumentStatus::TooShort, version, hash_exact);
        out.hash_canonical = hash_canonical;
        out.own_content_length = own_content_length;
        return out;
    }

    let (language, language_confidence) = language::detect(&canonical);
    let english = language.as_ref().is_some_and(|l| l.fully_supported);
    let prose = prose::assess(&canonical, raw.source, english, cfg, res);

    DocumentResult {
        status: DocumentStatus::Ok,
        pipeline_version: version,
        hash_exact,
        hash_canonical,
        own_content_length,
        language,
        language_confidence,
        keywords: run_stages(&canonical, &prose, cfg, res),
        prose: Some(prose),
    }
}

/// Run every enabled stage and rank the union.
///
/// Stages are **complements, not substitutes** — Stage 1 emits identifiers and technical
/// vocabulary, Stage 3 emits topical keyphrases — so their outputs are concatenated
/// rather than selected between, and ranked once at the end. Ranking here rather than
/// inside each stage is what keeps `rank` dense within a kind: two stages ranking
/// themselves would each start at 0 and collide.
///
/// Stages may legitimately emit the same term from different evidence. Those are kept
/// as separate records distinguished by `origin`, not merged: an orthographic guess and
/// a definitional match are different claims, and collapsing them would discard which
/// one was made. A consumer wanting uniqueness deduplicates on `(normalised, kind)`.
fn run_stages(
    canonical: &str,
    prose: &prose::ProseVerdict,
    cfg: &Config,
    res: &Resources,
) -> Vec<Keyword> {
    let mut keywords = stages::shape::extract(canonical, cfg, res);

    // Runs whatever the language. Schwartz–Hearst matches orthography, not vocabulary:
    // `Bundesamt für Sicherheit (BSI)` resolves without a word of English. Stage 3 is the
    // one that depends on an English stopword list, and it is gated accordingly.
    if cfg.enable_definitions {
        keywords.extend(stages::definition::extract(canonical, cfg, res));
    }

    // Gated, unlike Stage 2. YAKE's features are computed against an English stopword
    // list and English sentence rhythm; run on a spreadsheet it returns column headers
    // and run on German it returns confident nonsense. Both are worse than nothing,
    // because nothing is visibly nothing. The gate's reasoning travels with the result
    // so an empty topical list can say why it is empty.
    if cfg.enable_topical && prose.is_prose {
        keywords.extend(stages::topical::extract(canonical, cfg, res));
    }

    stages::shape::rank_within_kind(&mut keywords);
    keywords
}

/// Regenerate the canonical text a result's offsets refer to.
///
/// Kept separate from [`extract`] so full document text is never returned by default.
/// Canonicalisation is idempotent and deterministic, so the string returned here is
/// byte-identical to the one the offsets were computed against.
pub fn canonicalise(
    bytes: &[u8],
    hint: FormatHint,
    cfg: &Config,
) -> Result<String, ExtractError> {
    let raw = parse::parse(bytes, hint, cfg)?;
    Ok(canonical::canonicalise(&raw.text, raw.source, cfg))
}
