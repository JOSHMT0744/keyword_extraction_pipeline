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
    /// The first `acronym_wordlist_depth` entries. Membership means "too ordinary for an
    /// all-caps spelling to be an acronym".
    frequent: HashSet<String>,
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
        let frequent: HashSet<String> =
            ranked.iter().take(cfg.acronym_wordlist_depth.min(depth)).cloned().collect();

        Self {
            common: ranked.into_iter().take(cutoff).collect(),
            frequent,
            stopwords: entries(stopwords).collect(),
            cutoff,
            depth,
            wordlist_digest: *blake3::hash(wordlist.as_bytes()).as_bytes(),
            stopwords_digest: *blake3::hash(stopwords.as_bytes()).as_bytes(),
        }
    }

    /// Whether a lowercase token is ordinary English. Absence is a Stage 1 shape signal.
    ///
    /// Falls back to [`base_forms`] on a miss: the list is one surface form per entry, so
    /// `automation` is present at rank 37543 while `automations` is not, and without this
    /// every plural, participle and possessive in a document reads as technical
    /// vocabulary. The reduction is applied to the *lookup key* only — surfaces,
    /// normalised forms and offsets are untouched, and digit-bearing tokens are already
    /// classified as identifiers before this is consulted, so nothing here can mangle
    /// `DS-2291`.
    pub fn is_common_word(&self, lower: &str) -> bool {
        self.common.contains(lower)
            || base_forms(lower).any(|base| self.common.contains(&base))
    }

    /// Whether a lowercase token is among the most frequent words in the list.
    ///
    /// Separate from [`is_common_word`] because it answers a different question: not "is
    /// this ordinary English" but "is this so ordinary that an all-caps spelling of it is
    /// shouting rather than an acronym". See [`crate::Config::acronym_wordlist_depth`].
    ///
    /// Deliberately an exact lookup with no morphology. A shouted `CODES` is caught by
    /// the surrounding run rule; widening this would start suppressing genuine acronyms
    /// that happen to reduce to a frequent word.
    pub fn is_frequent_word(&self, lower: &str) -> bool {
        self.frequent.contains(lower)
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

/// Candidate base forms of a token, for the wordlist lookup only.
///
/// A short, explicit list rather than a stemmer. A stemmer is a dependency whose
/// tie-breaking could move [`crate::PipelineVersion`] on an upgrade, and it produces
/// non-words (`automat`) that a surface-form list can never match anyway. What is needed
/// here is narrower: undo the handful of regular English endings that separate a token
/// from an entry the list already holds.
///
/// The `-ise`/`-ize` fold is not cosmetic. The embedded list carries `organise` (12913)
/// and `analyse` (21060) but not `generalise`, so British spellings are covered patchily
/// and `generalises` was emitted as technical vocabulary.
///
/// Yields candidates, not a single answer: the caller accepts the first that is in the
/// list, so an over-eager reduction costs nothing unless it happens to land on a real word.
///
/// **Only inflections, never derivations.** `-s`, `-ed` and `-ing` make the same word
/// again; `-able` makes a different one. Adding `-able` would suppress `auditable` and
/// `inspectable`, which are noise — but it would equally suppress `injectable`
/// (`inject` 11324) and `filterable` (`filter` 9329), which are exactly the domain
/// vocabulary this crate exists to find. Considered and rejected: the recall cost is
/// unbounded and the precision gain is two words. The same argument rules out `-ly`,
/// `-ness` and `-ment`.
fn base_forms(lower: &str) -> impl Iterator<Item = String> + '_ {
    fn push(out: &mut Vec<String>, s: String) {
        if s.chars().count() >= 2 && !out.contains(&s) {
            out.push(s);
        }
    }
    let mut out: Vec<String> = Vec::new();

    // Possessive clitic. Both the typographic and ASCII apostrophe reach here, because
    // canonicalisation folds neither — they are not interchangeable in every context.
    for clitic in ["\u{2019}s", "'s", "\u{2019}", "'"] {
        if let Some(stem) = lower.strip_suffix(clitic) {
            push(&mut out, stem.to_string());
        }
    }

    // Regular inflections, longest suffix first so `ies` is tried before `s`.
    if let Some(stem) = lower.strip_suffix("ies") {
        push(&mut out, format!("{stem}y"));
    }
    for suffix in ["es", "ed", "ing", "s"] {
        if let Some(stem) = lower.strip_suffix(suffix) {
            push(&mut out, stem.to_string());
            // `regenerated` -> `regenerate`, `filing` -> `file`.
            if suffix != "s" {
                push(&mut out, format!("{stem}e"));
            }
            // `shipped` -> `ship`, `running` -> `run`.
            if let Some(undoubled) = undouble(stem) {
                push(&mut out, undoubled);
            }
        }
    }

    // British `-ise` spellings, applied to the reductions above as well as the surface:
    // `generalises` -> `generalise` -> `generalize`.
    let ised: Vec<String> = std::iter::once(lower.to_string())
        .chain(out.clone())
        .filter_map(|w| w.strip_suffix("ise").map(|s| format!("{s}ize"))
            .or_else(|| w.strip_suffix("isation").map(|s| format!("{s}ization")))
            .or_else(|| w.strip_suffix("ised").map(|s| format!("{s}ized")))
            .or_else(|| w.strip_suffix("ising").map(|s| format!("{s}izing"))))
        .collect();
    for w in ised {
        push(&mut out, w);
    }

    out.into_iter()
}

