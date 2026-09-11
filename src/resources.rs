//! Static resources: the general English wordlist, stopword list and lemma exception list.
//!
//! Defaults are compiled in with `include_str!`, so they cannot drift and need no
//! deployment step. An optional runtime override is supported; its digest folds into
//! [`crate::PipelineVersion`] exactly as the embedded digest does, so an override can
//! never change output silently.
//!
//! # Why the lists are embedded rather than fetched
//!
//! Not a packaging preference — fetching is incompatible with what this crate promises.
//!
//! [`crate::PipelineVersion`] hashes these bytes. A list pulled from the network would
//! make the stamp depend on what the network returned at that moment, so either two runs
//! a week apart produce different keywords under one stamp, or the stamp moves with no
//! code change. Either way "a keyword set is a pure function of `(bytes, config,
//! resources)`" stops being true, and that guarantee is the reason the crate exists.
//!
//! [`crate::extract`] is also a pure function with no failure mode for this: it returns a
//! [`crate::DocumentStatus`] describing the *document*, and there is deliberately no
//! variant meaning "a resource could not be reached". Fetching would mean blocking I/O
//! inside a pure function, or an async API, or a fallible construction that can fail for
//! reasons having nothing to do with the document being read.
//!
//! Fetching in `build.rs` only moves the problem: it breaks offline builds, `cargo
//! vendor` and docs.rs, and makes the digest depend on build-day content rather than
//! run-day content. Taking the data from another crate does not remove it either — it
//! relocates it and gives up control over its content, which is how a list built for a
//! different tokeniser gets in (see `fold_apostrophes` and the stopword file's header).
//!
//! The `scripts/fetch_*.sh` scripts are therefore developer tools run deliberately, with
//! their output committed and reviewed. The committed file is the artifact; the script is
//! the record of how it was derived and the means to redo it.
//!
//! Cost of the whole arrangement: about 740 KB embedded, 455 KB in the packaged crate.
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

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};

use crate::{config::Config, error::ExtractError};

const DEFAULT_WORDLIST: &str = include_str!("../resources/wordlist.txt");
const DEFAULT_STOPWORDS: &str = include_str!("../resources/stopwords.txt");
const DEFAULT_LEMMAS: &str = include_str!("../resources/lemmas.txt");

#[derive(Debug, Clone)]
pub struct Resources {
    /// Words at or above the configured frequency rank. Membership means "ordinary".
    common: HashSet<String>,
    /// The first `acronym_wordlist_depth` entries. Membership means "too ordinary for an
    /// all-caps spelling to be an acronym".
    frequent: HashSet<String>,
    stopwords: HashSet<String>,
    /// Irregular inflection to lemma or lemmas. The half of WordNet's morphy algorithm
    /// that [`base_forms`] cannot supply, because irregular inflection is enumeration
    /// rather than rule.
    lemmas: HashMap<String, Vec<String>>,
    /// How many of the ranked words are active. Retained for diagnostics.
    cutoff: usize,
    /// Total available depth, so a config asking for more than exists can be reported.
    depth: usize,
    pub(crate) wordlist_digest: [u8; 32],
    pub(crate) stopwords_digest: [u8; 32],
    pub(crate) lemmas_digest: [u8; 32],
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

/// Parse the lemma file: one inflected form per line, followed by its lemmas.
///
/// Deliberately not [`entries`]. That function splits on whitespace, which is right for
/// the wordlist and the stopword list and would turn `mice mouse` here into two
/// unrelated entries — the same shape of silent failure as LOGIC_REVISION 2, where the
/// stopword list was read a line at a time and every lookup quietly returned false.
fn lemma_entries(text: &str) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for line in text.lines() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace().map(str::to_lowercase);
        let Some(inflected) = fields.next() else { continue };
        for lemma in fields {
            if lemma == inflected {
                continue;
            }
            let slot = out.entry(inflected.clone()).or_default();
            if !slot.contains(&lemma) {
                slot.push(lemma);
            }
        }
    }
    out
}

impl Default for Resources {
    /// Resources under [`Config::default`]. Use [`Resources::for_config`] when the config
    /// is not the default, or the active cutoff will not match the version stamp.
    fn default() -> Self {
        Self::build(DEFAULT_WORDLIST, DEFAULT_STOPWORDS, DEFAULT_LEMMAS, &Config::default())
    }
}

