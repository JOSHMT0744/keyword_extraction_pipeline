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
{
  sed -n '1,/^# Regenerate with/p' resources/wordlist.txt
  grep -oP '^[a-z]+(?=\s)' "$tmp" | head -"$DEPTH"
} > resources/wordlist.txt.new
mv resources/wordlist.txt.new resources/wordlist.txt
rm -f "$tmp"
echo "wrote $(grep -vc '^#' resources/wordlist.txt) words"
