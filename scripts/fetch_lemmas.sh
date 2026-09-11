#!/usr/bin/env bash
# Regenerate resources/lemmas.txt from WordNet's morphological exception lists.
#
# Committed output, reproducible input — the same arrangement as fetch_wordlist.sh and
# fetch_stopwords.sh. The list is embedded in the binary and its digest feeds
# PipelineVersion, so it must never be fetched at build or run time.
#
# WHAT THIS IS FOR. Resources::is_common_word lemmatises its lookup key, because the
# embedded wordlist holds one surface form per entry. `base_forms` in src/resources.rs
# is the suffix-detachment half of WordNet's morphy algorithm and reaches `running` ->
# `run` by rule. It cannot reach `ran` -> `run`, `mice` -> `mouse` or `analyses` ->
# `analysis` — irregular inflection is not a rule, it is a list. This is that list, and
# it is what makes the lookup lemmatisation rather than suffix stripping.
#
# Two mechanical filters are applied, because a lemma the wordlist could never hold is
# dead weight: entries are kept only where the inflected form and every lemma match
# ^[a-z]+$. That drops WordNet's underscore collocations (`allows_for` -> `allow_for`)
# and its hyphenated and non-ASCII entries. fetch_wordlist.sh selects wordlist entries
# with the same pattern, so the two files agree on what a word is by construction.
#
# Entries carry every lemma WordNet gives, across all four parts of speech, unioned:
# `better` is both `good` and `well`, and `axes` is both `axis` and `axe`. The lookup
# accepts any lemma present in the wordlist, so it needs all of them and needs no
# part-of-speech tagger to choose between them.
set -euo pipefail

URL="https://raw.githubusercontent.com/nltk/nltk_data/gh-pages/packages/corpora/wordnet.zip"

cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

curl -sSfL "$URL" -o "$tmp/wordnet.zip"
unzip -o -q "$tmp/wordnet.zip" -d "$tmp"

cat "$tmp"/wordnet/*.exc \
  | tr -d '\r' \
  | tr '[:upper:]' '[:lower:]' \
  | awk 'NF >= 2 {
        for (i = 1; i <= NF; i++) if ($i !~ /^[a-z]+$/) next
        for (i = 2; i <= NF; i++) if ($i != $1) pairs[$1 " " $i] = 1
    }
    END { for (p in pairs) print p }' \
  | sort \
  | awk '{ if ($1 == last) { line = line " " $2 } else { if (last != "") print line; last = $1; line = $0 } }
         END { if (last != "") print line }' \
  > "$tmp/body"

# Without this guard a missing sentinel makes the sed range run to end-of-file, so the
# old body is copied and the new one appended. The result still parses, so the duplicate
# is invisible and only the digest moves.
grep -q '^# Regenerate with' resources/lemmas.txt \
  || { echo "resources/lemmas.txt has no '# Regenerate with' header line" >&2; exit 1; }

{
  sed -n '1,/^# Regenerate with/p' resources/lemmas.txt
  cat "$tmp/body"
} > resources/lemmas.txt.new
mv resources/lemmas.txt.new resources/lemmas.txt

echo "wrote $(grep -vc '^#' resources/lemmas.txt) inflected forms"
