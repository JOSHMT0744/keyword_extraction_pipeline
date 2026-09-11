#!/usr/bin/env bash
# Regenerate tests/data/injection/{sources,distractors}/*.tsv for instrument 3
# (the identifier-injection harness, tests/injection.rs).
#
# Two kinds of file, and this script is honest about which is which:
#
#   sources/*.tsv     — real identifier examples pulled from a public API, unfiltered,
#                        in the order the source returned them. The harness later takes
#                        a PREFIX of each file, so nobody hand-picks which entries survive.
#   distractors/*.tsv — identifier-SHAPED text that is not an identifier. Some of these
#                        (semver strings, git hashes) are mined the same way as sources.
#                        Others (dates, phone numbers, invented ISBNs) have no single public
#                        list to mine — for those this script generates instances from a
#                        documented format spec and a fixed seed, and each such file's
#                        header says so explicitly. Never presented as mined when it isn't.
#
# This script is a developer tool, run deliberately; its output is committed and reviewed,
# same arrangement as scripts/fetch_wordlist.sh. Difficulty TIER is not decided here — it
# is computed by tests/injection_harness/tiers.rs from a fixed rule, so nothing about
# what this script chooses to include can make a plant look easier than it is.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=tests/data/injection
mkdir -p "$OUT/sources" "$OUT/distractors"

header() {
  # $1 = target file, $2 = source name, $3 = url, $4 = licence, $5 = register, $6 = scheme,
  # $7 = extra note (mined vs generated)
  cat > "$1" <<EOF
# Injection source: $2
# URL: $3
# Licence: $4
# Mined: $(date -u +%Y-%m-%d). $7
# Register: $5
# Scheme: $6
#
# Columns: surface	register	scheme
# The harness (tests/injection_harness/tiers.rs) computes difficulty tier from the
# surface itself, at fixed wordlist depth — it is never read from this file.
#
# Regenerate with scripts/fetch_injection_tiers.sh
EOF
}

echo "== sources: real identifier examples =="

echo "-- Apache JIRA issue keys (it/ticket register) --"
header "$OUT/sources/jira_issue_keys.tsv" \
  "Apache Software Foundation public JIRA (KAFKA project)" \
  "https://issues.apache.org/jira/rest/api/2/search" \
  "Public project metadata, Apache Software Foundation" \
  "it" "ticket-key" \
  "Entries are the first N issues returned by the REST API in its own (recency) order, unfiltered."
curl -sSf --max-time 15 \
  "https://issues.apache.org/jira/rest/api/2/search?jql=project=KAFKA%20ORDER%20BY%20created%20DESC&maxResults=150&fields=key" \
  | python3 -c "