impl Resources {
    pub fn for_config(cfg: &Config) -> Self {
        Self::build(DEFAULT_WORDLIST, DEFAULT_STOPWORDS, DEFAULT_LEMMAS, cfg)
    }

    /// Load overrides from disk. Any file may be omitted to keep the embedded default.
    ///
    /// An override wordlist must also be frequency-ranked, most frequent first. An
    /// override lemma file carries one inflected form per line followed by its lemmas.
    /// Every override's digest folds into [`crate::PipelineVersion`] exactly as the
    /// embedded digest does, so none of them can change output silently.
    pub fn with_overrides(
        cfg: &Config,
        wordlist: Option<&Path>,
        stopwords: Option<&Path>,
        lemmas: Option<&Path>,
    ) -> Result<Self, ExtractError> {
        let read = |p: &Path| {
            fs::read_to_string(p).map_err(|e| ExtractError::Resource(format!("{}: {e}", p.display())))
        };
        let or_default = |path: Option<&Path>, default: &str| match path {
            Some(p) => read(p),
            None => Ok(default.to_string()),
        };
        let wl = or_default(wordlist, DEFAULT_WORDLIST)?;
        let sw = or_default(stopwords, DEFAULT_STOPWORDS)?;
        let lm = or_default(lemmas, DEFAULT_LEMMAS)?;
        Ok(Self::build(&wl, &sw, &lm, cfg))
    }

    fn build(wordlist: &str, stopwords: &str, lemmas: &str, cfg: &Config) -> Self {
        let ranked: Vec<String> = entries(wordlist).collect();
        let depth = ranked.len();
        let cutoff = cfg.wordlist_size.min(depth);
        let frequent: HashSet<String> =
            ranked.iter().take(cfg.acronym_wordlist_depth.min(depth)).cloned().collect();

        Self {
            common: ranked.into_iter().take(cutoff).collect(),
            frequent,
            stopwords: entries(stopwords).collect(),
            lemmas: lemma_entries(lemmas),
            cutoff,
            depth,
            wordlist_digest: *blake3::hash(wordlist.as_bytes()).as_bytes(),
            stopwords_digest: *blake3::hash(stopwords.as_bytes()).as_bytes(),
            lemmas_digest: *blake3::hash(lemmas.as_bytes()).as_bytes(),
        }
    }

    /// Whether a lowercase token is ordinary English. Absence is a Stage 1 shape signal.
    ///
    /// Lemmatises the key on a miss: the list is one surface form per entry, so
    /// `automation` is present at rank 37524 while `automations` is not, and without this
    /// every plural, participle and possessive in a document reads as technical
    /// vocabulary. The reduction is applied to the *lookup key* only — surfaces,
    /// normalised forms and offsets are untouched, and digit-bearing tokens are already
    /// classified as identifiers before this is consulted, so nothing here can mangle
    /// `DS-2291`.
    pub fn is_common_word(&self, lower: &str) -> bool {
        let lower = &*fold_apostrophes(lower);
        self.common.contains(lower)
            || self.lemmas_of(lower).iter().any(|l| self.common.contains(l))
            || base_forms(lower).any(|base| self.common.contains(&base))
    }

    /// Irregular lemmas of a form, empty when it has none.
    ///
    /// Consulted before [`base_forms`], which is WordNet morphy's own order: the
    /// exception list is authoritative where it applies, and the suffix rules are the
    /// fallback. For a boolean membership test the order cannot change the answer, but
    /// it is the order the algorithm specifies and reversing it would be a trap for
    /// anyone later making this return a single lemma.
    fn lemmas_of(&self, lower: &str) -> &[String] {
        self.lemmas.get(lower).map_or(&[], Vec::as_slice)
    }

    /// Whether a lowercase token is among the most frequent words in the list.
    ///
    /// Separate from [`Self::is_common_word`] because it answers a different question: not "is
    /// this ordinary English" but "is this so ordinary that an all-caps spelling of it is
    /// shouting rather than an acronym". See [`crate::Config::acronym_wordlist_depth`].
    ///
    /// Deliberately an exact lookup with no morphology. A shouted `CODES` is caught by
    /// the surrounding run rule; widening this would start suppressing genuine acronyms
    /// that happen to reduce to a frequent word.
    pub fn is_frequent_word(&self, lower: &str) -> bool {
        self.frequent.contains(lower)
    }

