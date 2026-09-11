use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::{stages::shape::ShapeFeatures, prose::ProseVerdict, serde_hex, version::PipelineVersion};

/// Which parser to use. Callers that know the format should say so; `Sniff` falls back
/// to content inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormatHint {
    Sniff,
    Pdf,
    Docx,
    Pptx,
    Xlsx,
    Csv,
    Email,
    PlainText,
    Markdown,
}

/// What kind of thing a keyword is.
///
/// Scores are comparable *within* a kind and meaningless across kinds — shape scores and
/// YAKE scores are on unrelated scales. Consumers wanting a single list interleave by
/// quota rather than by score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    /// Digit-bearing or coded tokens: `DS-2291`, `HEK293T`, `SOP-114`.
    Identifier,
    /// Forms absent from the general English wordlist: `MabSelect SuRe`, `chromatography`.
    Technical,
    /// Topical keyphrases made of ordinary words: `column regeneration`.
    Topical,
}

/// Which stage produced a keyword. Affects where it ranks, never whether it exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Origin {
    /// Stage 1 — shape features and wordlist absence.
    Shape,
    /// Stage 2 — Schwartz–Hearst definitional context. Higher confidence prior.
    Definition,
    /// Stage 3 — YAKE statistical keyphrase extraction.
    Statistic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keyword {
    /// The keyword as it is actually written in the document.
    ///
    /// Known as the *surface form* in the literature; named for what it is here, because
    /// this is the field a reader of the output sees first.
    ///
    /// One `Keyword` covers one distinct `normalised` form, so a document containing both
    /// `Chromatography` and `chromatography` yields a single record with three offsets,
    /// not two records — they are one finding. This field holds the first variant seen,
    /// which means **it is not guaranteed to equal the text at every offset**. The
    /// invariant that does hold is on `normalised`: for every span in `offsets`,
    /// `canonical[span]`, lowercased with internal whitespace collapsed, equals
    /// `normalised`. Anything highlighting occurrences should use the offsets; anything
    /// matching should use `normalised`.
    pub original_keyword: String,
    /// NFKC + casefold. Never stemmed — stemming mangles alphanumeric identifiers.
    pub normalised: String,
    pub kind: Kind,
    pub origin: Origin,
    /// Comparable only against other keywords of the same `kind`.
    pub score: f32,
    /// Rank within `kind`, 0-based. Lets a consumer impose its own top-N without
    /// re-extracting, since output is uncapped above a per-kind threshold.
    pub rank: u32,
    pub frequency: u32,
    /// Byte offsets into the canonical text, which [`crate::canonicalise`] regenerates.
    pub offsets: Vec<Range<usize>>,
    /// The canonical expansion, when a definition stage resolved one: `SOP` carries
    /// `Standard Operating Procedure`. Only ever set for [`Origin::Definition`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expansion: Option<String>,
    /// Stage 1's retained feature vector, present only when [`crate::Config`]'s
    /// `retain_features` is on. The plan calls Stage 1 "a transparent weighted sum with
    /// stored components"; this is where the components are stored, so a score can be
    /// accounted for rather than taken on trust.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<ShapeFeatures>,
}

/// Why a document produced the keywords it did — including none.
///
/// `NoTextLayer` is load-bearing. A scanned PDF parses successfully and yields nothing,
/// and an empty keyword list would be indistinguishable from a document with nothing
/// distinctive in it. That is the silent omission this enum exists to prevent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocumentStatus {
    Ok,
    /// Parsed cleanly but has no meaningful text layer — almost always a scanned PDF.
    /// The consumer can queue these for OCR, which is out of scope here.
    NoTextLayer,
    Encrypted,
    UnsupportedFormat,
    ParseError(String),
    /// Below the configured minimum length for extraction to mean anything.
    TooShort,
}

impl DocumentStatus {
    /// Whether keywords are expected to be present and meaningful.
    pub fn is_ok(&self) -> bool {
        matches!(self, DocumentStatus::Ok)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Language {
    pub code: String,
    /// True when the full stage set ran. Non-English documents degrade to Stage 1 only.
    pub fully_supported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentResult {
    pub status: DocumentStatus,
    pub pipeline_version: PipelineVersion,
    /// blake3 over the raw input bytes. Serialised as lowercase hex.
    #[serde(with = "serde_hex")]
    pub hash_exact: [u8; 32],
    /// blake3 over the canonical text. The natural cache key for everything downstream.
    #[serde(with = "serde_hex::option")]
    pub hash_canonical: Option<[u8; 32]>,
    /// Characters of canonical text.
    pub own_content_length: usize,
    pub language: Option<Language>,
    pub language_confidence: f32,
    /// Flat, ranked, uncapped above a per-kind threshold. Filter on `kind`.
    pub keywords: Vec<Keyword>,
    /// The prose gate's ruling on this document, and the measurements behind it.
    ///
    /// Carried rather than consumed so an empty `Topical` list can be reported *with its
    /// reason*. "No topical keywords" and "no topical keywords, because this is a
    /// spreadsheet" are different statements, and only the second is actionable.
    /// `None` when the document never reached the gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prose: Option<ProseVerdict>,
}

impl DocumentResult {
    /// A result for a document that could not be read. Carries the hash and version so
    /// the consumer can still record and deduplicate it.
    pub fn failed(
        status: DocumentStatus,
        pipeline_version: PipelineVersion,
        hash_exact: [u8; 32],
    ) -> Self {
        Self {
            status,
            pipeline_version,
            hash_exact,
            hash_canonical: None,
            own_content_length: 0,
            language: None,
            language_confidence: 0.0,
            keywords: Vec::new(),
            prose: None,
        }
    }
}
