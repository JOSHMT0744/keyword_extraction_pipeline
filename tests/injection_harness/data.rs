//! Loads `tests/data/injection/{sources,distractors}/*.tsv`.
//!
//! Every file is a plain TSV: `surface\tregister\tscheme`, with `#`-prefixed header lines
//! recording where the data came from. See `scripts/fetch_injection_tiers.sh` for the
//! convention, which mirrors `resources/wordlist.txt`'s.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Entry {
    pub surface: String,
    pub register: String,
    pub scheme: String,
    /// The file this entry came from, relative to `tests/data/injection/` — kept so a
    /// gate can point back at the exact source on failure.
    pub file: String,
}

fn data_root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/injection"))
}

/// Parse one TSV file. Panics on a malformed row rather than skipping it silently — a
/// row this harness can't parse is a row nobody is actually testing against.
fn parse_tsv(path: &Path) -> Vec<Entry> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}; run scripts/fetch_injection_tiers.sh", path.display()));
    let file = path
        .strip_prefix(data_root())
        .unwrap_or(path)
        .display()
        .to_string();

    let mut out = Vec::new();
    for (lineno, line) in text.lines().enumerate() {
        if line.trim_start().starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            fields.len(),
            3,
            "{}:{}: expected 3 tab-separated fields, got {}: {line:?}",
            path.display(),
            lineno + 1,
            fields.len()
        );
        out.push(Entry {
            surface: fields[0].to_string(),
            register: fields[1].to_string(),
            scheme: fields[2].to_string(),
            file: file.clone(),
        });
    }
    out
}

fn list_tsv(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "tsv"))
        .collect();
    paths.sort();
    paths
}

/// Every planted-identifier source file, each entry tagged with the file it came from.
///
/// Takes a fixed-size PREFIX of each file's entries — never the whole file, and never a
/// hand-picked subset. `scripts/fetch_injection_tiers.sh` already writes each source in
/// the order its API returned it, unfiltered; taking a prefix here is the second half of
/// that control; nobody downstream of the fetch can select which entries survive.
pub fn sources(prefix_per_file: usize) -> Vec<Entry> {
    let mut out = Vec::new();
    for path in list_tsv(&data_root().join("sources")) {
        let entries = parse_tsv(&path);
        out.extend(entries.into_iter().take(prefix_per_file));
    }
    out
}

/// Every distractor file, same prefix discipline as `sources`.
pub fn distractors(prefix_per_file: usize) -> Vec<Entry> {
    let mut out = Vec::new();
    for path in list_tsv(&data_root().join("distractors")) {
        let entries = parse_tsv(&path);
        out.extend(entries.into_iter().take(prefix_per_file));
    }
    out
}

/// Every source and distractor TSV file, header lines included — for the provenance gate.
pub fn all_tsv_files() -> Vec<PathBuf> {
    let mut paths = list_tsv(&data_root().join("sources"));
    paths.extend(list_tsv(&data_root().join("distractors")));
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_source_and_distractor_file_names_the_fetch_script() {
        // Mirrors `every_embedded_resource_names_the_script_that_regenerates_it` in
        // src/resources.rs. Data with no reproduction path is an assertion, not evidence.
        let files = all_tsv_files();
        assert!(!files.is_empty(), "no tsv files found under tests/data/injection/");
        for path in &files {
            let text = std::fs::read_to_string(path).unwrap();
            assert!(
                text.contains("# Regenerate with scripts/fetch_injection_tiers.sh"),
                "{}: no regeneration sentinel",
                path.display()
            );
        }
        assert!(
            PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/fetch_injection_tiers.sh"))
                .exists(),
            "scripts/fetch_injection_tiers.sh is named but does not exist"
        );
    }

    #[test]
    fn every_source_entry_declares_where_it_came_from() {
        for path in list_tsv(&data_root().join("sources")) {
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(
                text.contains("# URL:") && text.contains("# Licence:"),
                "{}: missing URL or Licence header — an entry with no declared source is \
                 exactly what tier_of() exists to prevent: unaudited difficulty",
                path.display()
            );
        }
    }

    #[test]
    fn generated_distractor_files_say_so_plainly() {
        // The distinction the module doc insists on: a file this script generated from a
        // format spec must never look, to a reader, like it was mined from a real source.
        // A file with no real URL to point at (its "# URL:" field is "n/a") is exactly the
        // case that must be labelled GENERATED — that is the tell a reader has no other
        // way to notice.
        for path in list_tsv(&data_root().join("distractors")) {
            let text = std::fs::read_to_string(&path).unwrap();
            let has_real_url = text.lines().any(|l| {
                l.starts_with("# URL:") && !l.trim_start_matches("# URL:").trim().starts_with("n/a")
            });
            if !has_real_url {
                assert!(
                    text.contains("GENERATED"),
                    "{}: no real source URL, but header does not say GENERATED — a reader                      would mistake this for mined data",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn no_surface_appears_in_two_different_files() {
        let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for path in all_tsv_files() {
            for e in parse_tsv(&path) {
                if let Some(other) = seen.get(&e.surface) {
                    if *other != e.file {
                        panic!(
                            "{:?} appears in both {} and {}",
                            e.surface, other, e.file
                        );
                    }
                }
                seen.insert(e.surface, e.file);
            }
        }
    }
}