    /// Whether a lowercase token is an English function word.
    ///
    /// Exact apart from apostrophe form, which is normalised in the lookup key. That is
    /// **not** a widening of the set — see `fold_apostrophes` for why the alternative,
    /// listing both spellings of every contraction, is the wrong shape of fix.
    pub fn is_stopword(&self, lower: &str) -> bool {
        self.stopwords.contains(&*fold_apostrophes(lower))
    }

    /// Active cutoff and total embedded depth. A cutoff below the requested size means the
    /// config asked for more words than the embedded list holds.
    pub fn wordlist_extent(&self) -> (usize, usize) {
        (self.cutoff, self.depth)
    }
}

/// Normalise apostrophe form in a lookup key.
///
/// A resource file can only carry one spelling of `don't`, and the one it carries is
/// ASCII. Real documents overwhelmingly carry U+2019: Word, most PDF toolchains and every
/// publishing pipeline substitute the typographic form, and canonicalisation deliberately
/// does not fold it, because U+2019 is also a closing quotation mark and folding it in the
/// *text* would change what offsets resolve to.
///
/// So the fold happens here, in the key, exactly as [`base_forms`] does — surfaces,
/// normalised forms and offsets are untouched.
///
/// The alternative is to list both spellings of every contraction in the resource file.
/// That is the wrong shape of fix, and NLTK's list is what it looks like carried to its
/// conclusion: entries such as `didn`, `isn` and `ve` exist there only to absorb a
/// tokeniser that fragments `didn't`, turning a *normalisation* problem into a membership
/// problem and leaving the set full of things that are not function words at all. A list
/// should hold one canonical spelling; making a token match it is the lookup's job.
///
/// Left unfixed, a typographic `don't` was absent from both resource lists, scored
/// `absent_from_wordlist` 1.0 for a shape total of 0.40, and was emitted as a technical
/// keyword — the same false-positive class as `anyone's` before LOGIC_REVISION 6.
fn fold_apostrophes(lower: &str) -> Cow<'_, str> {
    const TYPOGRAPHIC: [char; 3] = ['\u{2019}', '\u{02bc}', '\u{ff07}'];
    if lower.contains(TYPOGRAPHIC) {
        Cow::Owned(lower.replace(TYPOGRAPHIC, "'"))
    } else {
        Cow::Borrowed(lower)
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
/// The `-ise`/`-ize` fold is not cosmetic. The embedded list carries `organise` (12894)
/// and `analyse` (21041) but not `generalise`, so British spellings are covered patchily
/// and `generalises` was emitted as technical vocabulary.
///
/// Yields candidates, not a single answer: the caller accepts the first that is in the
/// list, so an over-eager reduction costs nothing unless it happens to land on a real word.
///
/// **Only inflections, never derivations.** `-s`, `-ed` and `-ing` make the same word
/// again; `-able` makes a different one. Adding `-able` would suppress `auditable` and
/// `inspectable`, which are noise — but it would equally suppress `injectable`
/// (`inject` 11305) and `filterable` (`filter` 9310), which are exactly the domain
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
        // `automation` is present at 37524 and `automations` is not; without reducing the
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
        // The list carries `organise` (12894) and `analyse` (21041) but not `generalise`,
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
    fn irregular_inflections_reduce_to_their_lemma() {
        // What the exception list is for. `base_forms` reaches `running` -> `run` by
        // rule; no rule reaches `ran` -> `run`. Without the list these are two different
        // words, and the irregular one reads as technical vocabulary because the wordlist
        // holds only the lemma.
        let res = Resources::default();
        for w in [
            "ran", "mice", "was", "children", "went", "feet", "geese", "women", "knives",
            "better", "analyses", "teeth", "oxen", "lives",
        ] {
            assert!(res.is_common_word(w), "{w} should lemmatise to an ordinary word");
        }
    }

    #[test]
    fn classical_plurals_no_longer_read_as_technical_vocabulary() {
        // The concrete cost of suffix-only reduction, in the register this crate's
        // documents are actually written in. Each of these scored absent_from_wordlist
        // 1.0 for a shape total of 0.40 and cleared the 0.38 technical threshold, so a
        // report containing the word `appendices` emitted it as a technical term.
        let res = Resources::default();
        for w in ["indices", "appendices", "syntheses", "addenda", "curricula", "formulae"] {
            assert!(res.is_common_word(w), "{w} should lemmatise to an ordinary word");
        }
    }

    #[test]
    fn lemmatisation_reaches_only_lemmas_the_wordlist_actually_holds() {
        // The reduction fixes lookup, not wordlist coverage — the same boundary as
        // `datasets`. `vertices` lemmatises to `vertex`, which is absent at this cutoff,
        // so it stays a technical candidate. Correct, and worth pinning so that a future
        // change cannot quietly turn lemmatisation into a second wordlist.
        let res = Resources::default();
        assert!(!res.is_common_word("vertices"));
    }

    #[test]
    fn the_exception_list_does_not_make_domain_vocabulary_ordinary() {
        // The red line. If a lemma ever lands on an ordinary word, the technical arm
        // loses the terms it exists to find.
        let res = Resources::default();
        for w in [
            "chromatography", "immunoglobulin", "sepharose", "superdex", "elution",
            "elutions", "hplc",
        ] {
            assert!(!res.is_common_word(w), "{w} must not be treated as ordinary English");
        }

        // The boundary, stated so it is not mistaken for a leak: `bacilli` DOES become
        // ordinary, because `bacillus` is a general English word the list holds at rank
        // 49 421. Lemmatisation is not deciding that; the wordlist is. A term is domain
        // vocabulary here only when its lemma is absent from general English.
        assert!(res.is_common_word("bacilli"));
    }

    #[test]
    fn the_lemma_file_is_read_a_line_at_a_time_not_split_on_whitespace() {
        // `entries()` splits on whitespace, which is right for the other two files and
        // would turn `mice mouse` into two unrelated entries here. This is the LOGIC
        // REVISION 2 failure shape: a parser mismatch that makes every lookup return
        // nothing, loudly correct in structure and silently empty in effect.
        let parsed = lemma_entries("# comment\nmice mouse\nbetter good well\naxes ax axis\n");
        assert_eq!(parsed["mice"], ["mouse"]);
        assert_eq!(parsed["better"], ["good", "well"], "a multi-lemma line was truncated");
        assert_eq!(parsed["axes"], ["ax", "axis"]);
        assert!(!parsed.contains_key("#"), "the comment line was parsed as an entry");
    }

    #[test]
    fn every_lemma_file_field_is_a_word_the_wordlist_could_hold() {
        // scripts/fetch_lemmas.sh keeps only ^[a-z]+$ on both sides, because a lemma the
        // wordlist can never hold is dead weight — WordNet's underscore collocations
        // (`allows_for` -> `allow_for`) and hyphenated entries among them. Pinned so a
        // hand edit cannot reintroduce an unusable entry.
        let parsed = lemma_entries(DEFAULT_LEMMAS);
        assert!(parsed.len() > 5_000, "the lemma file looks truncated: {}", parsed.len());
        for (inflected, lemmas) in &parsed {
            for field in std::iter::once(inflected).chain(lemmas) {
                assert!(
                    field.chars().all(|c| c.is_ascii_lowercase()),
                    "{field:?} can never match a wordlist entry"
                );
            }
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
        // 39891 and must stay outside the frequent band; `code` and `today` must not.
        let res = Resources::default();
        assert!(res.is_frequent_word("today"), "today is rank 224");
        assert!(res.is_frequent_word("code"), "code is rank 1398");
        assert!(!res.is_frequent_word("sop"), "sop is rank 39891 and is a real collision");
        for absent in ["hplc", "eln", "lims", "qms"] {
            assert!(!res.is_frequent_word(absent), "{absent} is absent at any depth");
        }
    }

    #[test]
    fn every_embedded_resource_names_the_script_that_regenerates_it() {
        // These files are data, and data with no reproduction path is an assertion. The
        // sentinel is also load-bearing: each fetch script rebuilds the file by copying
        // everything up to this line and appending a fresh body, so a missing sentinel
        // makes the sed range run to end-of-file and silently emit the old body twice.
        for (name, text) in [
            ("wordlist", DEFAULT_WORDLIST),
            ("stopwords", DEFAULT_STOPWORDS),
            ("lemmas", DEFAULT_LEMMAS),
        ] {
            let line = text
                .lines()
                .find(|l| l.starts_with("# Regenerate with"))
                .unwrap_or_else(|| panic!("resources/{name}.txt names no regeneration script"));
            let script = format!("scripts/fetch_{name}.sh");
            assert!(
                line.contains(&script),
                "resources/{name}.txt points at something other than {script}: {line:?}"
            );
            assert!(
                std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/"))
                    .join(&script)
                    .exists(),
                "{script} is named but does not exist"
            );
        }
    }

    #[test]
    fn stopword_entries_are_tokens_our_tokeniser_can_actually_produce() {
        // The instrument behind rejecting the `stop-words` crate and NLTK's list verbatim.
        // A stopword list is only meaningful against the tokeniser that consumes it: NLTK
        // splits `didn't` into `did` + `n't`, so a quarter of its entries are fragments
        // (`didn`, `isn`, `ve`, `s`, `t`) that never appear in our token stream, and one
        // (`won`) is a real English verb it would suppress.
        //
        // This is also the shape of the LOGIC_REVISION 2 bug: eighteen multi-word entries
        // that no lookup could ever match, failing silently.
        for entry in entries(DEFAULT_STOPWORDS) {
            let toks = crate::tokenize::tokens(&entry);
            assert_eq!(
                toks.len(),
                1,
                "{entry:?} is not one token — no lookup can ever match it"
            );
            assert_eq!(
                toks[0].text, entry,
                "{entry:?} survives tokenisation as {:?}, so it would never be matched",
                toks[0].text
            );
        }
    }

    #[test]
    fn the_stopword_list_carries_the_modals_nltk_omits() {
        // NLTK's English list has no modals, so under it YAKE admits phrases headed by
        // one: `may require calibration`, `must be restarted`. They are function words.
        let res = Resources::default();
        for w in ["may", "might", "must", "shall", "should", "would", "could", "cannot", "ought"] {
            assert!(res.is_stopword(w), "{w} is a modal and must be a stopword");
        }
        // And the ordinary verbs a tokeniser-mismatched list would wrongly swallow.
        for w in ["won", "ma", "act", "research", "accordance"] {
            assert!(!res.is_stopword(w), "{w} is a content word, not a function word");
        }
    }

    #[test]
    fn a_typographic_apostrophe_matches_the_ascii_entry_in_the_list() {
        // Real documents carry U+2019, not U+0027: Word and most PDF toolchains
        // substitute it. Canonicalisation deliberately does not fold it, because it is
        // also a closing quote, so the fold lives in the lookup key.
        //
        // Before this, `don’t` was in neither resource list, scored absent_from_wordlist
        // 1.0 for a shape total of 0.40, and cleared the 0.38 technical threshold.
        let res = Resources::default();
        for (ascii, typographic) in [
            ("don't", "don\u{2019}t"),
            ("it's", "it\u{2019}s"),
            ("we're", "we\u{2019}re"),
            ("that's", "that\u{2019}s"),
            ("shouldn't", "shouldn\u{2019}t"),
        ] {
            assert!(res.is_stopword(ascii), "{ascii} is not in the list at all");
            assert!(
                res.is_stopword(typographic),
                "{typographic} does not match its ASCII entry {ascii}"
            );
        }
        // The same fold reaches the wordlist, which is what fixed `anyone’s`.
        assert!(res.is_common_word("anyone\u{2019}s"));
    }

    #[test]
    fn folding_apostrophes_does_not_widen_either_set() {
        // The fold normalises spelling; it must not make a non-member match. If it ever
        // does, the cure has become the disease the NLTK fragments are.
        let res = Resources::default();
        for w in ["chromatography", "chromatography\u{2019}s", "theses", "sepharose"] {
            assert!(!res.is_stopword(w), "{w} leaked into the stopword set");
        }
        assert!(!res.is_common_word("sepharose\u{2019}s"));
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
