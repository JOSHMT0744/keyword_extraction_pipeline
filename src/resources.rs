//! Static resources: the general English wordlist and stopword list.
//!
//! Defaults are compiled in, so they cannot drift and need no deployment step. An optional
//! runtime override is supported; its digest folds into [`crate::PipelineVersion`] exactly
//! as the embedded digest does, so an override can never change output silently.
//!
//! # Why the wordlist is frequency-ranked
//!
//! Stage 1 asks "is this an ordinary English word". The answer is a matter of degree, and
//! the cutoff trades recall against precision directly: too inclusive a list makes genuine
//! technical terms look ordinary and suppresses them; too small a list flags ordinary
//! words as technical. Ranking by frequency turns that trade-off into a single scalar,
//! [`crate::Config::wordlist_size`], which the injection instrument can sweep rather than
//! anyone having to argue for a value.
//!
//! The list is deliberately **general** English, never domain text. The corpus is
//! heterogeneous and must not be assumed scientific.

use std::{collections::HashSet, fs, path::Path};

use crate::{config::Config, error::ExtractError};

const DEFAULT_WORDLIST: &str = include_str!("../resources/wordlist.txt");
const DEFAULT_STOPWORDS: &str = include_str!("../resources/stopwords.txt");

#[derive(Debug, Clone)]
pub struct Resources {
    /// Words at or above the configured frequency rank. Membership means "ordinary".
    common: HashSet<String>,
    stopwords: HashSet<String>,
    /// How many of the ranked words are active. Retained for diagnostics.
    cutoff: usize,
    /// Total available depth, so a config asking for more than exists can be reported.
    depth: usize,
    pub(crate) wordlist_digest: [u8; 32],
    pub(crate) stopwords_digest: [u8; 32],
}

/// Strip comments and blank lines, preserving order — for the wordlist, order is rank.
///
/// Entries are split on whitespace rather than taken a line at a time. The wordlist is
/// strictly one token per line, so this is a no-op there and rank order is untouched; the
/// stopword list is written as space-separated runs, and reading *it* a line at a time
/// silently produced eighteen multi-word entries that no lookup could ever match. That
/// made `is_stopword` return false for every English stopword, including "the".
fn entries(text: &str) -> impl Iterator<Item = String> + '_ {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .flat_map(str::split_whitespace)
        .map(str::to_lowercase)
}

impl Default for Resources {
    /// Resources under [`Config::default`]. Use [`Resources::for_config`] when the config
    /// is not the default, or the active cutoff will not match the version stamp.
    fn default() -> Self {
        Self::build(DEFAULT_WORDLIST, DEFAULT_STOPWORDS, &Config::default())
    }
}

impl Resources {
    pub fn for_config(cfg: &Config) -> Self {
        Self::build(DEFAULT_WORDLIST, DEFAULT_STOPWORDS, cfg)
    }

    /// Load overrides from disk. Either file may be omitted to keep the embedded default.
    /// An override wordlist must also be frequency-ranked, most frequent first.
    pub fn with_overrides(
        cfg: &Config,
        wordlist: Option<&Path>,
        stopwords: Option<&Path>,
    ) -> Result<Self, ExtractError> {
        let read = |p: &Path| {
            fs::read_to_string(p).map_err(|e| ExtractError::Resource(format!("{}: {e}", p.display())))
        };
        let wl = match wordlist {
            Some(p) => read(p)?,
            None => DEFAULT_WORDLIST.to_string(),
        };
        let sw = match stopwords {
            Some(p) => read(p)?,
            None => DEFAULT_STOPWORDS.to_string(),
        };
        Ok(Self::build(&wl, &sw, cfg))
    }

    fn build(wordlist: &str, stopwords: &str, cfg: &Config) -> Self {
        let ranked: Vec<String> = entries(wordlist).collect();
        let depth = ranked.len();
        let cutoff = cfg.wordlist_size.min(depth);

        Self {
            common: ranked.into_iter().take(cutoff).collect(),
            stopwords: entries(stopwords).collect(),
            cutoff,
            depth,
            wordlist_digest: *blake3::hash(wordlist.as_bytes()).as_bytes(),
            stopwords_digest: *blake3::hash(stopwords.as_bytes()).as_bytes(),
        }
    }

    /// Whether a lowercase token is ordinary English. Absence is a Stage 1 shape signal.
    pub fn is_common_word(&self, lower: &str) -> bool {
        self.common.contains(lower)
    }

    pub fn is_stopword(&self, lower: &str) -> bool {
        self.stopwords.contains(lower)
    }

    /// Active cutoff and total embedded depth. A cutoff below the requested size means the
    /// config asked for more words than the embedded list holds.
    pub fn wordlist_extent(&self) -> (usize, usize) {
        (self.cutoff, self.depth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_stopwords_are_recognised() {
        // Regression: the stopword file is space-separated, and reading it a line at a
        // time produced eighteen unmatchable multi-word entries instead of ~250 words.
        // Nothing failed loudly — Stage 1 happened to reject stopwords via the wordlist
        // instead — but the prose gate and Stage 3 both measure stopword ratio, and both
        // would have measured zero.
        let res = Resources::default();
        for word in ["the", "a", "is", "and", "of", "to", "in", "that", "it"] {
            assert!(res.is_stopword(word), "{word} is not recognised as a stopword");
        }
        assert!(!res.is_stopword("chromatography"));
    }

    #[test]
    fn the_wordlist_keeps_its_frequency_order() {
        // The same parser reads both files; splitting on whitespace must not disturb
        // rank, which is what the wordlist's line order *means*.
        let small = Config { wordlist_size: 10, ..Config::default() };
        let res = Resources::for_config(&small);
        assert!(res.is_common_word("the"), "a top-10 word fell out of the cutoff");
        assert!(!res.is_common_word("chromatography"));
    }

    #[test]
    fn ordinary_business_vocabulary_is_common_at_the_default_cutoff() {
        let res = Resources::default();
        // The register a report, contract or SOP is written in must count as ordinary,
        // or every formal document would emit its own prose as "technical".
        for w in [
            "procedure", "invoice", "compliance", "calibration", "specification",
            "deviation", "contractual", "analysis", "threshold", "regeneration",
        ] {
            assert!(res.is_common_word(w), "{w} should be ordinary English");
        }
    }

    #[test]
    fn domain_vocabulary_is_not_common_and_so_survives_as_a_signal() {
        let res = Resources::default();
        for w in ["chromatography", "immunoglobulin", "sepharose", "superdex", "elution"] {
            assert!(!res.is_common_word(w), "{w} must not be treated as ordinary English");
        }
    }

    #[test]
    fn cutoff_is_honoured_and_changes_membership() {
        let small = Resources::for_config(&Config { wordlist_size: 5_000, ..Config::default() });
        let large = Resources::default();

        assert_eq!(small.wordlist_extent().0, 5_000);
        assert!(large.is_common_word("calibration"));
        assert!(
            !small.is_common_word("calibration"),
            "a smaller cutoff must actually exclude deeper words"
        );
    }

    #[test]
    fn requesting_more_words_than_embedded_clamps_rather_than_failing() {
        let res = Resources::for_config(&Config { wordlist_size: 10_000_000, ..Config::default() });
        let (cutoff, depth) = res.wordlist_extent();
        assert_eq!(cutoff, depth);
    }
}
