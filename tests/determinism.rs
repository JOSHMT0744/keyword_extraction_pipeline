//! Instrument 2 — determinism and regression.
//!
//! Settles: is the README's guarantee real. Needs no human judgements at all, and runs on
//! every CI run forever. The guarantee under test is that a document's keyword set is a
//! pure function of `(bytes, config, resources)`, summarised by `PipelineVersion`.

use keyword_extraction_pipeline::{
    extract, Config, DocumentStatus, FormatHint, PipelineVersion, Resources,
};

const FIXTURES: &[&str] = &[
    "simple.pdf", "two_column.pdf", "scanned.pdf", "simple.docx",
    "simple.xlsx", "simple.pptx", "thread.eml", "simple.txt",
];

fn fixture(name: &str) -> Vec<u8> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/");
    std::fs::read(format!("{dir}{name}"))
        .unwrap_or_else(|e| panic!("fixture {name}: {e}; run scripts/make_fixtures.py"))
}

#[test]
fn repeated_extraction_is_identical() {
    let (cfg, res) = (Config::default(), Resources::default());
    for name in FIXTURES {
        let bytes = fixture(name);
        let a = extract(&bytes, FormatHint::Sniff, &cfg, &res);
        let b = extract(&bytes, FormatHint::Sniff, &cfg, &res);

        assert_eq!(a.hash_canonical, b.hash_canonical, "{name}: canonical text differed");
        assert_eq!(a.own_content_length, b.own_content_length, "{name}");
        assert_eq!(a.keywords, b.keywords, "{name}: keyword set differed");
        assert_eq!(a.language, b.language, "{name}");
    }
}

#[test]
fn canonicalise_reproduces_the_text_offsets_refer_to() {
    // `extract` deliberately does not return canonical text. If the separately-exposed
    // `canonicalise` did not reproduce it byte-for-byte, every offset would be unusable.
    let (cfg, res) = (Config::default(), Resources::default());
    for name in FIXTURES {
        let bytes = fixture(name);
        let result = extract(&bytes, FormatHint::Sniff, &cfg, &res);
        let Some(expected) = result.hash_canonical else { continue };

        let text = keyword_extraction_pipeline::canonicalise(&bytes, FormatHint::Sniff, &cfg)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            *blake3::hash(text.as_bytes()).as_bytes(),
            expected,
            "{name}: regenerated canonical text does not match the extracted one"
        );
    }
}

#[test]
fn a_config_change_changes_the_version_stamp() {
    // The guard that makes the snapshot suite meaningful: behaviour must not be able to
    // change while the version stays the same.
    let res = Resources::default();
    let base = PipelineVersion::compute(&Config::default(), &res);

    let variants = [
        Config { strip_quoted_blocks: false, ..Config::default() },
        Config { min_content_length: 65, ..Config::default() },
        Config { enable_topical: false, ..Config::default() },
        Config { yake_ngram_max: 4, ..Config::default() },
    ];
    for cfg in variants {
        assert_ne!(base, PipelineVersion::compute(&cfg, &res));
    }
}

#[test]
fn every_fixture_reports_a_status_and_never_panics() {
    let (cfg, res) = (Config::default(), Resources::default());
    for name in FIXTURES {
        let out = extract(&fixture(name), FormatHint::Sniff, &cfg, &res);
        match name {
            // The load-bearing distinction: an image-only PDF is flagged, not reported as
            // an ordinary document that happened to contain nothing.
            &"scanned.pdf" => assert_eq!(out.status, DocumentStatus::NoTextLayer),
            _ => assert_eq!(out.status, DocumentStatus::Ok, "{name}"),
        }
    }
}

#[test]
fn an_identifier_dense_short_document_is_not_rejected_as_too_short() {
    // simple.xlsx is ~60 characters of almost pure identifiers. It is short, and it is
    // the single most valuable shape of document for the identifier lane. `TooShort` must
    // mean "empty", not "brief" — gating on prose-sized length would silently discard the
    // instrument-output case entirely.
    let out = extract(&fixture("simple.xlsx"), FormatHint::Sniff, &Config::default(), &Resources::default());
    assert_eq!(out.status, DocumentStatus::Ok, "len={}", out.own_content_length);
    assert!(out.own_content_length < 120, "fixture is no longer the short case it tests");
}

#[test]
fn truncated_and_malformed_input_yields_a_status_not_a_panic() {
    let (cfg, res) = (Config::default(), Resources::default());
    let pdf = fixture("simple.pdf");

    for bytes in [
        &pdf[..pdf.len() / 2],
        &pdf[..16],
        b"%PDF-1.7 but nothing else at all".as_slice(),
        b"".as_slice(),
        &[0xff, 0xfe, 0x00, 0x01],
    ] {
        let out = extract(bytes, FormatHint::Sniff, &cfg, &res);
        assert!(
            !out.status.is_ok() || out.keywords.is_empty(),
            "malformed input reported as a healthy document"
        );
    }
}
