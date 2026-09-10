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
Nothing measured here is evidence about downstream cost or relevance; the crate is
evaluated intrinsically — did it extract the right things, reproducibly.

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
            kw.surface, kw.kind, kw.origin, kw.score, kw.rank, kw.frequency,
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
assert_eq!(&text[span.clone()], result.keywords[0].surface);
```

Canonicalisation is idempotent and deterministic, so the string is byte-identical to
the one the offsets were computed against.

## CLI

`kep` re-extracts a directory of documents offline — for corpus passes and for
eyeballing output during tuning.

```
kep <path>... [--pretty]
```

Directories are walked. One JSON object per document is written to stdout (with an
added `path` field), including for documents that yielded nothing. Exit status is
non-zero if any path could not be read.

```sh
cargo run --release --bin kep -- ./corpus --pretty
```

## Pipeline

| Stage | What happens |
|-------|--------------|
| **Parse** | Bytes → raw text. Format sniffed from magic bytes and content shape, never the extension. One backend per format behind a `TextExtractor` trait. |
| **Canonicalise** | Newline and page-break normalisation, NFKC, typographic-hyphen folding, soft-hyphen removal, line-break dehyphenation, email quote/signature stripping, whitespace collapse. Idempotent. Every keyword offset is relative to this output. |
| **Gates** | No text layer (scanned PDF) → `NoTextLayer`. Below `min_content_length` → `TooShort`. Both are recorded outcomes, not errors. |
| **Language detect** | `lingua` over 7 European languages, deterministic. English runs the full lane set; other languages degrade *visibly* to Lane 1 only, recorded as `fully_supported: false`. |
| **Lanes** | Complementary extractors (see below). All enabled lanes run; the consumer filters on `Kind` rather than a lane being selected out. |

### Formats

| Format | Backend | Notes |
|--------|---------|-------|
| PDF | `pdf_oxide` (pinned `=0.3.78`) | Born-digital only. The exact pin feeds `PipelineVersion`, so an upgrade is a deliberate re-extraction event. |
| docx, pptx | `quick-xml` over the zip container | XML read directly. |
| xlsx | `calamine` | Handles shared strings and cell types. Always treated as tabular. |
| email (.eml, .msg) | `mail-parser` | Subject and text bodies only; routing headers are identifier-shaped noise. |
| txt, md, csv | built-in | Encoding decided at parse time. |

### Lanes

| Lane | `Origin` | `Kind` | Status |
|------|----------|--------|--------|
| **1 — shape & wordlist** | `Shape` | `Identifier`, `Technical` | **Implemented.** Transparent weighted sum over a retained feature vector (internal caps, digit/letter mix, separator segments, length, absence from the general English wordlist, short all-caps, in-document frequency). Deliberately not a classifier — there are no labels, and a learned model would forfeit reproducibility. Merges split multi-word product names (`MabSelect SuRe`) while refusing to merge title-case headings or runs of codes. |
| **2 — definitions** | `Definition` | `Technical` | Designed, gated by `enable_definitions`; Schwartz–Hearst pass not yet wired in. |
| **3 — topical** | `Statistic` | `Topical` | Designed, gated by `enable_topical` behind a deterministic prose heuristic; YAKE keyphrase extraction not yet wired in. |

Scores are comparable **within** a `Kind` and meaningless across kinds (shape scores
and YAKE scores are on unrelated scales). `rank` is 0-based within a kind; output is
uncapped above a per-kind threshold so a consumer can impose its own top-N without
re-extracting.

## Output

`DocumentResult`:

| Field | Meaning |
|-------|---------|
| `status` | `Ok`, `NoTextLayer`, `Encrypted`, `UnsupportedFormat`, `ParseError(String)`, `TooShort`. |
| `pipeline_version` | The `PipelineVersion` these keywords were produced under. |
| `hash_exact` | blake3 over the raw input bytes. |
| `hash_canonical` | blake3 over the canonical text — the natural cache key for everything downstream. |
| `own_content_length` | Characters of canonical text. |
| `language` / `language_confidence` | Detected language and whether the full lane set ran. |
| `keywords` | Flat, ranked, uncapped above a per-kind threshold. Filter on `kind`. |

Each `Keyword` carries `surface`, `normalised` (NFKC + casefold, never stemmed),
`kind`, `origin`, `score`, `rank`, `frequency`, and `offsets` (byte ranges into the
canonical text).

## Configuration

Every `Config` field feeds `PipelineVersion` in a fixed order, so changing any of them
changes the stamp and invalidates cached keyword sets. Key fields:

| Field | Default | Purpose |
|-------|---------|---------|
| `min_content_length` | 16 | "Nothing here at all" cutoff — deliberately near-zero, not a readability floor. |
| `no_text_layer_threshold` | 32 | Chars below which a PDF is treated as having no text layer. |
| `wordlist_size` | 65 000 | How many frequency-ranked words count as ordinary English. The single scalar governing Lane 1's `absent_from_wordlist` feature; intended to be swept, not argued over. |
| `thresholds` | per-kind | Emission cutoffs for identifier / technical / topical (topical is *lower is better*). |
| `shape_weights` | see `src/config.rs` | Lane 1 feature weights; sum to 1.0 by construction. |
| `prose` | — | Deterministic heuristic gating Lane 3 (mean sentence length, stopword ratio, table-line ratio, minimum tokens). |
| `strip_quoted_blocks` | true | Strip quoted blocks and signatures from email. |
| `enable_definitions`, `enable_topical` | true | Lane 2 / Lane 3 switches. |
| `yake_ngram_max` | 3 | Max phrase length for Lane 3. |

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
  by config or dependencies — a new lane, an altered canonicalisation step);
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

```sh
cargo test
```

Fixtures are committed, so tests need no generation step; the generator scripts are
committed too so the fixtures can be reviewed rather than trusted as opaque binaries.

## License

MIT. See `LICENSE` and `NOTICE`.
