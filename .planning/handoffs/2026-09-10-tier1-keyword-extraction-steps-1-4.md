# Handoff — Tier-1 keyword extraction, steps 1–4 complete

**Date:** 2026-09-10
**Repo:** `/home/dell/armature/keyword_extraction_pipeline` (branch `main`, nothing committed yet — all work is untracked in the working tree)
**Plan:** `/home/dell/.claude/plans/develop-a-keyword-identifier-extraction-fluttering-spring.md` — read this first; the handoff assumes it.

**Purpose of this document:** enough context to replan steps 5, 6, 7 and 9 cold, without re-deriving decisions already settled or undoing corrections already made.

---

## 1. Status against the plan's build order

| Step | Description | State |
|---|---|---|
| 1 | Crate skeleton, `Config`, `PipelineVersion`, error/status types | **Done** |
| 2 | Parsing layer behind `TextExtractor`, fixtures, instrument 1 | **Done** |
| 3 | Canonicalisation, hashes, language detection, instruments 2 and 5 | **Done** |
| 4 | Lane 1 — shape + wordlist | **Done** |
| 5 | Instrument 3 — identifier injection; tune Lane 1 | **Not started** |
| 6 | Lane 2 — Schwartz–Hearst | **Not started** |
| 7 | Prose gate + Lane 3 YAKE + instrument 4 (Inspec/SemEval) | **Not started** |
| 8 | CLI | **Done — brought forward**, needed to inspect Lane 1 output |
| 9 | `criterion` benchmarks | **Not started** |

**73 tests passing, clippy clean.** `cargo test` and `cargo clippy --all-targets` are both green.

---

## 2. What exists

```
build.rs              scans Cargo.lock for tracked parser versions → PARSER_VERSIONS const
src/version.rs        PipelineVersion: blake3 fingerprint, LOGIC_REVISION = 1
src/config.rs         Config, Thresholds, ShapeWeights, ProseParams; explicit ordered feed()
src/resources.rs      frequency-ranked wordlist + stopwords, embedded + overridable
src/error.rs          ExtractError (caller-facing failures only)
src/types.rs          DocumentResult, DocumentStatus, Keyword, Kind, Origin, FormatHint
src/parse/            mod (sniff + dispatch), pdf, office (docx/pptx/xlsx), email, plain
src/canonical.rs      NFKC, de-hyphenation, quote/signature stripping — idempotent
src/language.rs       lingua, 7 EU languages for discrimination, English fully supported
src/tokenize.rs       identifier-preserving tokeniser + sentence splitter
src/lanes/shape.rs    Lane 1
src/bin/kep.rs        CLI: paths in, JSONL out
resources/            wordlist.txt (80k ranked), stopwords.txt
scripts/              make_fixtures.py, fetch_wordlist.sh
tests/                parsing.rs, determinism.rs, cross_format.rs, fixtures/
```

### Public API as built

```rust
pub fn extract(bytes: &[u8], hint: FormatHint, cfg: &Config, res: &Resources) -> DocumentResult;
pub fn canonicalise(bytes: &[u8], hint: FormatHint, cfg: &Config) -> Result<String, ExtractError>;
```

Differs from the plan in one way: `extract` takes `&Resources` explicitly rather than building them internally, because the wordlist cutoff is now a config parameter and the two must stay consistent. `extract` **never** returns `Err` — an unreadable document is a `DocumentResult` with a `DocumentStatus` explaining why, so the consumer records it instead of losing it.

`Keyword` carries `surface`, `normalised`, `kind`, `origin`, `score`, `rank`, `frequency`, `offsets`. Ranks are dense and scoped per `kind`; scores are never comparable across kinds.

---

## 3. Decisions made during implementation

These refine or correct the plan. **Do not re-litigate them without reading the reasoning.**

### 3.1 Wordlist is frequency-ranked, and its size is a tunable scalar

The plan said "SCOWL size 60 or equivalent". Built instead from **hermitdave/FrequencyWords** (MIT, 2018 English, OpenSubtitles-derived), top 80,000 alphabetic entries embedded in rank order, with `Config::wordlist_size` (default **65,000**) selecting the active prefix.

Rationale: list size trades recall against precision directly, and frequency-ranking turns "how big" into one scalar the step-5 sweep can measure rather than anyone arguing a value. Deliberately **general** English, never domain text — the user was explicit that a scientific corpus must not be assumed.

Measured separation that motivates 65k (recorded in `resources/wordlist.txt` header and pinned by tests in `src/resources.rs`):

