//! Instrument 2, CLI arm — determinism and reporting.
//!
//! Settles: does the binary report every document, in a stable form, whatever the shell
//! does to its stdout. The library's own guarantees are tested elsewhere; what is tested
//! here is the part a person or a cron job actually touches.

use std::{path::PathBuf, process::Command};

fn kep() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kep"))
}

fn fixtures() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures"))
}

fn run(args: &[&str]) -> (String, String, i32) {
    let out = kep().args(args).output().expect("kep runs");
    (
        String::from_utf8(out.stdout).expect("stdout is utf-8"),
        String::from_utf8(out.stderr).expect("stderr is utf-8"),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn every_jsonl_line_is_one_valid_json_document() {
    let (stdout, _, code) =
        run(&["extract", "--format", "jsonl", fixtures().to_str().unwrap()]);
    assert_eq!(code, 0);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 8, "one line per fixture, got {}", lines.len());
    for line in lines {
        let v: serde_json::Value = serde_json::from_str(line).expect("line parses");
        assert!(v["path"].is_string(), "no path in {line}");
        // Grouping is the contract: three named lists, never one flat array.
        for kind in ["identifier", "technical", "topical"] {
            assert!(v["keywords"][kind].is_array(), "missing {kind} group in {line}");
        }
    }
}

#[test]
fn digests_are_hex_rather_than_arrays_of_numbers() {
    let (stdout, _, _) = run(&[
        "extract",
        "--format",
        "jsonl",
        fixtures().join("simple.pdf").to_str().unwrap(),
    ]);
    let v: serde_json::Value = serde_json::from_str(stdout.lines().next().unwrap()).unwrap();
    for field in ["hash_exact", "hash_canonical", "pipeline_version"] {
        let s = v[field].as_str().unwrap_or_else(|| panic!("{field} is not a string"));
        assert_eq!(s.len(), 64, "{field} is not a 32-byte digest: {s}");
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()), "{field} is not hex: {s}");
    }
}

#[test]
fn repeated_runs_are_byte_identical() {
    let dir = fixtures();
    let args = ["extract", "--format", "jsonl", dir.to_str().unwrap()];
    assert_eq!(run(&args).0, run(&args).0, "output drifted between runs");
}

#[test]
fn an_explicit_format_does_not_depend_on_where_stdout_goes() {
    // Output is captured here, so stdout is never a terminal; the point is that asking
    // for a format gets that format rather than the redirect-time default.
    let path = fixtures().join("simple.pdf");
    let (table, _, _) = run(&["extract", "--format", "table", path.to_str().unwrap()]);
    assert!(table.contains("Identifier"), "not the table format: {table}");
    assert!(serde_json::from_str::<serde_json::Value>(&table).is_err());
}

#[test]
fn a_document_that_yielded_nothing_is_still_reported() {
    // The silent-omission failure, at the CLI level: a scanned PDF must appear in the
    // output carrying its status, not be quietly dropped for having no keywords.
    let (stdout, _, code) = run(&[
        "extract",
        "--format",
        "jsonl",
        fixtures().join("scanned.pdf").to_str().unwrap(),
    ]);
    let v: serde_json::Value = serde_json::from_str(stdout.lines().next().unwrap()).unwrap();
    assert_eq!(v["status"], "NoTextLayer");
    assert_eq!(code, 0, "an expected status is not a failure");
}

#[test]
fn strict_promotes_a_non_ok_status_to_a_failing_exit_code() {
    let path = fixtures().join("scanned.pdf");
    let (_, _, lenient) = run(&["extract", "--format", "jsonl", path.to_str().unwrap()]);
    let (_, _, strict) =
        run(&["extract", "--strict", "--format", "jsonl", path.to_str().unwrap()]);
    assert_eq!(lenient, 0);
    assert_eq!(strict, 2);
}

#[test]
fn a_missing_path_fails_loudly_rather_than_producing_an_empty_run() {
    let (stdout, stderr, code) = run(&["extract", "no/such/place"]);
    assert_eq!(code, 1);
    assert!(stdout.is_empty(), "produced output for a path that does not exist");
    assert!(stderr.contains("no such file"), "unhelpful error: {stderr}");
}

#[test]
fn the_run_header_names_the_pipeline_version() {
    // A stamp change invalidates every stored keyword set. It has to be visible without
    // anyone asking for it.
    let (_, stderr, _) = run(&[
        "extract",
        "--format",
        "jsonl",
        fixtures().join("simple.pdf").to_str().unwrap(),
    ]);
    assert!(stderr.contains("pipeline_version"), "no version in header: {stderr}");
    assert!(stderr.contains("1 document: 1 ok"), "no status trailer: {stderr}");
}

