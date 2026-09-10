//! Instrument 5 — cross-format agreement.
//!
//! Settles: is canonicalisation silently format-dependent. The same source content in PDF,
//! docx and plain text should canonicalise to near-identical token sets. Needs no human
//! judgements, and catches the class of bug where one format's reader introduces spurious
//! breaks or drops content that the others keep.
//!
//! **Reported, not gated.** Some divergence between formats is legitimate — a spreadsheet
//! genuinely has different content from a memo — so a hard CI threshold would produce
//! false failures. The number is printed so a regression is visible in the log.

use std::collections::HashSet;

use keyword_extraction_pipeline::{canonicalise, Config, FormatHint};

fn fixture(name: &str) -> Vec<u8> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/");
    std::fs::read(format!("{dir}{name}")).unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

fn token_set(name: &str) -> HashSet<String> {
    let text = canonicalise(&fixture(name), FormatHint::Sniff, &Config::default())
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    text.split_whitespace()
        .map(|t| t.trim_matches(|c: char| c.is_ascii_punctuation() && c != '-').to_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    let union = a.union(b).count();
    if union == 0 {
        return 1.0;
    }
    a.intersection(b).count() as f64 / union as f64
}

#[test]
fn pdf_and_plain_text_of_the_same_content_agree() {
    // simple.pdf and simple.txt carry the same prose by construction.
    let (pdf, txt) = (token_set("simple.pdf"), token_set("simple.txt"));
    let score = jaccard(&pdf, &txt);

    println!("cross-format jaccard  pdf/txt = {score:.3}");
    let only_pdf: Vec<_> = pdf.difference(&txt).collect();
    let only_txt: Vec<_> = txt.difference(&pdf).collect();
    println!("  only in pdf: {only_pdf:?}");
    println!("  only in txt: {only_txt:?}");

    // A floor low enough never to fire on legitimate divergence, high enough to catch a
    // reader that has started dropping or mangling content wholesale.
    assert!(score > 0.5, "pdf/txt agreement collapsed to {score:.3}");
}

#[test]
fn planted_identifiers_survive_every_format() {
    // The one cross-format property that is not a matter of degree: an identifier present
    // in the source must be recoverable whatever the container.
    for name in ["simple.pdf", "simple.txt", "simple.docx", "simple.pptx", "simple.xlsx", "thread.eml"] {
        let tokens = token_set(name);
        assert!(
            tokens.contains("ds-2291"),
            "{name} lost the planted identifier; tokens: {tokens:?}"
        );
    }
}
