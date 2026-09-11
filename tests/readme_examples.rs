//! Every code example in README.md, compiled and run.
//!
//! A README example that does not compile is worse than no example: it costs a newcomer
//! the time to discover that the problem is ours, not theirs.

use keyword_extraction_pipeline::{
    canonicalise, extract, Config, DocumentStatus, FormatHint, Kind, Origin, Resources,
};

fn sample() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/simple.pdf")).unwrap()
}

#[test]
fn quick_start() {
    let bytes = sample();

    let cfg = Config::default();
    let res = Resources::for_config(&cfg);
    let result = extract(&bytes, FormatHint::Sniff, &cfg, &res);

    match result.status {
        DocumentStatus::Ok => {
            for kw in &result.keywords {
                println!("{:?}  {:<20} {:.3}", kw.kind, kw.original_keyword, kw.score);
            }
        }
        other => println!("no keywords, and here is why: {other:?}"),
    }
    assert!(!result.keywords.is_empty());
}

#[test]
fn top_five_identifiers() {
    let bytes = sample();
    let cfg = Config::default();
    let result = extract(&bytes, FormatHint::Sniff, &cfg, &Resources::for_config(&cfg));

    let ids: Vec<&str> = result
        .keywords
        .iter()
        .filter(|k| k.kind == Kind::Identifier)
        .take(5)
        .map(|k| k.original_keyword.as_str())
        .collect();

    assert!(ids.contains(&"DS-2291"), "{ids:?}");
}

#[test]
fn resolving_offsets() {
    let bytes = sample();
    let cfg = Config::default();
    let result = extract(&bytes, FormatHint::Sniff, &cfg, &Resources::for_config(&cfg));

    let text = canonicalise(&bytes, FormatHint::Sniff, &cfg).unwrap();
    let kw = &result.keywords[0];
    let span = kw.offsets[0].clone();
    assert_eq!(text[span].to_lowercase(), kw.normalised);
}

#[test]
fn deduplicating_across_stages() {
    let bytes = sample();
    let cfg = Config::default();
    let mut result = extract(&bytes, FormatHint::Sniff, &cfg, &Resources::for_config(&cfg));

    // Keep definitional evidence over an orthographic guess for the same term.
    result.keywords.sort_by_key(|k| (k.origin != Origin::Definition) as u8);
    let mut seen = std::collections::HashSet::new();
    result.keywords.retain(|k| seen.insert((k.normalised.clone(), k.kind)));

    let mut pairs: Vec<_> = result.keywords.iter().map(|k| (&k.normalised, k.kind)).collect();
    let before = pairs.len();
    pairs.dedup();
    assert_eq!(pairs.len(), before, "duplicates survived");
}

#[test]
fn tuning_config() {
    let bytes = sample();
    let cfg = Config {
        wordlist_size: 40_000,
        retain_features: true,
        ..Config::default()
    };
    // Resources must be rebuilt for the config, or the active wordlist cutoff and the
    // version stamp disagree.
    let res = Resources::for_config(&cfg);
    let result = extract(&bytes, FormatHint::Sniff, &cfg, &res);

    let kw = &result.keywords[0];
    let features = kw.features.expect("retain_features was on");
    assert!(features.absent_from_wordlist >= 0.0);
}

#[test]
fn why_was_topical_empty() {
    let bytes = sample();
    let cfg = Config::default();
    let result = extract(&bytes, FormatHint::Sniff, &cfg, &Resources::for_config(&cfg));

    if !result.keywords.iter().any(|k| k.kind == Kind::Topical) {
        let verdict = result.prose.expect("the document reached the gate");
        println!("no topical keywords: {}", verdict.summary());
        assert!(!verdict.is_prose);
    }
}

#[test]
fn resources_must_be_built_from_the_same_config() {
    // The README's gotcha, as code: `Resources::default()` matches `Config::default()`
    // and nothing else, so pairing it with a tuned config silently uses the wrong
    // wordlist cutoff under a stamp that claims otherwise.
    let cfg = Config { wordlist_size: 40_000, ..Config::default() };

    let right = Resources::for_config(&cfg);
    let wrong = Resources::default();

    assert_eq!(right.wordlist_extent().0, 40_000);
    assert_eq!(wrong.wordlist_extent().0, Config::default().wordlist_size);
    assert_ne!(right.wordlist_extent().0, wrong.wordlist_extent().0);
}
