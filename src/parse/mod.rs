//! Format parsing: raw bytes to raw text.
//!
//! Each format sits behind [`TextExtractor`] so a backend can be swapped without touching
//! anything downstream — which matters most for PDF, where the choice of extractor is the
//! largest single influence on output and the most likely thing to be revisited.
//!
//! OCR is deliberately out of scope. A scanned PDF is reported as
//! [`crate::DocumentStatus::NoTextLayer`] rather than silently yielding nothing.

mod email;
mod office;
mod pdf;
mod plain;

use crate::{config::Config, error::ExtractError, types::FormatHint};

pub use email::EmailExtractor;
pub use office::{DocxExtractor, PptxExtractor, XlsxExtractor};
pub use pdf::PdfExtractor;
pub use plain::PlainTextExtractor;

/// What kind of document the text came from. Drives canonicalisation (email needs quote
/// stripping) and the Stage 3 prose gate (spreadsheets are never prose).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Pdf,
    WordProcessing,
    Spreadsheet,
    Presentation,
    Email,
    Plain,
}

impl SourceKind {
    /// Formats that are structurally tabular. The prose gate rejects these outright
    /// rather than relying on a threshold that a wide enough sheet could sneak past.
    pub fn is_inherently_tabular(&self) -> bool {
        matches!(self, SourceKind::Spreadsheet)
    }
}

#[derive(Debug, Clone)]
pub struct RawText {
    pub text: String,
    pub source: SourceKind,
    /// `Some(false)` only where the format can distinguish "no text layer" from "empty":
    /// currently PDF alone. `None` means the question does not apply.
    pub text_layer_present: Option<bool>,
}

impl RawText {
    fn new(text: String, source: SourceKind) -> Self {
        Self { text, source, text_layer_present: None }
    }
}

pub trait TextExtractor {
    fn extract(&self, bytes: &[u8], cfg: &Config) -> Result<RawText, ExtractError>;
}

/// Identify the format from magic bytes and content shape.
///
/// Deliberately does not trust file extensions: the caller supplies bytes, and in a
/// heterogeneous corpus extensions are wrong often enough to matter.
pub fn sniff(bytes: &[u8]) -> FormatHint {
    if bytes.starts_with(b"%PDF-") {
        return FormatHint::Pdf;
    }
    // OLE compound file — legacy .doc/.xls/.msg. Only .msg is in scope.
    if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        return FormatHint::Email;
    }
    if bytes.starts_with(b"PK\x03\x04") {
        return sniff_ooxml(bytes);
    }
    if looks_like_email(bytes) {
        return FormatHint::Email;
    }
    FormatHint::PlainText
}

/// Distinguish OOXML flavours by which part names the container holds.
fn sniff_ooxml(bytes: &[u8]) -> FormatHint {
    let cursor = std::io::Cursor::new(bytes);
    let Ok(mut zip) = zip::ZipArchive::new(cursor) else {
        return FormatHint::PlainText;
    };
    let mut hint = FormatHint::PlainText;
    for i in 0..zip.len() {
        let Ok(f) = zip.by_index_raw(i) else { continue };
        let name = f.name();
        if name.starts_with("word/") {
            return FormatHint::Docx;
        } else if name.starts_with("xl/") {
            return FormatHint::Xlsx;
        } else if name.starts_with("ppt/") {
            return FormatHint::Pptx;
        }
        if name == "[Content_Types].xml" {
            hint = FormatHint::PlainText;
        }
    }
    hint
}

/// RFC 5322 messages begin with headers. Check the first few lines for the ones that a
/// stored message essentially always carries.
fn looks_like_email(bytes: &[u8]) -> bool {
    const MARKERS: &[&str] = &[
        "from:", "received:", "return-path:", "message-id:", "subject:", "date:", "to:",
        "mime-version:", "delivered-to:",
    ];
    let head = &bytes[..bytes.len().min(2048)];
    let Ok(text) = std::str::from_utf8(head) else { return false };

    let mut hits = 0;
    for line in text.lines().take(12) {
        let lower = line.to_ascii_lowercase();
        if MARKERS.iter().any(|m| lower.starts_with(m)) {
            hits += 1;
        }
    }
    hits >= 2
}

/// Resolve a hint to a backend.
pub fn extractor_for(hint: FormatHint) -> Result<Box<dyn TextExtractor>, ExtractError> {
    Ok(match hint {
        FormatHint::Pdf => Box::new(PdfExtractor),
        FormatHint::Docx => Box::new(DocxExtractor),
        FormatHint::Pptx => Box::new(PptxExtractor),
        FormatHint::Xlsx => Box::new(XlsxExtractor),
        FormatHint::Email => Box::new(EmailExtractor),
        FormatHint::Csv | FormatHint::PlainText | FormatHint::Markdown => {
            Box::new(PlainTextExtractor)
        }
        FormatHint::Sniff => return Err(ExtractError::UnsupportedFormat("unresolved".into())),
    })
}

/// Parse bytes to raw text, sniffing the format when the caller did not supply one.
pub fn parse(bytes: &[u8], hint: FormatHint, cfg: &Config) -> Result<RawText, ExtractError> {
    let resolved = if hint == FormatHint::Sniff { sniff(bytes) } else { hint };
    extractor_for(resolved)?.extract(bytes, cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_pdf_by_magic() {
        assert_eq!(sniff(b"%PDF-1.7\n..."), FormatHint::Pdf);
    }

    #[test]
    fn sniffs_email_by_headers() {
        let raw = b"From: a@example.com\r\nSubject: Batch DS-2291\r\n\r\nBody text.";
        assert_eq!(sniff(raw), FormatHint::Email);
    }

    #[test]
    fn a_single_header_like_line_is_not_an_email() {
        // Prose can open with something that looks like a header; two markers are needed.
        assert_eq!(sniff(b"Subject: the meeting\n\nand then a lot of prose."), FormatHint::PlainText);
    }

    #[test]
    fn falls_back_to_plain_text() {
        assert_eq!(sniff(b"just some words"), FormatHint::PlainText);
    }

    #[test]
    fn handles_empty_input_without_panicking() {
        assert_eq!(sniff(b""), FormatHint::PlainText);
    }
}
