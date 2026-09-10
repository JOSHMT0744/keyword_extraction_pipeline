//! Deterministic language detection.
//!
//! English is the only language whose full lane set runs. Detection exists so that a
//! non-English document *degrades visibly* — Lane 1 still runs, since shape and
//! identifiers are language-independent, but the result records that the topical and
//! definitional lanes did not. Silent degradation is the failure this crate is built to
//! avoid, and "we never extracted topics from the French documents" is exactly the kind
//! of omission that would otherwise never announce itself.
//!
//! The detector carries a small set of European languages purely to *discriminate*. A
//! single-language detector would report English for every document with full confidence,
//! which is not detection.

use std::sync::OnceLock;

use lingua::{Language as LinguaLanguage, LanguageDetector, LanguageDetectorBuilder};

use crate::types::Language;

/// Languages the detector can tell apart. Adding one changes results, so this list is
/// part of the extraction contract and a change to it warrants a logic-revision bump.
const CANDIDATES: &[LinguaLanguage] = &[
    LinguaLanguage::English,
    LinguaLanguage::French,
    LinguaLanguage::German,
    LinguaLanguage::Spanish,
    LinguaLanguage::Italian,
    LinguaLanguage::Dutch,
    LinguaLanguage::Portuguese,
];

/// Below this many characters, detection is guesswork. A short identifier-dense document
/// is reported as undetermined rather than assigned a language it does not have.
const MIN_CHARS_FOR_DETECTION: usize = 40;

fn detector() -> &'static LanguageDetector {
    static DETECTOR: OnceLock<LanguageDetector> = OnceLock::new();
    DETECTOR.get_or_init(|| {
        LanguageDetectorBuilder::from_languages(CANDIDATES)
            .with_preloaded_language_models()
            .build()
    })
}

/// Returns the detected language and the detector's confidence in it.
///
/// `None` for text too short to judge. The caller treats that as "run Lane 1 only",
/// the same as a confidently non-English result.
pub fn detect(text: &str) -> (Option<Language>, f32) {
    if text.chars().count() < MIN_CHARS_FOR_DETECTION {
        return (None, 0.0);
    }

    let Some(detected) = detector().detect_language_of(text) else {
        return (None, 0.0);
    };
    let confidence = detector().compute_language_confidence(text, detected) as f32;

    let code = match detected {
        LinguaLanguage::English => "en",
        LinguaLanguage::French => "fr",
        LinguaLanguage::German => "de",
        LinguaLanguage::Spanish => "es",
        LinguaLanguage::Italian => "it",
        LinguaLanguage::Dutch => "nl",
        LinguaLanguage::Portuguese => "pt",
        // Deliberately no wildcard arm: the enum is narrowed by the cargo features above,
        // so adding a language becomes a compile error here rather than a silent "und".
    };

    (
        Some(Language {
            code: code.to_string(),
            fully_supported: detected == LinguaLanguage::English,
        }),
        confidence,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_english_as_fully_supported() {
        let text = "The column was regenerated according to the standard operating \
                    procedure and the batch was released after review.";
        let (lang, conf) = detect(text);
        let lang = lang.expect("english should be detected");
        assert_eq!(lang.code, "en");
        assert!(lang.fully_supported);
        assert!(conf > 0.5, "confidence was {conf}");
    }

    #[test]
    fn marks_non_english_as_degraded_rather_than_failing() {
        let text = "La colonne a été régénérée conformément à la procédure standard \
                    et le lot a été libéré après examen par le responsable qualité.";
        let (lang, _) = detect(text);
        let lang = lang.expect("french should be detected");
        assert_eq!(lang.code, "fr");
        assert!(
            !lang.fully_supported,
            "non-English must be recorded as degraded, not silently treated as English"
        );
    }

    #[test]
    fn short_text_is_undetermined_rather_than_guessed() {
        assert_eq!(detect("DS-2291").0, None);
    }

    #[test]
    fn is_deterministic() {
        let text = "The chromatography step achieved the expected yield in every run.";
        assert_eq!(detect(text).0, detect(text).0);
    }
}
