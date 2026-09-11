#!/usr/bin/env bash
# Regenerate resources/wordlist.txt from the upstream frequency list.
#
# Committed output, reproducible input: the list is embedded in the binary and its digest
# feeds PipelineVersion, so it must never be fetched at build or run time.
set -euo pipefail
URL="https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018/en/en_full.txt"
DEPTH=80000   # embedded depth; Config::wordlist_size selects a cutoff at or below this
cd "$(dirname "$0")/.."
tmp=$(mktemp)
curl -sSfL "$URL" -o "$tmp"
# Without this guard a missing sentinel makes the sed range run to end-of-file, so the
# old body is copied and the new one appended. The result still parses, so the duplicate
# is invisible and only the digest moves.
grep -q '^# Regenerate with' resources/wordlist.txt \
  || { echo "resources/wordlist.txt has no '# Regenerate with' header line" >&2; exit 1; }

{
  sed -n '1,/^# Regenerate with/p' resources/wordlist.txt
  # `head` closes the pipe, which SIGPIPEs grep; under `set -o pipefail` that would
  # make a successful run exit 141. Bound the input instead of the output.
  grep -oP '^[a-z]+(?=\s)' "$tmp" | awk -v n="$DEPTH" 'NR <= n'
} > resources/wordlist.txt.new
mv resources/wordlist.txt.new resources/wordlist.txt
rm -f "$tmp"
echo "wrote $(grep -vc '^#' resources/wordlist.txt) words"