| Ordinary formal English | rank | Domain vocabulary | rank |
|---|---|---|---|
| procedure | 3.5k | chromatography | 87k |
| invoice | 18k | immunoglobulin | 97k |
| compliance | 22.5k | elution | 1.29M |
| calibration | 45k | sepharose | absent |
| specification | 60k | superdex | absent |

Licence attribution is in `NOTICE`. Regenerate with `scripts/fetch_wordlist.sh`.

### 3.2 `TooShort` was separated from prose-length gating

`min_content_length` was 64 chars and rejected `simple.xlsx` (~58 chars). That was the *design* being wrong: an instrument report whose entire content is forty sample codes is short in characters and is the highest-value document for the identifier lane. `min_content_length` is now **16** and means "nothing here at all". "Too short for topical keyphrases" is a separate question, to be answered by `ProseParams::min_tokens` (default 120) at step 7.

Regression test: `tests/determinism.rs::an_identifier_dense_short_document_is_not_rejected_as_too_short`.

### 3.3 `ShapeWeights` were structurally wrong, not merely mistuned

Original weighting put `absent_from_wordlist` at 0.20. A purely alphabetic technical term like `chromatography` has no digits, no separators and no internal capitals — absence is its **only** signal — so no threshold above 0.20 could ever admit one, whatever it was set to.

Current weights (sum 1.0), in `src/config.rs`:

```
absent_from_wordlist 0.40   digit_letter_mix 0.25   internal_caps 0.12
separator_segments   0.12   short_all_caps   0.06   unusual_length 0.03
in_document_frequency 0.02
```

Thresholds: `identifier 0.55`, `technical 0.38` (deliberately just below the weight of absence alone), `topical 0.15` (unused until step 7).

### 3.4 Wordlist membership is case-aware, via acronym-vs-shouting discrimination

`SOP` was suppressed because "sop" is an ordinary lowercase word. Treating all-caps as absent instead admits every word of a shouted heading. The discriminator implemented is **context**: a token counts as an acronym when it is short (2–6 chars), all-caps, and *not* inside a run of three or more consecutive all-caps tokens. See `acronym_flags` in `src/lanes/shape.rs`.

Both directions are pinned: `an_acronym_among_lowercase_prose_survives_a_wordlist_collision` and `a_word_inside_a_shouted_run_is_not_promoted_to_an_acronym`.

**Known residual:** a short non-stopword inside a *two*-token capitalised run can still be promoted. Step 5's distractor set should quantify this.

### 3.5 Phrase merging refuses to cross a line break, and excludes digit-bearing tokens

Found by eyeballing CLI output, not by unit tests: the merger produced the phantom term `'DS-2291 HEK293T DS-2292'` from `simple.xlsx` by running off the end of one spreadsheet row into the next. **Token adjacency in the vector is not adjacency in the text.**

Two rules now: components must be on the same line (`same_line`), and digit-bearing tokens are excluded from merging entirely, since a run of codes is a list rather than a name. Pinned by `does_not_merge_across_a_line_break` and `does_not_merge_runs_of_identifiers_into_a_phantom_name`.

### 3.6 PDF backend confirmed working on both hard cases

`pdf_oxide` pinned `=0.3.78`, `ReadingOrderMode::ColumnAware`. `StructureTreeFirst` was rejected: it requires supplying MCID order manually and falls back to ColumnAware anyway. `has_text_layer()` gives `NoTextLayer` detection directly from the library. Two-column reading order does not interleave. Fixtures are generated by PyMuPDF, so agreement is between two independent implementations.

### 3.7 Version fingerprint covers transitive parser versions

`build.rs` finds `zip` and `quick-xml` three times each, because `pdf_oxide` pulls its own. Conservative-correct: a bump inside `pdf_oxide` can change extracted text. `LOGIC_REVISION` is bumped by hand only when extraction logic changes in a way config and dependency versions do not capture.

### 3.8 Language detection carries 7 languages purely to discriminate

English, French, German, Spanish, Italian, Dutch, Portuguese. A single-language detector reports English for everything with full confidence, which is not detection. Non-English is recorded as `fully_supported: false` so degradation is visible. There is deliberately **no wildcard match arm** in the code mapping, so adding a language feature is a compile error rather than a silent `"und"`.

---

## 4. What remains

### Step 5 — Instrument 3: identifier injection *(next, and it gates step 4's tuning)*

Plan section "Evaluation → 3. Identifier extraction". Build:

