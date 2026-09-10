//! Emits the resolved versions of extraction-relevant dependencies as a compile-time
//! constant, so `PipelineVersion` changes automatically when a parser is upgraded.
//!
//! Only the crates listed in `TRACKED` are included. Hashing all of `Cargo.lock` would
//! churn the fingerprint on unrelated dev-dependency bumps and invalidate every stored
//! keyword set for no reason.

use std::{env, fs, path::PathBuf};

/// Crates whose behaviour can change extracted text or candidate selection.
const TRACKED: &[&str] = &[
    "pdf_oxide",
    "calamine",
    "quick-xml",
    "zip",
    "mail-parser",
    "lingua",
    "unicode-normalization",
    "unicode-segmentation",
];

fn main() {
    let lock = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("Cargo.lock");
    println!("cargo:rerun-if-changed=Cargo.lock");
    println!("cargo:rerun-if-changed=build.rs");

    let text = fs::read_to_string(&lock).unwrap_or_default();
    let mut found: Vec<String> = Vec::new();
    let mut name: Option<String> = None;

    // Cargo.lock is TOML, but a three-line scan avoids a build-dependency on a TOML parser.
    for line in text.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("name = ") {
            name = Some(v.trim_matches('"').to_string());
        } else if let Some(v) = line.strip_prefix("version = ") {
            if let Some(n) = name.take() {
                if TRACKED.contains(&n.as_str()) {
                    found.push(format!("{n}={}", v.trim_matches('"')));
                }
            }
        }
    }
    found.sort();

    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("parser_versions.rs");
    fs::write(
        &out,
        format!(
            "/// Resolved versions of extraction-relevant dependencies, sorted.\npub const PARSER_VERSIONS: &str = {:?};\n",
            found.join(",")
        ),
    )
    .unwrap();
}