import json, sys
d = json.load(sys.stdin)
for i in d['issues']:
    print(f\"{i['key']}\tit\tticket-key\")
" >> "$OUT/sources/jira_issue_keys.tsv"
echo "  wrote $(grep -vc '^#' "$OUT/sources/jira_issue_keys.tsv") entries"

echo "-- NVD CVE identifiers (it/security register) --"
header "$OUT/sources/nvd_cve_identifiers.tsv" \
  "National Vulnerability Database" \
  "https://services.nvd.nist.gov/rest/json/cves/2.0" \
  "US Government public data, no restriction" \
  "it" "cve-id" \
  "Entries are the first page returned by the public NVD API, unfiltered."
curl -sSf --max-time 15 "https://services.nvd.nist.gov/rest/json/cves/2.0?resultsPerPage=150" \
  | python3 -c "
import json, sys
d = json.load(sys.stdin)
for v in d.get('vulnerabilities', []):
    print(f\"{v['cve']['id']}\tit\tcve-id\")
" >> "$OUT/sources/nvd_cve_identifiers.tsv"
echo "  wrote $(grep -vc '^#' "$OUT/sources/nvd_cve_identifiers.tsv") entries"

echo "-- RCSB PDB entry ids (lab register) --"
header "$OUT/sources/rcsb_pdb_entry_ids.tsv" \
  "RCSB Protein Data Bank full-text search (query: chromatography)" \
  "https://search.rcsb.org/rcsbsearch/v2/query" \
  "Public domain, RCSB PDB" \
  "lab" "pdb-entry-id" \
  "Entries are the first page returned by RCSB's search API, unfiltered."
curl -sSf --max-time 15 -X POST "https://search.rcsb.org/rcsbsearch/v2/query" \
  -H "Content-Type: application/json" \
  -d '{"query":{"type":"terminal","service":"full_text","parameters":{"value":"chromatography"}},"return_type":"entry","request_options":{"paginate":{"start":0,"rows":150}}}' \
  | python3 -c "
import json, sys
d = json.load(sys.stdin)
for r in d.get('result_set', []):
    print(f\"{r['identifier']}\tlab\tpdb-entry-id\")
" >> "$OUT/sources/rcsb_pdb_entry_ids.tsv"
echo "  wrote $(grep -vc '^#' "$OUT/sources/rcsb_pdb_entry_ids.tsv") entries"

echo "== distractors: identifier-shaped non-identifiers =="

echo "-- semver strings (mined from crates.io index: serde) --"
header "$OUT/distractors/semver_strings.tsv" \
  "crates.io sparse index, the 'serde' crate's published versions" \
  "https://index.crates.io/se/rd/serde" \
  "Public package metadata, crates.io" \
  "it" "semver" \
  "Entries are every version crates.io has ever indexed for this one crate, unfiltered, in index order."
curl -sSf --max-time 15 "https://index.crates.io/se/rd/serde" \
  | python3 -c "
import json, sys
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    d = json.loads(line)
    print(f\"v{d['vers']}\tit\tsemver\")
" >> "$OUT/distractors/semver_strings.tsv"
echo "  wrote $(grep -vc '^#' "$OUT/distractors/semver_strings.tsv") entries"

echo "-- git short hashes (mined from this repo's own log) --"
header "$OUT/distractors/git_short_hashes.tsv" \
  "This repository's own commit log" \
  "(local: git log --format=%h)" \
  "n/a — repository metadata, not third-party content" \
  "it" "git-short-hash" \
  "Mined from this repository's own commit log. Entries are the most recent commits at generation time, unfiltered."
git log --format="%h	it	git-short-hash" >> "$OUT/distractors/git_short_hashes.tsv"
echo "  wrote $(grep -vc '^#' "$OUT/distractors/git_short_hashes.tsv") entries"

echo "-- ISO 8601 dates (GENERATED, not mined — see header) --"
header "$OUT/distractors/iso8601_dates.tsv" \
  "Generated from the ISO 8601 date format, not mined from any list" \
  "https://en.wikipedia.org/wiki/ISO_8601" \
  "n/a — generated data, format spec only" \
  "any" "iso8601-date" \
  "GENERATED: there is no public 'list of dates' to mine. Every YYYY-MM-DD in [2015-01-01, 2025-12-31] step 37 days, deterministic, not random."
python3 -c "
import datetime
d = datetime.date(2015, 1, 1)
end = datetime.date(2025, 12, 31)
while d <= end:
    print(f'{d.isoformat()}\tany\tiso8601-date')
    d += datetime.timedelta(days=37)
" >> "$OUT/distractors/iso8601_dates.tsv"
echo "  wrote $(grep -vc '^#' "$OUT/distractors/iso8601_dates.tsv") entries"

echo "-- URLs (GENERATED, not mined — see header) --"
header "$OUT/distractors/urls.tsv" \
  "Generated from a fixed set of realistic path templates, not mined from any list" \
  "n/a" \
  "n/a — generated data" \
  "any" "url" \
  "GENERATED: realistic document-reference URLs, deterministic template x counter, not random."
python3 -c "
templates = [
    'https://example.com/docs/ref-{n}',
    'https://internal.example.org/tickets/{n}',
    'https://kb.example.net/articles/{n}',
    'https://portal.example.com/orders/{n}',
]
n = 1000
for t in templates:
    for i in range(20):
        print(f'{t.format(n=n+i)}\tany\turl')
    n += 100
" >> "$OUT/distractors/urls.tsv"
echo "  wrote $(grep -vc '^#' "$OUT/distractors/urls.tsv") entries"

echo "-- phone numbers (GENERATED, not mined — see header) --"
header "$OUT/distractors/phone_numbers.tsv" \
  "Generated from the E.164 international phone number format, not mined from any list" \
  "n/a" \
  "n/a — generated data, format spec only" \
  "any" "phone-number" \
  "GENERATED: deterministic template x counter over a fixed set of country codes, not random."
python3 -c "
codes = ['+1', '+44', '+49', '+33', '+61']
n = 2000000
for c in codes:
    for i in range(20):
        print(f'{c} {n+i*7}	any	phone-number')
    n += 1000000
" >> "$OUT/distractors/phone_numbers.tsv"
echo "  wrote $(grep -vc '^#' "$OUT/distractors/phone_numbers.tsv") entries"

echo
echo "done. Review the diff before committing — this is developer-run, output-committed data."
