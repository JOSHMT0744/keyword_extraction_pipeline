//! The CLI's wire format.
//!
//! **This is a presentation format, not a serde round-trip of `DocumentResult`.**
//! `serde_json::from_str::<DocumentResult>` will not accept it, because keywords are
//! grouped by kind here and flat in the library type. That is deliberate: the library
//! contract is a flat ranked list so a consumer can stream and filter it, while a person
//! or a downstream store reading this JSON wants the three kinds separated — scores are
//! meaningless across kinds, and grouping makes that structural rather than documentary.
//!
//! The grouping happens here, borrowing from the result, so the library type is untouched.

use keyword_extraction_pipeline::{
    types::Language, DocumentResult, DocumentStatus, Keyword, Kind, PipelineVersion,
};
use serde::Serialize;

#[derive(Serialize)]
pub struct DocumentRecord<'a> {
    pub path: String,
    pub status: &'a DocumentStatus,
    pub pipeline_version: &'a PipelineVersion,
    pub hash_exact: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash_canonical: Option<String>,
    pub own_content_length: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<&'a Language>,
    pub language_confidence: f32,
    pub keywords: KeywordsByKind<'a>,
}

/// Keywords split by kind, each already ranked. Never one list: a consumer that
/// concatenated them would be sorting shape scores against YAKE scores.
#[derive(Serialize)]
pub struct KeywordsByKind<'a> {
    pub identifier: Vec<&'a Keyword>,
    pub technical: Vec<&'a Keyword>,
    pub topical: Vec<&'a Keyword>,
}

/// What the caller asked to see. Applied at render time only — filtering never changes
/// what was extracted, so `rank` still refers to the full uncapped set.
#[derive(Clone, Copy)]
pub struct View {
    pub kinds: Option<[bool; 3]>,
    pub top: Option<usize>,
}

impl View {
    fn wants(&self, kind: Kind) -> bool {
        self.kinds.is_none_or(|k| k[kind as usize])
    }

    fn take<'a>(&self, result: &'a DocumentResult, kind: Kind) -> Vec<&'a Keyword> {
        if !self.wants(kind) {
            return Vec::new();
        }
        let it = result.keywords.iter().filter(move |k| k.kind == kind);
        match self.top {
            Some(n) => it.take(n).collect(),
            None => it.collect(),
        }
    }
}

impl<'a> DocumentRecord<'a> {
    pub fn new(path: &str, result: &'a DocumentResult, view: View) -> Self {
        Self {
            path: path.to_string(),
            status: &result.status,
            pipeline_version: &result.pipeline_version,
            hash_exact: hex(&result.hash_exact),
            hash_canonical: result.hash_canonical.as_ref().map(hex),
            own_content_length: result.own_content_length,
            language: result.language.as_ref(),
            language_confidence: result.language_confidence,
            keywords: KeywordsByKind {
                identifier: view.take(result, Kind::Identifier),
                technical: view.take(result, Kind::Technical),
                topical: view.take(result, Kind::Topical),
            },
        }
    }

    pub fn by_kind(&self) -> [(Kind, &[&'a Keyword]); 3] {
        [
            (Kind::Identifier, &self.keywords.identifier),
            (Kind::Technical, &self.keywords.technical),
            (Kind::Topical, &self.keywords.topical),
        ]
    }
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
