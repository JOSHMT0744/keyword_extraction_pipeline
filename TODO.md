# TODO

Work deferred from the extraction-quality review of 2026-09-11, which produced
`ff6be68` (`LOGIC_REVISION` 6). Each item states the decision it settles and what has to
be true before it can start — several are blocked on the same thing, and starting them
out of order means tuning numbers against one PDF.

Context for all of it: the review traced every keyword the pipeline emitted for
`corpus/Armature_Value_Proposition.pdf` back to its features. `ff6be68` fixed the prose
gate measuring render-layout line breaks, and three defects in Stage 1's
absent-from-wordlist test. What remains is mostly *unmeasurable* rather than unknown.

---

## 1. Build a gold set — blocks 2, 3 and 4

**Settles:** whether any change to weights, thresholds or resources is an improvement.

10–20 representative documents with hand-marked keywords, plus a precision/recall
instrument alongside the existing ones in `tests/`. Representative means spanning the
formats the crate actually claims: born-digital PDF (single and multi-column), docx,
pptx, xlsx, eml, and at least one non-English document.

Until this exists, every value in `Config` is defended by argument rather than evidence,
and `README.md` says so. Three separate items below are blocked on it. **Do this first.**

Note when assembling: mark identifiers, technical terms and topical keyphrases
*separately*. They are different kinds with non-comparable scores, and a single merged
list of "good keywords" cannot measure the arms independently.

---

## 2. `thresholds.topical` and the shape of Stage 3's output

**Blocked on:** item 1, or the Inspec/SemEval benchmark.
**Settles:** whether 0.15 is a threshold or an accident.

Stage 3 now runs on real documents for the first time, and its first output is bad in a
specific, diagnosable way. On the corpus document it admits **18 keywords, every one a
unigram**, ordered close to raw frequency:

```
ARMATURE 0.978 x18 | One 0.962 x16 | work 0.953 x10 | every 0.928 x8
job 0.924 x8 | person 0.922 x7 | time 0.920 x7 | week 0.919 x7
organisation 0.913 x6 | data 0.904 x6 | SOURCE 0.898 x5 | runs 0.891 x5
first 0.890 x5 | research 0.887 x5 | program 0.882 x5 | already 0.879 x5
```

`One`, `every`, `time`, `first` and `already` are not topics. And not one multi-word
phrase survived, despite `yake_ngram_max = 3`.

The hypothesis worth testing first: a candidate's score is `Π S(w) / (TF · (1 + Σ S(w)))`,
so a phrase's numerator is a *product* of term scores. If term scores are typically above
1, every phrase is structurally larger than any of its unigrams and an upper bound of 0.15
cuts precisely the candidates YAKE exists to produce. Check the actual distribution of
`score_candidate` output by n-gram length before changing anything — the fix may be the
threshold, or it may be that the combination needs length normalisation.

**Deliberately not tuned in `ff6be68`.** Moving it against one marketing PDF is how you
overfit to one marketing PDF.

---

## 3. The identifier-injection instrument

**Blocked on:** item 1.
**Settles:** `wordlist_size` (65 000) and `acronym_wordlist_depth` (20 000).

Both are values read off the separation in the embedded list, not measured optima, and
both say so in their doc comments. `wordlist_size` was placed between `specification`
(55 568) and `chromatography` (78 523); `acronym_wordlist_depth` between `labs` (8 289)
and `sop` (39 910). Sweep both.

Be aware when sweeping `wordlist_size`: the 80k list contains domain vocabulary, so list
*depth* carries no signal at the cutoff — `chromatography` (78 523), `armature` (69 330)
and `headcount` (79 839) sit in the same band. This is why graded
absent-from-wordlist was rejected (see Settled below).

---

## 4. `pdf_oxide::extract_structured` in the PDF path

**Blocked on:** item 1 — it changes extracted text for every PDF at once, so its effect is
unmeasurable without a baseline.
**Settles:** whether page furniture, columns and body-text detection should come from the
parser rather than from our heuristics.

`StructuredRegion` carries a `RegionRole` (`BodyBlock`, `StructuralHeading { level }`,
`MarginalLabel`, `Header`, `Footer`, `PageNumber`, `Artifact`) and a `column_index`. Three
things we currently approximate:

- drop page furniture **by role**, instead of trusting `strip_running_headers_footers`
- honour `column_index` instead of flattening columns via `ReadingOrderMode::ColumnAware`
- give the prose gate a real body-text ratio instead of `looks_like_a_table_row`

Three things to weigh against that, established during the review:

- It is **PDF-only**. The prose gate and Stage 3 run on docx/pptx/eml/txt too, so this
  creates a second path — and `tests/cross_format.rs`, whose whole job is asking "is
  canonicalisation silently format-dependent?", would report divergence by construction.