/// Undo a doubled final consonant: `shipp` -> `ship`. `None` when the ending is not a
/// doubled consonant, so `pass` is left alone.
fn undouble(stem: &str) -> Option<String> {
    let mut chars = stem.chars().rev();
    let last = chars.next()?;
    let previous = chars.next()?;
    (last == previous && last.is_alphabetic() && !"aeiou".contains(last))
        .then(|| stem[..stem.len() - last.len_utf8()].to_string())
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
    fn inflections_of_ordinary_words_are_ordinary() {
        // The dominant false-positive class. The list is one surface form per entry, so
        // `automation` is present at 37543 and `automations` is not; without reducing the
        // lookup key, every plural and participle in a document reads as technical.
        let res = Resources::default();
        for w in [
            "automations", "workspaces", "handoffs", "versioned",
            "regenerations", "calibrations", "specifications", "analysed", "shipped",
            "running", "companies",
        ] {
            assert!(res.is_common_word(w), "{w} should reduce to an ordinary word");
        }

        // Reduction can only reach entries the list actually holds. `dataset` is absent
        // at any depth, so `datasets` stays a technical candidate — correctly, and as a
        // reminder that this fixes lookup, not wordlist coverage.
        assert!(!res.is_common_word("datasets"));
    }

    #[test]
    fn possessives_are_ordinary_when_their_stem_is() {
        // `anyone’s` and `scientist’s` were the two highest-scoring keywords in a real
        // document, on no evidence but a clitic the wordlist does not carry.
        let res = Resources::default();
        for w in ["anyone\u{2019}s", "anyone's", "scientist\u{2019}s", "company's"] {
            assert!(res.is_common_word(w), "{w} should reduce to an ordinary word");
        }
    }

    #[test]
    fn british_ise_spellings_reduce_to_their_ize_entries() {
        // The list carries `organise` (12913) and `analyse` (21060) but not `generalise`,
        // so -ise coverage is patchy and `generalises` was emitted as technical.
        let res = Resources::default();
        for w in ["generalise", "generalises", "generalised", "generalising"] {
            assert!(res.is_common_word(w), "{w} should fold to its -ize entry");
        }
    }

    #[test]
    fn reduction_does_not_make_domain_vocabulary_look_ordinary() {
        // The cost of the reduction, contained. If any of these acquire an ordinary base
        // form the technical arm loses the terms it exists to find.
        let res = Resources::default();
        for w in [
            "chromatography", "immunoglobulin", "sepharose", "superdex", "elution",
            "elutions", "chromatographies",
        ] {
            assert!(!res.is_common_word(w), "{w} must not be treated as ordinary English");
        }
    }

    #[test]
    fn reduction_never_produces_a_match_from_a_stub() {
        // A two-character floor on candidates, so `as` -> `a` and similar cannot make an
        // arbitrary token ordinary by shrinking it into a high-frequency fragment.
        for w in ["ies", "ing", "es", "ed"] {
            assert!(
                base_forms(w).all(|b| b.chars().count() >= 2),
                "{w} produced a one-character base form"
            );
        }
    }

    #[test]
    fn the_acronym_depth_is_narrower_than_the_wordlist_cutoff() {
        // The rule that separates an acronym from a shouted common word. `sop` sits at
        // 39910 and must stay outside the frequent band; `code` and `today` must not.
        let res = Resources::default();
        assert!(res.is_frequent_word("today"), "today is rank 243");
        assert!(res.is_frequent_word("code"), "code is rank 1417");
        assert!(!res.is_frequent_word("sop"), "sop is rank 39910 and is a real collision");
        for absent in ["hplc", "eln", "lims", "qms"] {
            assert!(!res.is_frequent_word(absent), "{absent} is absent at any depth");
        }
    }

    #[test]
    fn stopword_lookup_stays_exact() {
        // Deliberately not widened alongside is_common_word. The stopword list is a
        // closed set of function words, and reducing into it would swallow real ones.
        let res = Resources::default();
        assert!(res.is_stopword("the"));
        assert!(!res.is_stopword("theses"), "reduction leaked into the stopword set");
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
