//! Batch extraction CLI: a directory of documents in, JSONL out.
//!
//! Exists so the whole corpus can be re-extracted offline without wiring the library into
//! a pipeline first, and so output can be eyeballed during tuning.

use std::{io::Write, path::PathBuf, process::ExitCode};

use keyword_extraction_pipeline::{extract, Config, FormatHint, Resources};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        eprintln!(
            "usage: kep <path>... [--pretty]\n\
             \n\
             Extracts keywords from each file (directories are walked) and writes one\n\
             JSON object per document to stdout. Status is always reported, including for\n\
             documents that yielded nothing."
        );
        return ExitCode::SUCCESS;
    }

    let pretty = args.iter().any(|a| a == "--pretty");
    let cfg = Config::default();
    let res = Resources::for_config(&cfg);

    let mut paths: Vec<PathBuf> = Vec::new();
    for arg in args.iter().filter(|a| !a.starts_with("--")) {
        collect(PathBuf::from(arg), &mut paths);
    }
    paths.sort();

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut failures = 0usize;

    for path in paths {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                failures += 1;
                continue;
            }
        };
        let result = extract(&bytes, FormatHint::Sniff, &cfg, &res);
        let mut value = serde_json::to_value(&result).expect("result is serialisable");
        value["path"] = serde_json::Value::String(path.display().to_string());

        let line = if pretty {
            serde_json::to_string_pretty(&value)
        } else {
            serde_json::to_string(&value)
        }
        .expect("serialisable");
        let _ = writeln!(out, "{line}");
    }

    if failures > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Walk directories depth-first. Unreadable entries are skipped rather than aborting the
/// run: a corpus pass should not be lost to one bad file.
fn collect(path: PathBuf, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let Ok(entries) = std::fs::read_dir(&path) else { return };
        for entry in entries.flatten() {
            collect(entry.path(), out);
        }
    } else if path.is_file() {
        out.push(path);
    }
}
