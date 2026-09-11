//! RFC 5322 email.
//!
//! Only the subject and text bodies are taken. Headers beyond the subject are routing
//! metadata: they contain addresses and message-ids that are identifier-shaped and would
//! swamp the identifier stage with content nobody would ever search for.
//!
//! Quoted blocks and signatures are stripped in canonicalisation rather than here, so a
//! caller regenerating canonical text to resolve offsets gets identical treatment.

use mail_parser::MessageParser;

use super::{RawText, SourceKind, TextExtractor};
use crate::{config::Config, error::ExtractError};

pub struct EmailExtractor;

impl TextExtractor for EmailExtractor {
    fn name(&self) -> &'static str {
        "mail-parser"
    }

    fn extract(&self, bytes: &[u8], _cfg: &Config) -> Result<RawText, ExtractError> {
        let msg = MessageParser::default()
            .parse(bytes)
            .ok_or_else(|| ExtractError::Parse("email: unparseable".into()))?;

        let mut text = String::new();
        if let Some(subject) = msg.subject() {
            text.push_str(subject);
            text.push('\n');
        }
        for i in 0..msg.text_body_count() {
            if let Some(body) = msg.body_text(i) {
                text.push_str(&body);
                text.push('\n');
            }
        }

        // A message whose only body is HTML yields nothing above. Treat it as empty
        // rather than guessing: HTML-to-text is its own problem with its own version.
        Ok(RawText::new(text, SourceKind::Email))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_subject_and_body_but_not_routing_headers() {
        let raw = b"From: alice@example.com\r\n\
                    Message-ID: <ABC123XYZ@mail.example.com>\r\n\
                    Subject: Batch DS-2291 release\r\n\
                    \r\n\
                    The column was regenerated.\r\n";
        let out = EmailExtractor.extract(raw, &Config::default()).unwrap();

        assert!(out.text.contains("DS-2291"));
        assert!(out.text.contains("column was regenerated"));
        assert!(
            !out.text.contains("ABC123XYZ"),
            "message-id is identifier-shaped noise and must not reach the stages: {:?}",
            out.text
        );
        assert_eq!(out.source, SourceKind::Email);
    }
}
