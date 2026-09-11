//! Tunable parameters. Every field feeds [`crate::PipelineVersion`] in a fixed order, so
//! changing any of them changes the version stamp and invalidates cached keyword sets.

use serde::{Deserialize, Serialize};

/// Parameters for the deterministic prose heuristic that gates Stage 3.
///
/// Without it, spreadsheets emit column headers as topics and emails emit footers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProseParams {
    /// Minimum mean sentence length in tokens.
    pub min_mean_sentence_len: f32,
    /// Minimum ratio of stopwords to all tokens. Prose is full of function words;
    /// tabular output is not.
    pub min_stopword_ratio: f32,
    /// Maximum ratio of lines that look like table or field rows.
    pub max_table_line_ratio: f32,
    /// Minimum tokens before the topical stage is worth running at all.
    pub min_tokens: usize,
}

impl Default for ProseParams {
    fn default() -> Self {
        Self {
            min_mean_sentence_len: 8.0,
            min_stopword_ratio: 0.20,
            max_table_line_ratio: 0.40,
            min_tokens: 120,
        }
    }
}

/// Per-kind emission thresholds.
///
/// Output is uncapped above these. A single global cutoff cannot serve both shape scores
/// and YAKE scores — they are on unrelated scales with different distributions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Thresholds {
    pub identifier: f32,
    pub technical: f32,
    /// YAKE scores are *lower is better*; this is an upper bound.
    pub topical: f32,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            identifier: 0.55,
            // Just below the weight of `absent_from_wordlist` alone, so a technical term
            // whose only evidence is being absent from general English still surfaces.
            technical: 0.38,
            topical: 0.15,
        }
    }
}

/// Weights for Stage 1's feature vector.
///
/// A transparent weighted sum with retained components, deliberately not a classifier:
/// there are no labels, and a learned model would forfeit the reproducibility that
/// motivates the whole design.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShapeWeights {
    pub internal_caps: f32,
    pub digit_letter_mix: f32,
    pub separator_segments: f32,
    pub unusual_length: f32,
    pub absent_from_wordlist: f32,
    pub short_all_caps: f32,
    pub in_document_frequency: f32,
}

impl Default for ShapeWeights {
    fn default() -> Self {
        // Absence from general English dominates because for a purely alphabetic
        // technical term it is the *only* available signal: `chromatography` has no
        // digits, no separators and no internal capitals. Any weighting that leaves
        // absence below the technical threshold makes single-word technical terms
        // unfindable in principle, whatever the threshold is set to.
        Self {
            internal_caps: 0.12,
            digit_letter_mix: 0.25,
            separator_segments: 0.12,
            unusual_length: 0.03,
            absent_from_wordlist: 0.40,
            short_all_caps: 0.06,
            in_document_frequency: 0.02,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Documents shorter than this return [`crate::DocumentStatus::TooShort`].
    ///
    /// Deliberately near-zero. It means "there is nothing here at all", *not* "too short
    /// to be worth reading": an instrument report whose entire content is forty sample
    /// codes is short in characters and is exactly the document the identifier stage
    /// exists for. Gating it out here would discard the crate's highest-value case.
    /// "Too short for topical keyphrases" is a different question, answered by
    /// [`ProseParams::min_tokens`].
    pub min_content_length: usize,
    /// A PDF yielding fewer than this many characters is treated as having no text
    /// layer rather than as an empty document.
    pub no_text_layer_threshold: usize,
    /// How many of the frequency-ranked wordlist entries count as ordinary English.
    ///
    /// The single scalar governing Stage 1's `absent_from_wordlist` feature. Measured
    /// separation in the embedded list puts ordinary formal vocabulary above ~60k
    /// (`specification` 60k) and domain vocabulary below (`chromatography` 87k), so the
    /// default sits between them. Intended to be swept by the injection instrument
    /// rather than argued about.
    pub wordlist_size: usize,
    pub thresholds: Thresholds,
    pub shape_weights: ShapeWeights,
    pub prose: ProseParams,
    /// Strip quoted blocks and signatures from email. Mandatory in practice: a
    /// forty-message thread otherwise inflates term frequency and boilerplate footers
    /// read as ubiquitous.
    pub strip_quoted_blocks: bool,
    /// Run Stage 2 (Schwartz–Hearst definitions).
    pub enable_definitions: bool,
    /// Run Stage 3 (YAKE), subject to the prose gate.
    pub enable_topical: bool,
    pub yake_ngram_max: usize,
    /// Retain Stage 1's feature vector on every emitted keyword.
    ///
    /// Debug and tuning only, and **deliberately absent from [`Config::feed`]**: it
    /// cannot change *which* keywords are emitted, only how much is reported about
    /// them. Folding it into the version stamp would invalidate every cached keyword
    /// set the moment someone ran `kep explain`.
    pub retain_features: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            min_content_length: 16,
            no_text_layer_threshold: 32,
            wordlist_size: 65_000,
            thresholds: Thresholds::default(),
            shape_weights: ShapeWeights::default(),
            prose: ProseParams::default(),
            strip_quoted_blocks: true,
            enable_definitions: true,
            enable_topical: true,
            yake_ngram_max: 3,
            retain_features: false,
        }
    }
}

impl Config {
    /// Feed every field into a hasher in a fixed, explicit order.
    ///
    /// Deliberately not derived from serde: field ordering in a serialised form is an
    /// implementation detail of the serialiser, and the version stamp must not depend on
    /// one. Adding a field here without updating this method is a bug that would let
    /// behaviour change without the version changing.
    pub(crate) fn feed(&self, h: &mut blake3::Hasher) {
        let Config {
            min_content_length,
            no_text_layer_threshold,
            wordlist_size,
            thresholds,
            shape_weights,
            prose,
            strip_quoted_blocks,
            enable_definitions,
            enable_topical,
            yake_ngram_max,
            // Reporting-only; see the field's documentation. Bound explicitly rather
            // than by `..` so a genuinely behavioural field added later cannot slip
            // through unfed.
            retain_features: _,
        } = self;

        h.update(&(*min_content_length as u64).to_le_bytes());
        h.update(&(*no_text_layer_threshold as u64).to_le_bytes());
        h.update(&(*wordlist_size as u64).to_le_bytes());

        for f in [thresholds.identifier, thresholds.technical, thresholds.topical] {
            h.update(&f.to_le_bytes());
        }
        for f in [
            shape_weights.internal_caps,
            shape_weights.digit_letter_mix,
            shape_weights.separator_segments,
            shape_weights.unusual_length,
            shape_weights.absent_from_wordlist,
            shape_weights.short_all_caps,
            shape_weights.in_document_frequency,
        ] {
            h.update(&f.to_le_bytes());
        }
        for f in [
            prose.min_mean_sentence_len,
            prose.min_stopword_ratio,
            prose.max_table_line_ratio,
        ] {
            h.update(&f.to_le_bytes());
        }
        h.update(&(prose.min_tokens as u64).to_le_bytes());

        h.update(&[
            *strip_quoted_blocks as u8,
            *enable_definitions as u8,
            *enable_topical as u8,
        ]);
        h.update(&(*yake_ngram_max as u64).to_le_bytes());
    }
}