- **Difficulty tiers, reported separately, never aggregated.** Easy (obviously coded), Medium (word-like product names), Hard (bare numerics, lowercase codes, English collisions). The hard-tier number is the only one that carries information.
- **Distractor injection** — identifier-shaped non-identifiers (`v2.14.3`, hex digests, dates, ISBNs, phone numbers, URLs). Measures false emission, which recall-only injection cannot see.
- **Natural-position injection** — plants go into realistic sentence positions, never appended blocks.
- **Tier contents mined from public sources**, not authored by whoever built the extractor.
- **`wordlist_size` sweep.** This is where the 65k default gets confirmed or moved. Sweep at minimum 40k / 55k / 65k / 80k and report per-tier recall and distractor false-emission at each.
- Track **identifiers per 1000 tokens** across fixtures as a volume-regression signal.

**Scope change the user asked for, not yet reflected in the plan file.** The corpus must not be assumed scientific. The tiers need non-scientific identifier schemes alongside the lab ones: invoice and PO numbers, contract and case references, part numbers, ticket IDs (`JIRA-4417`), ISO standard references, SKUs, VAT/company numbers. The existing fixtures are *entirely* chromatography — that is a real gap in coverage, and the plan's hard-tier examples (`Titan`, `Blue`, `Superdex`) are all lab reagents. Widen both.

*Optional, ~20 lines, previously declined:* trivial baselines (digit-bearing regex; absent-from-wordlist) reported per tier, making "does the feature vector earn its complexity" explicit rather than implicit in the distractor number.

### Step 6 — Lane 2: Schwartz–Hearst

Implement directly from the paper (~200 lines), not a crate, so behaviour pins to `pipeline_version`. Emits `Kind::Technical` with `Origin::Definition`. Gives both the evidence a term is technical and its canonical expansion.

`src/tokenize.rs::sentences()` already exists and is tested — Schwartz–Hearst needs sentence scope, so that dependency is met. This lane is also the principled fix for the acronym/wordlist-collision problem in §3.4: `Standard Operating Procedure (SOP)` resolves `SOP` from context rather than orthography.

### Step 7 — Prose gate + Lane 3: YAKE + instrument 4

- **Prose heuristic** using `ProseParams` (already defined, unused): mean sentence length, stopword ratio, table-line ratio, min tokens. `SourceKind::is_inherently_tabular()` already exists and should short-circuit spreadsheets. Needs its own version coverage — it is already in `Config::feed`.
- **YAKE** implemented from the paper. `Config::yake_ngram_max` (default 3) exists.
- **Instrument 4** — Inspec + SemEval-2010, P/R/F1@5 and @10. Python harness as a **dev dependency only**. Primary job is a *correctness check*: YAKE's paper publishes numbers on both sets, so scoring materially below published YAKE is a bug, not a tuning question.

**Lanes union — no adoption gates.** All three lanes ship. The ablation is descriptive, not a go/no-go. Lane 1 emits Identifier/Technical and Lane 3 emits Topical; they are complements producing different kinds, not substitutes competing for slots. See the plan's "Lane composition: union, not selection".

### Step 9 — `criterion` benchmarks

Per-format throughput, published and tracked. **Not CI-enforced** — thresholds are machine-dependent and would produce false failures.

### Not yet done, small

- README does not yet carry the explicit boundary statement from the plan: *this library cannot prove the funnel works; nothing measured here is evidence about blurb cost.*
- Nothing is committed. `git status` shows the whole tree untracked.

---

## 5. Open risks carried forward

| Risk | State |
|---|---|
| No instrument for identifier **precision** on real-world noise | Step 5's distractors are the only planned coverage; volume tracking backs it up |
| Injection difficulty is chosen by us | Mitigate by mining tiers from public sources; not eliminated |
| Fixtures are entirely scientific | **Open** — must be widened at step 5 per the user's instruction |
| Acronym rule can promote a word in a two-token capitalised run | Quantify with step-5 distractors |
| `pdf_oxide` churns ~every 4 days | Exact pin + fingerprint + regression suite; upgrade is a deliberate re-extraction event |
| Personal names emitted untagged | Explicit decision; GDPR burden sits with the consumer |
| Nothing here is evidence the blurb funnel saves money | Explicit boundary; belongs to the consumer |

---

## 6. Verification

```bash
cd /home/dell/armature/keyword_extraction_pipeline
cargo test                       # 73 tests
cargo clippy --all-targets       # clean
cargo run --bin kep -- tests/fixtures --pretty   # eyeball Lane 1 output
python3 scripts/make_fixtures.py # regenerate fixtures (needs PyMuPDF)
./scripts/fetch_wordlist.sh      # regenerate the wordlist
```

Expected shape of CLI output on `simple.pdf`:

```
Identifier 0.722 'DS-2291'  0.722 'SOP-114'  0.650 'HEK293T'
Technical  0.535 'MabSelect SuRe'  0.520 'MabSelect'  0.415 'chromatography'
```

`scanned.pdf` must report `NoTextLayer`, never an empty keyword list.