#[test]
fn quiet_suppresses_commentary_but_never_data() {
    let path = fixtures().join("simple.pdf");
    let (stdout, stderr, _) =
        run(&["extract", "--quiet", "--format", "jsonl", path.to_str().unwrap()]);
    assert!(stderr.is_empty(), "--quiet still wrote to stderr: {stderr}");
    assert_eq!(stdout.lines().count(), 1);
}

#[test]
fn filters_apply_to_the_view_and_leave_rank_referring_to_the_full_set() {
    let path = fixtures().join("simple.pdf");
    let (stdout, _, _) = run(&[
        "extract",
        "--format",
        "jsonl",
        "--kind",
        "identifier",
        "--top",
        "2",
        path.to_str().unwrap(),
    ]);
    let v: serde_json::Value = serde_json::from_str(stdout.lines().next().unwrap()).unwrap();
    assert_eq!(v["keywords"]["identifier"].as_array().unwrap().len(), 2);
    assert!(v["keywords"]["technical"].as_array().unwrap().is_empty());
    assert_eq!(v["keywords"]["identifier"][1]["rank"], 1);
}

#[test]
fn a_partial_config_file_is_filled_in_from_defaults() {
    // Hand-editing a config must not mean restating all fifteen fields.
    let dir = std::env::temp_dir().join("kep-cli-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("partial.json");
    std::fs::write(&path, br#"{"wordlist_size": 40000}"#).unwrap();

    let (_, stderr, code) = run(&[
        "extract",
        "--config-file",
        path.to_str().unwrap(),
        "--format",
        "jsonl",
        fixtures().join("simple.pdf").to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "partial config rejected: {stderr}");

    // The same change made by flag must land on the same stamp, or the two paths are
    // not resolving to the same config.
    let (_, by_flag, _) = run(&[
        "extract",
        "--wordlist-size",
        "40000",
        "--format",
        "jsonl",
        fixtures().join("simple.pdf").to_str().unwrap(),
    ]);
    let stamp = |s: &str| s.split("pipeline_version ").nth(1).unwrap().trim().to_string();
    assert_eq!(stamp(&stderr), stamp(&by_flag));
}

#[test]
fn explain_accounts_for_a_score_and_shows_what_a_lower_threshold_would_admit() {
    let (stdout, _, code) =
        run(&["explain", fixtures().join("simple.pdf").to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(stdout.contains("absent_from_wordlist"), "no feature breakdown: {stdout}");
    assert!(stdout.contains("BELOW THRESHOLD"), "no near-misses: {stdout}");
    // `SuRe` is admitted only as part of `MabSelect SuRe`; alone it is below the
    // technical threshold, which makes it the near-miss this document should surface.
    assert!(stdout.contains("SuRe"), "expected near-miss missing: {stdout}");
}

#[test]
fn an_empty_kind_states_why_it_is_empty() {
    // A blank where keywords should be is the same silent-omission failure
    // `DocumentStatus` exists to prevent, one level down.
    let (stdout, _, _) = run(&[
        "extract",
        "--format",
        "table",
        fixtures().join("simple.xlsx").to_str().unwrap(),
    ]);
    assert!(stdout.contains("Technical"), "no technical row at all: {stdout}");
    assert!(
        stdout.contains("threshold of 0.38"),
        "empty technical gave no reason: {stdout}"
    );
}

#[test]
fn version_reports_the_provenance_of_the_stamp() {
    let (stdout, _, code) = run(&["version"]);
    assert_eq!(code, 0);
    for expected in ["logic_revision", "wordlist", "pipeline_version"] {
        assert!(stdout.contains(expected), "missing {expected}: {stdout}");
    }
}

#[test]
fn config_emits_json_the_extract_command_accepts_back() {
    let out = kep().arg("config").output().expect("kep runs");
    let json = String::from_utf8(out.stdout).unwrap();
    let dir = std::env::temp_dir().join("kep-cli-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("roundtrip.json");
    std::fs::write(&path, &json).unwrap();

    let (_, _, code) = run(&[
        "extract",
        "--config-file",
        path.to_str().unwrap(),
        "--format",
        "jsonl",
        fixtures().join("simple.pdf").to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "kep config produced a file kep extract rejects");
}
