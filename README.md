# keyword_extraction_pipeline

Pull the distinctive, exact-matchable terms out of a document — batch codes, ticket
references, part numbers, product names, domain vocabulary, topics — with no model, no
training, and no corpus.

Raw bytes in (PDF, docx, pptx, xlsx, email, text), ranked keywords out. The same bytes
always produce the same keywords, and a version stamp makes that checkable.

```
tests/fixtures/simple.pdf
  Ok  en 1.00  261 chars
  Identifier  0.722  DS-2291                       x1   shape
              0.722  SOP-114                       x1   shape
              0.650  HEK293T                       x1   shape
  Technical   0.535  MabSelect SuRe                x1   shape
              0.520  MabSelect                     x1   shape
              0.415  chromatography                x1   shape
  Topical     —      prose gate: too few tokens for statistical extraction
                     (mean sentence 9.5 tokens, 29% stopwords, 38 tokens)
```

**Contents** — [Quick start](#quick-start) · [What you get](#what-you-get) ·
[Using the library](#using-the-library) · [Using the CLI](#using-the-cli) ·
[Recipes](#recipes) · [Gotchas](#gotchas) · [Configuration](#configuration) ·
[How it works](#how-it-works) · [Reproducibility](#reproducibility) ·
[Scope](#what-this-is-not-for)

---

## Quick start

The fastest way to see what it does on your own files:

```sh
git clone https://github.com/JOSHMT0744/keyword_extraction_pipeline
cd keyword_extraction_pipeline
cargo run --release --bin kep -- extract --format table ./your-documents/
```

Or install the `kep` binary onto your PATH:

```sh
cargo install --path .
kep extract ./your-documents/
```

As a dependency:

```toml
[dependencies]
keyword_extraction_pipeline = { git = "https://github.com/JOSHMT0744/keyword_extraction_pipeline" }
```

```rust
use keyword_extraction_pipeline::{extract, Config, DocumentStatus, FormatHint, Resources};

let bytes = std::fs::read("report.pdf").unwrap();

let cfg = Config::default();
let res = Resources::for_config(&cfg);
let result = extract(&bytes, FormatHint::Sniff, &cfg, &res);

match result.status {
    DocumentStatus::Ok => {
        for kw in &result.keywords {
            println!("{:?}  {:<20} {:.3}", kw.kind, kw.original_keyword, kw.score);
        }
    }
    other => println!("no keywords, and here is why: {other:?}"),
}
```

Requires Rust 1.88+. Every Rust example in this README is compiled and run by
`tests/readme_examples.rs`, so none of them can quietly rot.

---

## What you get

Every keyword has a **kind**. This is the central idea, and most of how you use the
output follows from it:

| Kind | What it is | Examples | Use it for |
|------|-----------|----------|-----------|
| `Identifier` | Coded, digit-bearing or structured tokens | `DS-2291`, `JIRA-4417`, `INV-2024-0917`, `HEK293T` | Exact matching, joins, lookup keys |
| `Technical` | Terms absent from general English, or defined in the text | `chromatography`, `MabSelect SuRe`, `SOP` | Faceting, domain vocabulary, glossaries |
| `Topical` | Ordinary-word phrases describing what the document is about | `column regeneration`, `resin lifetime` | Summaries, clustering, human-readable tags |

**Scores are comparable within a kind and meaningless across kinds.** An `Identifier`
at 0.72 and a `Topical` at 0.72 have nothing to do with each other — they come from
different scoring systems. Never sort the three together. `rank` is 0-based *within*
each kind, so `rank == 0` means "best of its kind in this document".

Output is **uncapped** above a per-kind threshold, so you impose your own top-N with
`--top` or by filtering on `rank` — without re-extracting.

Each `Keyword` carries:

| Field | Meaning |
|-------|---------|
| `original_keyword` | The keyword as actually written in the document |
| `normalised` | Lowercased, whitespace-collapsed. **Match on this.** Never stemmed |
| `kind` | `Identifier` / `Technical` / `Topical` |
| `origin` | Which stage found it: `Shape`, `Definition`, `Statistic` |
| `score` | Higher is a stronger claim. Within-kind only |
| `rank` | 0-based within kind |
| `frequency` | Occurrence count |
| `offsets` | Byte ranges into the canonical text |
| `expansion` | The long form, when a definition resolved one (`SOP` → `Standard Operating Procedure`) |
| `features` | Stage 1's feature breakdown, when `retain_features` is on |

### A document that yields nothing still tells you why

`extract` never returns `Err` for a document it simply cannot read. You always get a
`DocumentResult` with a `status`:

| Status | Meaning |
|--------|---------|
| `Ok` | Extraction ran |
| `NoTextLayer` | Parsed fine but has no text — almost always a scanned PDF. **Queue it for OCR**, don't discard it |
| `Encrypted` | Password-protected |
| `UnsupportedFormat` | No parser for these bytes |
| `ParseError(String)` | The parser rejected the file |
| `TooShort` | No meaningful content at all |

This matters: a scanned PDF and a genuinely bland document would otherwise both be
"empty keyword list", and you could never tell them apart. `Err` is reserved for a
caller mistake.

---

## Using the library

Two functions, both pure:

```rust
pub fn extract(bytes: &[u8], hint: FormatHint, cfg: &Config, res: &Resources) -> DocumentResult;
pub fn canonicalise(bytes: &[u8], hint: FormatHint, cfg: &Config) -> Result<String, ExtractError>;
```

`FormatHint::Sniff` detects the format from the bytes. Pass a specific hint
(`FormatHint::Pdf`, `Docx`, `Xlsx`, `Email`, …) if you already know it — file
extensions are never trusted.

`extract` does not return the document text. To resolve a keyword's `offsets`,
regenerate the exact canonical string they refer to with `canonicalise`. It is
idempotent and deterministic, so the string is byte-identical to the one the offsets
were computed against.

---

## Using the CLI

```
kep extract <path>...   Walk files and directories, emit keywords
kep explain <file>      Account for one document's scores, feature by feature
kep config              Print the effective config as JSON, ready to edit
kep version             Crate version, logic revision, wordlist extent, stamp
```

**Data goes to stdout, commentary goes to stderr.** So `kep extract ./corpus >
out.jsonl` is a clean file, and you still see the run header and status trailer:

```
kep 8 documents | format Jsonl | config defaults | pipeline_version 632c19e857df
...
kep 8 documents: 6 ok, 1 no-text-layer, 1 too-short
```

`--format` is `table`, `jsonl`, `json` or `csv`. It defaults to `table` on a terminal
and `jsonl` when redirected, so you get something readable interactively and something
parseable in a pipeline. The flag always wins, and the header states which was used.
Rendering is the *only* thing it affects.

Useful flags: `--kind identifier` (repeatable), `--top N`, `--features`,
`--config-file <f.json>`, `--wordlist-size N`, `--threshold-identifier`,
`--threshold-technical`, `--quiet`, `--strict`.

**Exit codes:** `0` every path was read · `1` a path could not be read · `2` under
`--strict`, some document returned a status other than `Ok`. A `NoTextLayer` result is
the pipeline working correctly, so by default it does not fail the run.

### `kep explain` — why did this score what it scored?

```
$ kep explain report.pdf

  EMITTED
    + DS-2291                      Identifier  score 0.722  x1
        absent_from_wordlist    1.00 x 0.40 = 0.400
        digit_letter_mix        1.00 x 0.25 = 0.250
        separator_segments      0.60 x 0.12 = 0.072

  BELOW THRESHOLD (what a lower threshold would admit, best first)
    - SuRe                         Technical  score 0.120  x1
        internal_caps           1.00 x 0.12 = 0.120
```

The near-misses are the useful half: they tell you what a lower threshold would buy
you. This is the tool for tuning, rather than guessing at numbers.

---

## Recipes

Each of these assumes you already have a `result`:

```rust
use keyword_extraction_pipeline::*;

let bytes = std::fs::read("report.pdf").unwrap();
let cfg = Config::default();
let res = Resources::for_config(&cfg);
let result = extract(&bytes, FormatHint::Sniff, &cfg, &res);
```

**Top 5 identifiers**

```rust
let ids: Vec<&str> = result
    .keywords
    .iter()
    .filter(|k| k.kind == Kind::Identifier)
    .take(5)                       // already ranked best-first within the kind
    .map(|k| k.original_keyword.as_str())
    .collect();
```

**Resolve an offset back to the text**

```rust
let text = canonicalise(&bytes, FormatHint::Sniff, &cfg).unwrap();
let kw = &result.keywords[0];
let span = kw.offsets[0].clone();
assert_eq!(text[span].to_lowercase(), kw.normalised);
```

**One record per term** (see [the same term twice](#two-stages-can-report-the-same-term))

```rust
// Prefer definitional evidence over an orthographic guess for the same term.
result.keywords.sort_by_key(|k| (k.origin != Origin::Definition) as u8);
let mut seen = std::collections::HashSet::new();
result.keywords.retain(|k| seen.insert((k.normalised.clone(), k.kind)));
```

**Find out why there were no topical keywords**

```rust
if !result.keywords.iter().any(|k| k.kind == Kind::Topical) {
    let verdict = result.prose.expect("the document reached the gate");
    println!("no topical keywords: {}", verdict.summary());
}
```

**Tune, and see the feature breakdown**

```rust
use keyword_extraction_pipeline::{Config, Resources};

let cfg = Config {
    wordlist_size: 40_000,      // smaller list = more words look "technical"
    retain_features: true,      // attach the feature vector to every keyword
    ..Config::default()
};
let res = Resources::for_config(&cfg);   // MUST rebuild for the new config
```

**Batch a corpus from the shell**

```sh
kep extract ./corpus > keywords.jsonl                      # one JSON object per line
kep extract --format csv ./corpus > keywords.csv           # one row per keyword
kep extract --format table --kind identifier --top 3 ./corpus
```

---

## Gotchas

Collected here because each one has cost someone time.

### `Resources` must be built from the same `Config`

```rust
let cfg = Config { wordlist_size: 40_000, ..Config::default() };

let res = Resources::for_config(&cfg);   // right
let res = Resources::default();          // WRONG here: only matches Config::default()
```

`Resources::default()` matches `Config::default()` and nothing else. Pairing a
non-default config with default resources gives you a wordlist cutoff that disagrees
with the version stamp — output that is wrong *and* mislabelled.

### Two stages can report the same term

`SOP` can be found by orthography *and* by the sentence `Standard Operating Procedure
(SOP)`. Both are kept, distinguished by `origin`, because an orthographic guess and a
definitional match are different claims about the same string.

The cost is that `keywords` can hold the same term twice within a kind, and naive
counting double-counts. Deduplicate on `(normalised, kind)` if you want uniqueness —
see the [recipe](#recipes). `Origin::Definition` entries score `1.0` by construction
(a definition is categorical evidence, not another weighted vote), which is the one
place two score scales coexist inside a kind.

### The CLI's JSON is not the library's type

`serde_json::from_str::<DocumentResult>` will **not** accept `kep`'s output. The CLI
groups keywords by kind (`{"identifier": [...], "technical": [...], "topical": [...]}`)
whereas the library type holds one flat ranked `Vec`. Use the library API if you want
the typed result back.

### Your documents may not be "prose"

The topical stage is gated: on a spreadsheet it would emit column headers as topics, and
on an email footer the disclaimer. Slide decks, bulleted reports and genuinely form-like
PDFs are rejected too.

The gate measures **clauses**, not lines. This matters more than it sounds. Canonical text
keeps one newline per *rendered* line, so a PDF's wrapped body text arrives as dozens of
short unpunctuated fragments. Measured as lines, a 1 091-token business PDF scored a mean
sentence length of 5.9 tokens and 42 % table-like lines, and was rejected as
`SentencesTooShort` — both numbers were reading the document's column width. Measured as
clauses (`tokenize::clauses`, which heals a break only when the text before it did not end
in sentence punctuation *and* the text after it begins lowercase) the same document scores
9.7 and 19 %, and passes.

If your corpus is genuinely tabular or bulleted and you want topics anyway, lower
`prose.min_mean_sentence_len`. The verdict is always on `DocumentResult::prose`, and the
CLI prints it, so you never have to guess which check rejected a document.

### Match on `normalised`, not `original_keyword`

One keyword covers one *normalised* form, so a document containing both
`Chromatography` and `chromatography` produces a single record with two offsets.
`original_keyword` holds only the first variant seen and is **not** guaranteed to equal
the text at every offset. The invariant that holds is: `canonical[span]`, lowercased and
whitespace-collapsed, equals `normalised`.

### `thresholds.topical` is not yet validated

The default of `0.15` predates any measurement against a keyphrase benchmark, and is
conservative — it admits only a handful of phrases per document. If topical output
looks thin, that number is the first thing to raise. `thresholds.identifier` and
`thresholds.technical` are similarly unswept.

---

## Configuration

`Config` derives `Serialize`/`Deserialize`, and every field has a default, so a config
file may set only what it changes:

```sh
kep config > kep.json          # dump the effective config
$EDITOR kep.json               # change what you need
kep extract --config-file kep.json ./corpus
```

| Field | Default | Purpose |
|-------|---------|---------|
| `wordlist_size` | 65 000 | How many frequency-ranked words count as ordinary English. **The main dial.** Lower it to treat more vocabulary as technical. The lookup is lemmatised, so `automations` is matched by `automation` and `mice` by `mouse` |
| `acronym_wordlist_depth` | 20 000 | How far down the list a word stays too ordinary for an all-caps spelling to be an acronym. Keeps `SOP` (39 910) while rejecting a shouted `CODE` (1 417) |
| `thresholds.identifier` | 0.55 | Emission cutoff for identifiers |
| `thresholds.technical` | 0.38 | Emission cutoff for technical terms |
| `thresholds.topical` | 0.15 | Upper bound for topical — YAKE is *lower is better* |
| `shape_weights` | sums to 1.0 | Stage 1 feature weights (see `src/config.rs`) |
| `prose` | — | Gate for Stage 3, measured over clauses rather than rendered lines: mean sentence length 8.0, stopword ratio 0.20, table-line ratio 0.40, min tokens 120 |
| `min_content_length` | 16 | "Nothing here at all" cutoff — not a readability floor |
| `no_text_layer_threshold` | 32 | Chars below which a PDF counts as having no text layer |
| `strip_quoted_blocks` | true | Strip quoted replies and signatures from email |
| `enable_definitions` / `enable_topical` | true | Stage 2 / Stage 3 switches |
| `yake_ngram_max` | 3 | Longest topical phrase |
| `retain_features` | false | Attach Stage 1's feature vector to each keyword |

**Every field except `retain_features` feeds the version stamp**, so changing one
invalidates cached keyword sets — deliberately, so a config change can never silently
produce different output under the same version. `retain_features` is excluded because
it cannot change *which* keywords are emitted, only how much is reported about them.

---

## How it works

| Step | What happens |
|------|--------------|
| **Parse** | Bytes → raw text. Format sniffed from magic bytes and content shape, never the extension |
| **Canonicalise** | NFKC, newline and page-break normalisation, typographic-hyphen folding, soft-hyphen removal, line-break dehyphenation, email quote and signature stripping, whitespace collapse. Idempotent. Every offset is relative to this |
| **Gates** | No text layer → `NoTextLayer`. Below `min_content_length` → `TooShort`. Recorded outcomes, not errors |
| **Language** | `lingua` over 7 European languages. English runs everything; other languages degrade *visibly*, recorded as `fully_supported: false` |
| **Stages** | Three complementary extractors. All enabled stages run and their results are ranked as one union — they produce different kinds, so they never compete for slots |

### Formats

| Format | Backend | Notes |
|--------|---------|-------|
| PDF | `pdf_oxide` (pinned `=0.3.78`) | Born-digital only; scanned PDFs report `NoTextLayer` |
| docx, pptx | `quick-xml` over the zip container | |
| xlsx | `calamine` | Shared strings and cell types; always treated as tabular |
| email (.eml, .msg) | `mail-parser` | Subject and text bodies only — routing headers are identifier-shaped noise |
| txt, md, csv | built-in | |

### Stage 1 — shape and wordlist → `Identifier`, `Technical`

A transparent weighted sum over a retained feature vector: internal capitalisation,
digit/letter mixing, separator segments, unusual length, absence from the general
English wordlist, short all-caps, in-document frequency.

Deliberately **not** a classifier. There are no labels to train one on, and a learned
model would forfeit the reproducibility the whole crate exists to provide. `kep
explain` shows every component of every score.

It merges split multi-word product names (`MabSelect SuRe`) while refusing to merge
title-case headings or runs of codes.

### Stage 2 — definitions → `Technical`, `origin: Definition`

Schwartz–Hearst (2003), implemented from the paper so behaviour pins to the version
stamp. Finds `long form (short form)` and `short form (long form)` within clause scope
and emits both halves with the `expansion` attached. Runs whatever the language — the
matching is orthographic, not lexical.

### Stage 3 — topical → `Topical`, behind the prose gate

YAKE (Campos et al., 2020), implemented from the paper. Five per-term features —
casing, position, frequency normalisation, relatedness to context, sentence dispersion
— combined over contiguous n-grams. Corpus-blind by construction: every feature comes
from the single document, so there is no IDF and no background corpus.

Phrases are bounded by stopwords, punctuation and line breaks; near-duplicate phrasings
are collapsed. YAKE scores are lower-is-better, so the emitted `score` is re-signed to
`1/(1+s)` — ordering preserved, and higher still means stronger, as everywhere else.

---

## Reproducibility

The pipeline is **corpus-blind**: every score derives from the document itself plus
pinned static resources. No document-frequency table, no IDF, no cross-document state.
A document's keyword set is a pure function of `(bytes, config, resources)`.

`PipelineVersion` is a computed blake3 fingerprint of exactly that triple — never a
hand-maintained string, so it cannot silently drift. It covers:

- the crate version and a hand-bumped `LOGIC_REVISION`;
- the resolved versions of extraction-relevant dependencies, read from `Cargo.lock`;
- every `Config` field, in an explicit fixed order;
- the digests of the active wordlist, stopword list and lemma list.

**Store it alongside any keywords you cache.** If it changes, your stored keywords were
produced by different code and should be regenerated. `kep version` prints it;
`PipelineVersion::short()` gives a 6-byte form for logs and filenames.

The English frequency wordlist, stopword list and lemma exception list are compiled into
the binary, so they cannot drift and need no deployment step. Each is regenerated by a
committed script (`scripts/fetch_*.sh`) from a named, permissively licensed source, so
what is in them is derived rather than asserted — see `NOTICE`. Runtime overrides go
through `Resources::with_overrides`, and every override's digest folds into the stamp
exactly as the embedded one does.

---

## What this is *not* for

Corpus-relative ranking, storage, deduplication policy, triage scoring, and any "is this
document worth a closer look" decision belong to the consuming system.

Out of scope by design: OCR (a scanned PDF is *reported*, not silently skipped), IDF or
any cross-document statistic, and trusting file extensions.

Lemmatisation is *partly* in scope, and the boundary matters. Surfaces, normalised forms
and offsets are never touched — that is what would mangle `DS-2291`, and it would also
break the guarantee that an offset resolves to the keyword it belongs to. Only the
**wordlist lookup key** is lemmatised, because the embedded list holds one surface form
per entry: `automation` is present and `automations` is not, so without it every plural,
participle and possessive in a document reads as technical vocabulary. Digit-bearing
tokens are classified as identifiers before the lookup is consulted, so identifiers never
reach it.

It is lemmatisation, not stemming. A stemmer produces non-words (`automat`) that a
surface-form list can never match, and its tie-breaking is a dependency that could move
`PipelineVersion` on an upgrade. The implementation is WordNet's *morphy* in both halves:
suffix detachment for regular inflection (`running` → `run`, `companies` → `company`,
`generalises` → `generalise`), and WordNet's exception lists for the irregulars no rule
can reach — `ran` → `run`, `mice` → `mouse`, `analyses` → `analysis`, `indices` → `index`.
That second half matters more than it sounds: without it `appendices`, `syntheses`,
`addenda`, `curricula` and `formulae` are each absent from the wordlist, score
`absent_from_wordlist` 1.0 for a shape total of 0.40, and clear the 0.38 technical
threshold. A report containing the word `appendices` emitted it as a technical term.

Lemmatisation reaches only lemmas the wordlist actually holds, which is the line between
fixing the lookup and growing the list: `vertices` lemmatises to `vertex`, absent at the
default cutoff, so it stays a technical candidate. The stopword lookup is deliberately
**not** lemmatised — that set is a closed list of function words, and reducing into it
would swallow real ones.

**This library cannot prove a shortlisting funnel works.** Where keyword extraction is
used to narrow a corpus before an expensive model-written summary, whether that actually
reduces spend is an extrinsic property of the *consuming* system, measurable only with
real documents and a real query matcher. Nothing measured in this repository is evidence
about that cost. The crate is evaluated intrinsically and only intrinsically — did it
extract the right things, reproducibly.

**Personal names are emitted with no special handling.** If your documents contain
them, your keyword store contains personal data and must be reachable by an erasure
request. The stable content hash and offsets in the output are what make that possible.

---

## Testing

Tests are organised as **instruments**, each settling one question with no human
judgement required:

| File | Question it settles |
|------|---------------------|
| `tests/parsing.rs` | Does the right text come out of each format? Fixtures are generated by PyMuPDF — a different implementation from the reader under test, so a pass means two independent codebases agree |
| `tests/determinism.rs` | Is the reproducibility guarantee real? Repeated extraction must be byte-identical |
| `tests/cross_format.rs` | Is canonicalisation silently format-dependent? Same content as PDF, docx and text should agree. **Reported, not gated** — some divergence is legitimate |
| `tests/cli.rs` | Does the binary report every document, in a stable form, whatever the shell does to stdout? Includes: every empty kind states why it is empty, and every `--format` value is exercised end to end |
| `tests/readme_examples.rs` | Does every code example in this README compile and run? |

```sh
cargo test
cargo clippy --all-targets
```

Fixtures are committed, so tests need no generation step; the generator scripts are
committed too, so fixtures can be reviewed rather than trusted as opaque binaries.

Not yet built:

- The **identifier-injection instrument** that would settle whether `wordlist_size` should
  be 65 000 and `acronym_wordlist_depth` 20 000. Both are values read off the separation
  in the embedded list, not measured optima.
- The **Inspec/SemEval keyphrase benchmark** that would settle `thresholds.topical`. Now
  that the prose gate no longer rejects wrapped PDFs, Stage 3 produces output on real
  documents for the first time, and the first thing it shows is that the admitted set is
  entirely unigrams ordered close to raw frequency. That is a measurement problem, not a
  tuning one, and it is what this instrument is for.
- A **gold set**: 10–20 representative documents with hand-marked keywords, and a
  precision/recall instrument over them. Without it, any change to weights or thresholds
  is unfalsifiable, and every value in `Config` is defended by argument rather than
  evidence.
- **`pdf_oxide::extract_structured`** in the PDF path. `StructuredRegion` carries a
  `RegionRole` (`BodyBlock`, `StructuralHeading`, `Header`, `Footer`, `PageNumber`,
  `Artifact`) and a `column_index`, which would let the parser drop page furniture by role
  rather than trusting `strip_running_headers_footers`, honour columns instead of
  flattening them, and give the prose gate a real body-text ratio. Deferred deliberately:
  it is PDF-only, so it puts `tests/cross_format.rs` in tension by construction; it widens
  the surface exposed to a dependency pinned precisely because it has no output-stability
  commitment; and it does **not** replace the soft-wrap rule, because a `BodyBlock`'s text
  still contains newlines. It should land against the gold set, not before it.

Until those exist, treat the defaults as reasonable starting points rather than measured
optima.

## License

MIT. See `LICENSE` and `NOTICE`.
