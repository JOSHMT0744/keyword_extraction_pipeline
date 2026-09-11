//! Plain text, Markdown and CSV.
//!
//! Encoding is decided here rather than in canonicalisation, because a mis-decoded
//! document produces plausible-looking rubbish that no later stage can detect.

use super::{RawText, SourceKind, TextExtractor};
use crate::{config::Config, error::ExtractError};

pub struct PlainTextExtractor;

impl TextExtractor for PlainTextExtractor {
    fn extract(&self, bytes: &[u8], _cfg: &Config) -> Result<RawText, ExtractError> {
        let text = decode(bytes);
        Ok(RawText::new(text, SourceKind::Plain))
    }
}

/// Decode as UTF-8, stripping a BOM; fall back to Latin-1, which cannot fail and is the
/// common case for legacy plain-text files in an old corpus.
fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_utf8_bom() {
        assert_eq!(decode(b"\xEF\xBB\xBFhello"), "hello");
    }

    #[test]
    fn falls_back_to_latin1_rather_than_failing() {
        // 0xE9 is invalid UTF-8 but valid Latin-1 'é'.
        assert_eq!(decode(b"caf\xE9"), "café");
    }
}
