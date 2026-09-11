#!/usr/bin/env bash
# Regenerate resources/stopwords.txt from NLTK's English stopword corpus.
#
# Committed output, reproducible input — the same arrangement as fetch_wordlist.sh. The
# list is embedded in the binary and its digest feeds PipelineVersion, so it must never
# be fetched at build or run time.
#
# The list is DERIVED, not adopted. Two deterministic edits are applied to NLTK's 198
# entries, and both are printed when this runs so the diff is reviewable:
#
#   DROPPED — entries NLTK's tokeniser produces and ours cannot. NLTK splits `didn't`
#   into `did` + `n't`, leaving `didn` as a token, so its list carries the stems and
#   clitics of every negated contraction. Ours keeps `didn't` whole (see src/tokenize.rs),
#   so those entries can never match. One of them, `won`, is a real English verb that
#   would otherwise be suppressed. The rule is mechanical: drop E where E+"'t" is also
#   in the list, plus the bare clitics.
#
#   ADDED — the modals and prepositions NLTK omits. This is the one hand-made judgement
#   in the file, so it is spelled out here rather than buried in the data: without them
#   YAKE admits phrases headed by a function word (`may require calibration`), and the
#   prose gate undercounts function-word density in formal register, which is exactly
#   the register this crate's documents are written in.
#
# src/resources.rs asserts the invariant behind the DROPPED rule, so a future edit that
# reintroduces an unmatchable entry fails the suite rather than failing silently.
set -euo pipefail

URL="https://raw.githubusercontent.com/nltk/nltk_data/gh-pages/packages/corpora/stopwords.zip"

ADDED="may might must shall should would could cannot ought
       across among upon within without also just now
       let's here's there's that's what's when's where's who's why's how's"

CLITICS="d ll m o re s t ve y ma ain"

cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

curl -sSfL "$URL" -o "$tmp/stopwords.zip"
unzip -o -q "$tmp/stopwords.zip" -d "$tmp"
tr -d '\r' < "$tmp/stopwords/english" | tr '[:upper:]' '[:lower:]' | grep -v '^$' | sort -u > "$tmp/nltk"

# Stems of negated contractions, plus the bare clitics. Mechanical, not a judgement call.
: > "$tmp/drop"
while read -r w; do
  grep -qxF "$w't" "$tmp/nltk" && echo "$w" >> "$tmp/drop"
done < "$tmp/nltk"
tr ' ' '\n' <<< "$CLITICS" | grep -v '^$' >> "$tmp/drop"
sort -u "$tmp/drop" -o "$tmp/drop"

comm -23 "$tmp/nltk" "$tmp/drop" > "$tmp/kept"
tr ' ' '\n' <<< "$ADDED" | sed 's/^[[:space:]]*//' | grep -v '^$' | sort -u > "$tmp/added"
sort -u "$tmp/kept" "$tmp/added" | fmt -w 78 > "$tmp/body"

# Without this guard a missing sentinel makes the sed range run to end-of-file, so the
# old body is copied and the new one appended. The result still parses — entries are
# whitespace-split into a set — so the duplicate is invisible and only the digest moves.
grep -q '^# Regenerate with' resources/stopwords.txt \
  || { echo "resources/stopwords.txt has no '# Regenerate with' header line" >&2; exit 1; }

{
  sed -n '1,/^# Regenerate with/p' resources/stopwords.txt
  cat "$tmp/body"
} > resources/stopwords.txt.new
mv resources/stopwords.txt.new resources/stopwords.txt

echo "DROPPED (unmatchable under our tokeniser): $(tr '\n' ' ' < "$tmp/drop")"
echo "ADDED   (modals and prepositions NLTK omits): $(tr '\n' ' ' < "$tmp/added")"
echo "wrote $(grep -v '^#' resources/stopwords.txt | wc -w) entries"
