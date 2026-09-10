//! Born-digital PDF text extraction via `pdf_oxide`, pinned to an exact version.
//!
//! The pin is not caution for its own sake: `pdf_oxide` ships roughly a release every four
//! days with no output-stability commitment, and any change to extracted text changes
//! every keyword set derived from it. The exact version feeds `PipelineVersion`, so an
//! upgrade is a deliberate re-extraction event rather than a silent drift.

use pdf_oxide::PdfDocument;

use super::{RawText, SourceKind, TextExtractor};
use crate::{config::Config, error::ExtractError};

pub struct PdfExtractor;

impl TextExtractor for PdfExtractor {
    fn name(&self) -> &'static str {
        "pdf_oxide"
    }

    fn extract(&self, bytes: &[u8], cfg: &Config) -> Result<RawText, ExtractError> {
        let doc = PdfDocument::from_bytes(bytes.to_vec())
            .map_err(|e| ExtractError::Parse(format!("pdf: {e}")))?;

        if doc.is_encrypted() {
            // An encrypted document may still decrypt with an empty owner password;
            // `from_bytes` having succeeded means it did. Only report Encrypted when no
            // text can be reached at all, which the page loop below determines.
            if doc.page_count().unwrap_or(0) == 0 {
                return Err(ExtractError::Encrypted);
            }
        }

        let pages = doc
            .page_count()
            .map_err(|e| ExtractError::Parse(format!("pdf page count: {e}")))?;

        // Ask the document itself whether a text layer exists before trusting an empty
        // extraction result. A scanned page extracts to "" exactly as a blank page does,
        // and conflating them is the silent omission this crate exists to avoid.
        let mut any_text_layer = false;
        let mut text = String::new();

        let options = pdf_oxide::converters::ConversionOptions {
            // XY-Cut column detection. `StructureTreeFirst` would be better on tagged
            // PDFs but requires supplying the MCID order manually, and falls back to
            // exactly this when a document is untagged.
            reading_order_mode: pdf_oxide::converters::ReadingOrderMode::ColumnAware,
            extract_tables: true,
            include_images: false,
            embed_images: false,
            strip_running_headers_footers: true,
            ..Default::default()
        };

        for page in 0..pages {
            if doc.has_text_layer(page).unwrap_or(true) {
                any_text_layer = true;
            }
            match doc.extract_text_with_options(page, &options) {
                Ok(t) => {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(&t);
                }
                // One unreadable page in a long document should not discard the rest.
                Err(_) => continue,
            }
        }

        let meaningful = text.trim().chars().count() >= cfg.no_text_layer_threshold;
        Ok(RawText {
            text,
            source: SourceKind::Pdf,
            text_layer_present: Some(any_text_layer && meaningful),
        })
    }
}
