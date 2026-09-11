# keyword_extraction_pipeline

Deterministic tier-1 keyword and identifier extraction for heterogeneous document
corpora. Raw file in, ranked keywords out.

The pipeline is **corpus-blind**: every score is derived from the document itself plus
pinned static resources. There is no document-frequency table, no IDF, and no
cross-document state anywhere, so a document's keyword set is a pure function of
`(bytes, config, resources)`. That triple is fingerprinted into a single
[`PipelineVersion`](#versioning) stamp, which is what makes the guarantee checkable:
the same bytes under the same version always produce the same keywords.

## What it is for

Finding the exact-matchable and distinctive terms in a document — batch codes, SOP
numbers, cell lines, instrument IDs, product names, domain vocabulary — without a
curated gazetteer of schemes, and reproducibly enough to cache the result by hash.

## What it is *not* for

Corpus-relative ranking, storage, deduplication policy, triage scoring, and any
"is this document worth a closer look" decision belong to the consuming system.

**This library cannot prove the funnel works.** Where keyword extraction is used to
shortlist documents before an expensive model-written summary, whether that
shortlisting actually reduces spend is an extrinsic property of the *consuming*
system, measurable only with real documents and a real query matcher. Nothing
measured in this repository is evidence about that cost. The crate is evaluated
intrinsically and only intrinsically — did it extract the right things, reproducibly.

Out of scope by design: OCR (a scanned PDF is *reported* as such, not silently
skipped), IDF or any cross-document statistic, stemming (it mangles alphanumeric
identifiers), and file-extension trust (formats are sniffed from bytes).

## Install

```toml
[dependencies]
keyword_extraction_pipeline = { git = "https://github.com/JOSHMT0744/keyword_extraction_pipeline" }
```

Requires Rust 1.88+.

## Library usage

```rust
use keyword_extraction_pipeline::{extract, Config, FormatHint, Resources};

let bytes = std::fs::read("report.pdf")?;

let cfg = Config::default();
let res = Resources::for_config(&cfg); // NB: Resources::default() only matches Config::default()

let result = extract(&bytes, FormatHint::Sniff, &cfg, &res);

if result.status.is_ok() {
    for kw in &result.keywords {
        println!(
            "{:<24} {:?}/{:?}  score={:.3} rank={} freq={}",
            kw.original_keyword, kw.kind, kw.origin, kw.score, kw.rank, kw.frequency,
        );
    }
} else {
    // Not an error — a recorded outcome. e.g. NoTextLayer, Encrypted, TooShort.
    eprintln!("no keywords: {:?}", result.status);
}
```

`extract` never returns `Err` for a document that merely cannot be read. An unreadable
document is a [`DocumentResult`](#output) carrying a [`DocumentStatus`] that explains
why, plus the input hash and version stamp, so the consumer records it rather than
losing it. `Err` is reserved for a caller mistake.

Full document text is never returned. To resolve a keyword's byte `offsets`, regenerate
the exact canonical string the offsets refer to:

```rust
use keyword_extraction_pipeline::{canonicalise, Config, FormatHint};

let text = canonicalise(&bytes, FormatHint::Sniff, &Config::default())?;
let span = &result.keywords[0].offsets[0];
assert_eq!(&text[span.clone()], result.keywords[0].original_keyword);
```

Canonicalisation is idempotent and deterministic, so the string is byte-identical to
the one the offsets were computed against.

## CLI

`kep` re-extracts a directory of documents offline — for corpus passes and for
eyeballing output during tuning.

| Command | Purpose |
|---------|---------|
| `kep extract <path>...` | Walk paths and emit keywords. |
| `kep explain <file>` | Account for one document's scores, feature by feature, including what a lower threshold would have admitted. |
| `kep config` | Print the effective configuration as JSON, ready to edit and pass back with `--config-file`. |
| `kep version` | Crate version, logic revision, wordlist extent, and the resulting `pipeline_version`. |

**Data goes to stdout, commentary goes to stderr** — the run header, the status
trailer and every warning — so `kep extract ./corpus > out.jsonl` is a clean file
while the operator still sees what happened.

`--format` is `table`, `jsonl`, `json` or `csv`, defaulting to `table` on a terminal
and `jsonl` when redirected. The flag is authoritative and the header states which
was used; the choice affects rendering only, never what was extracted.

```sh
cargo run --release --bin kep -- extract ./corpus > keywords.jsonl
cargo run --release --bin kep -- extract --format table --kind identifier ./corpus
cargo run --release --bin kep -- explain ./corpus/report.pdf
```

Exit status is `0` when every path was read, `1` when one could not be, and `2` under
`--strict` if any document returned a status other than `Ok`. A `NoTextLayer` result
is the pipeline working correctly, so by default it does not fail the run.

### The CLI's JSON is a presentation format

`kep`'s JSON is **not** a serde round-trip of `DocumentResult`, and
`serde_json::from_str::<DocumentResult>` will not accept it. Two differences, both
deliberate:

- **Keywords are grouped by kind** (`identifier`, `technical`, `topical`) rather than
  being one flat list. Scores are meaningless across kinds, and grouping makes that
  structural instead of documentary.
- **Digests are lowercase hex strings.** The library type serialises them as hex too,
  so it does round-trip; the grouping is what the CLI adds on top.

Use the library API if you want the typed result back.

## Pipeline

| Step | What happens |
|------|--------------|
| **Parse** | Bytes → raw text. Format sniffed from magic bytes and content shape, never the extension. One backend per format behind a `TextExtractor` trait. |
| **Canonicalise** | Newline and page-break normalisation, NFKC, typographic-hyphen folding, soft-hyphen removal, line-break dehyphenation, email quote/signature stripping, whitespace collapse. Idempotent. Every keyword offset is relative to this output. |
| **Gates** | No text layer (scanned PDF) → `NoTextLayer`. Below `min_content_length` → `TooShort`. Both are recorded outcomes, not errors. |
| **Language detect** | `lingua` over 7 European languages, deterministic. English runs the full stage set; other languages degrade *visibly* to Stage 1 only, recorded as `fully_supported: false`. |
| **Stages** | Complementary extractors (see below). All enabled stages run; the consumer filters on `Kind` rather than a stage being selected out. |

### Formats

| Format | Backend | Notes |
|--------|---------|-------|
| PDF | `pdf_oxide` (pinned `=0.3.78`) | Born-digital only. The exact pin feeds `PipelineVersion`, so an upgrade is a deliberate re-extraction event. |
| docx, pptx | `quick-xml` over the zip container | XML read directly. |
| xlsx | `calamine` | Handles shared strings and cell types. Always treated as tabular. |
| email (.eml, .msg) | `mail-parser` | Subject and text bodies only; routing headers are identifier-shaped noise. |
| txt, md, csv | built-in | Encoding decided at parse time. |

### Stages

| Stage | `Origin` | `Kind` | Status |
|------|----------|--------|--------|
| **1 — shape & wordlist** | `Shape` | `Identifier`, `Technical` | **Implemented.** Transparent weighted sum over a retained feature vector (internal caps, digit/letter mix, separator segments, length, absence from the general English wordlist, short all-caps, in-document frequency). Deliberately not a classifier — there are no labels, and a learned model would forfeit reproducibility. Merges split multi-word product names (`MabSelect SuRe`) while refusing to merge title-case headings or runs of codes. |
| **2 — definitions** | `Definition` | `Technical` | **Implemented**, gated by `enable_definitions`. Schwartz–Hearst (2003) implemented directly from the paper rather than pulled from a crate, so behaviour pins to `PipelineVersion`. Finds `long form (short form)` and `short form (long form)` within clause scope, emitting both halves with the canonical `expansion` attached. Runs whatever the language — the matching is orthographic, not lexical. Offsets record the *definition site*, not every occurrence; where Stage 1 also emitted the term, that record carries the full occurrence set. |
| **3 — topical** | `Statistic` | `Topical` | **Implemented**, gated by `enable_topical` *and* the prose gate. YAKE (Campos et al., 2020) implemented directly from the paper — five per-term features (casing, position, frequency normalisation, relatedness to context, sentence dispersion) combined over contiguous n-grams up to `yake_ngram_max`. Corpus-blind by construction: every feature comes from the single document. Phrases are bounded by stopwords, punctuation and line breaks. Near-duplicate phrasings are collapsed. **The default `thresholds.topical` of 0.15 is an unvalidated guess** pending the keyphrase benchmark. |

Scores are comparable **within** a `Kind` and meaningless across kinds (shape scores
and YAKE scores are on unrelated scales). `rank` is 0-based within a kind; output is
uncapped above a per-kind threshold so a consumer can impose its own top-N without
re-extracting.

YAKE's own scores are *lower is better*, which `thresholds.topical` reflects — it is an
upper bound, not a floor. The emitted `score` is re-signed to `1/(1+s)` so that one
ranking function serves every stage and higher always means a stronger claim.

### The prose gate

Stage 3 assumes running text. On a spreadsheet it emits column headers as topics; on an
email footer it emits the disclaimer. Both are confident, plausible and wrong — worse
than emitting nothing, because nothing is visibly nothing.

`prose::assess` rules on each document using `ProseParams` (mean sentence length,
stopword ratio, table-line ratio, minimum tokens), short-circuiting on formats that are
inherently tabular and on non-English documents. **The verdict travels on
`DocumentResult::prose`** rather than being consumed and discarded, so an empty
`Topical` list always carries its reason:

```
Topical  —  prose gate: the format is inherently tabular
            (mean sentence 2.2 tokens, 0% stopwords, 100% table-like lines, 9 tokens)
```

The measurements are reported whether or not they decided the outcome, so a threshold
can be moved against real numbers rather than guessed at.

## Output

`DocumentResult`:

| Field | Meaning |
|-------|---------|
| `status` | `Ok`, `NoTextLayer`, `Encrypted`, `UnsupportedFormat`, `ParseError(String)`, `TooShort`. |
| `pipeline_version` | The `PipelineVersion` these keywords were produced under. |
| `hash_exact` | blake3 over the raw input bytes. |
| `hash_canonical` | blake3 over the canonical text — the natural cache key for everything downstream. |
| `own_content_length` | Characters of canonical text. |
| `language` / `language_confidence` | Detected language and whether the full stage set ran. |
| `keywords` | Flat, ranked, uncapped above a per-kind threshold. Filter on `kind`. |

Each `Keyword` carries `original_keyword`, `normalised` (NFKC + casefold, never stemmed),
`kind`, `origin`, `score`, `rank`, `frequency`, and `offsets` (byte ranges into the
canonical text). Two optional fields are present only when they apply: `expansion`,
the canonical long form when a definition stage resolved one, and `features`, Stage 1's
retained feature vector when `Config::retain_features` is on.

### Stages may emit the same term twice

A term can be reached by more than one route — `SOP` from orthography, and again from
`Standard Operating Procedure (SOP)`. **Those are kept as separate records
distinguished by `origin`, not merged.** An orthographic guess and a definitional
match are different claims about the same string, and collapsing them would discard
which one was made.

The cost is that `keywords` can contain the same term twice within a `Kind`, and a
consumer that counts naively will double-count. If you want uniqueness, deduplicate
on `(normalised, kind)` keeping the record whose `origin` you trust most —
`Definition` is the stronger evidence:

```rust
let mut seen = std::collections::HashSet::new();
result.keywords.retain(|k| seen.insert((k.normalised.clone(), k.kind)));
```

`Origin::Definition` entries score `1.0` by construction: a definition is categorical
evidence rather than another weighted vote. This is the one place two score scales
coexist inside a single `Kind`, which is why `origin` is on every record.

## Configuration

Every `Config` field feeds `PipelineVersion` in a fixed order, so changing any of them
changes the stamp and invalidates cached keyword sets. Key fields:

| Field | Default | Purpose |
|-------|---------|---------|
| `min_content_length` | 16 | "Nothing here at all" cutoff — deliberately near-zero, not a readability floor. |
| `no_text_layer_threshold` | 32 | Chars below which a PDF is treated as having no text layer. |
| `wordlist_size` | 65 000 | How many frequency-ranked words count as ordinary English. The single scalar governing Stage 1's `absent_from_wordlist` feature; intended to be swept, not argued over. |
| `thresholds` | per-kind | Emission cutoffs for identifier / technical / topical (topical is *lower is better*). |
| `shape_weights` | see `src/config.rs` | Stage 1 feature weights; sum to 1.0 by construction. |
| `prose` | — | Deterministic heuristic gating Stage 3 (mean sentence length, stopword ratio, table-line ratio, minimum tokens). |
| `strip_quoted_blocks` | true | Strip quoted blocks and signatures from email. |
| `enable_definitions`, `enable_topical` | true | Stage 2 / Stage 3 switches. |
| `yake_ngram_max` | 3 | Max phrase length for Stage 3. |

## Resources

The general English frequency wordlist and stopword list are **compiled into the
binary**, so they cannot drift and need no deployment step. Runtime overrides are
supported via `Resources::with_overrides`; an override's digest folds into
`PipelineVersion` exactly as the embedded digest does, so it can never change output
silently.

The wordlist is frequency-ranked (order is rank) and deliberately *general* English,
never domain text. Regenerate it from the upstream source with
`scripts/fetch_wordlist.sh` (committed output, reproducible input — it is never fetched
at build or run time). See `NOTICE` for attribution.

## Versioning

`PipelineVersion` is a computed blake3 fingerprint, never a hand-maintained string.
It is derived from:

- the crate version and a hand-bumped `LOGIC_REVISION` (for logic changes not captured
  by config or dependencies — a new stage, an altered canonicalisation step);
- the resolved versions of extraction-relevant dependencies (`build.rs` reads these
  from `Cargo.lock`; only the crates in its `TRACKED` list, so unrelated dev-dependency
  bumps don't churn the stamp);
- every `Config` field, in an explicit fixed order;
- the digests of the active wordlist and stopword list.

`PipelineVersion::short()` gives a 6-byte hex form for logs and filenames.

## Testing

Tests are organised as **instruments**, each settling one question with no human
judgement required:

| Test file | Question |
|-----------|----------|
| `tests/parsing.rs` | Does the right text come out of each format? Fixtures are generated by `scripts/make_fixtures.py` using PyMuPDF — a different implementation from the reader under test, so a pass means two independent codebases agree. |
| `tests/determinism.rs` | Is the reproducibility guarantee real? Repeated extraction of every fixture must be byte-identical. |
| `tests/cross_format.rs` | Is canonicalisation silently format-dependent? The same content in PDF, docx and plain text should canonicalise to near-identical token sets. **Reported, not gated** — some divergence is legitimate. |
| `tests/cli.rs` | Does the binary report every document, in a stable form, whatever the shell does to its stdout? Covers the JSONL contract, hex digests, exit codes, `--strict`, config resolution by file and by flag agreeing on one stamp, and that **every empty kind states why it is empty**. |

```sh
cargo test
```

Fixtures are committed, so tests need no generation step; the generator scripts are
committed too so the fixtures can be reviewed rather than trusted as opaque binaries.

## License

MIT. See `LICENSE` and `NOTICE`.