- `pdf_oxide` is pinned `=0.3.78` precisely because it ships roughly a release every four
  days with no output-stability commitment. Today we depend on one method; this would add
  their heading heuristic, column detector and artifact classifier as three more unstable
  surfaces. `PARSER_VERSIONS` makes the change visible, not small.
- It does **not** replace `tokenize::clauses`. A `BodyBlock`'s `text` joins spans "with
  spaces or newlines as appropriate", so the soft-wrap question survives intact. It
  improves the *inputs* to the clause rule.

Also worth knowing before starting: `corpus/Armature_Value_Proposition.pdf` is **untagged**
(XeTeX/xdvipdfmx, PDF 1.5, zero `StructTreeRoot`/`MarkInfo`/`BDC`), so
`extract_hierarchical_content` returns `None` on it and `section_id` is `None`. On that
document every role would come from font-size and geometry heuristics. Tagged-ness varies
by producer — Word emits tagged PDFs, LaTeX generally does not — so the heuristic path is
needed regardless.

---

## 5. Residual false positives, and the corpus question behind them

**Blocked on:** item 1, and on a product decision that is not the crate's to make.

After `ff6be68` the corpus document still emits `auditable`, `inspectable`, `headcount`
and `dataset`/`Datasets`. These are ordinary English words genuinely absent from a
65 000-word general list. Nothing corpus-blind removes them — that is the standing cost of
the corpus-blindness decision, not a bug to be patched.

Two ways out, both large, neither to be taken without evidence:

- **A frozen, offline-built document-frequency artifact**, shipped like `wordlist.txt`
  with its digest folded into `PipelineVersion`. Purity survives; it is functionally a
  much better wordlist with graded rather than binary membership.
- **A live DF table over the ingested corpus.** Real IDF, but extraction stops being a
  pure function of the document, and `PipelineVersion` needs an epoching scheme or every
  new document invalidates every stored keyword set.

Revisit only if the gold set shows the residual actually costs the consumer something.
See Settled below for why this was not done now.

---

## 6. Unexamined, low priority

Noticed during the review, not investigated:

- **`topical::deduplicate` compares whole normalised strings** with Levenshtein at 0.8.
  For short surfaces that is a coarse test — check it is not collapsing genuinely distinct
  four- and five-character keywords.
- **`shape::is_phrase_head` requires an uppercase first character**, so an all-lowercase
  multi-word technical term can never merge. May be correct; has never been examined.
- **`in_document_frequency` is weighted 0.02 and saturates at four occurrences.** At a
  0.38 threshold it is a tie-breaker rather than evidence. Left alone deliberately (see
  Settled); revisit with the gold set if ranking within a kind looks frequency-blind.

---

## 7. Housekeeping

- **`.gitignore` has a typo.** It carries `.corpus/.`, which matches nothing —
  `corpus/` is still untracked and showing in `git status`. Intended rule is presumably
  `corpus/`. `keywords.jsonl` is untracked and unignored too; decide whether run output
  belongs in the repo at all.
- `.gitignore` is missing a trailing newline.

---

## Settled — do not relitigate without new evidence

Decisions taken during the 2026-09-11 review, recorded so they are not re-proposed.

| Decision | Settled as | Why |
|---|---|---|
| Corpus DF / IDF table | **No**, for now | Corpus-blindness is a documented invariant (`lib.rs`, `README.md`). It also would not have fixed any keyword that prompted the review: Stage 3 never ran, and Stage 1 has no statistical component. See item 5. |
| Reflow canonical PDF text | **No** | Moves every byte offset, and erodes the page-line information `shape::same_line` and `topical::is_phrase` need to avoid splicing table cells. Healing is derived on demand in `tokenize::clauses` instead, keeping both views available. |
| Graded `absent_from_wordlist` | **No — dead on arrival** | Absent-entirely = 1.0, past-the-cutoff = 0.5 would demote `armature` (69 330) and `headcount` (79 839) — but `chromatography` sits at 78 523, in the same band. List depth carries no signal at the cutoff. |
| Lower `absent_from_wordlist` to 0.35 to force a second signal | **No** | Would wipe out most residual noise, but kills single-occurrence single-word technical terms in principle, `chromatography` included. That is the red line the weighting exists to protect. |
| Raise `in_document_frequency` | **No** | `ARMATURE` (x18) reached technical rank 0 because `CODE` and `TODAY` stopped outscoring it, not because frequency was reweighted. No number changed by eye. |
| `-able` in the wordlist reduction | **No** | Takes `auditable` and `inspectable`, but equally `injectable` (`inject` 11 324) and `filterable` (`filter` 9 329). `-s`/`-ed`/`-ing` make the same word again; `-able` makes a different one. Same argument rules out `-ly`, `-ness`, `-ment`. |
| Role-based furniture dropping now | **No** | Belongs with item 4; doing it alone means two separate changes to PDF text instead of one. |
