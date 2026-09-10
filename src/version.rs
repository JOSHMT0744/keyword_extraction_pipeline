//! The pipeline version stamp.
//!
//! A *computed fingerprint*, never a hand-maintained string. A forgotten manual bump
//! would let two different keyword sets share a version, which is precisely the silent
//! failure the whole design exists to avoid.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{config::Config, resources::Resources};

include!(concat!(env!("OUT_DIR"), "/parser_versions.rs"));

/// Bumped by hand only when extraction *logic* changes in a way not captured by config
/// or dependency versions — a new lane, an altered canonicalisation step.
///
/// History: 1 initial; 2 stopword list read as words rather than lines.
const LOGIC_REVISION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PipelineVersion([u8; 32]);

impl PipelineVersion {
    /// Derived from crate version, logic revision, resolved parser versions, the
    /// effective config, and the digests of the active resource lists.
    pub fn compute(cfg: &Config, res: &Resources) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(b"kep-v1\0");
        h.update(env!("CARGO_PKG_VERSION").as_bytes());
        h.update(&[0]);
        h.update(&LOGIC_REVISION.to_le_bytes());
        h.update(PARSER_VERSIONS.as_bytes());
        h.update(&[0]);
        cfg.feed(&mut h);
        h.update(&res.wordlist_digest);
        h.update(&res.stopwords_digest);
        Self(*h.finalize().as_bytes())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Short hex form for logs and filenames.
    pub fn short(&self) -> String {
        self.0[..6].iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl fmt::Display for PipelineVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in &self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_stable_across_calls() {
        let (cfg, res) = (Config::default(), Resources::default());
        assert_eq!(
            PipelineVersion::compute(&cfg, &res),
            PipelineVersion::compute(&cfg, &res)
        );
    }

    #[test]
    fn changes_with_config() {
        let res = Resources::default();
        let base = PipelineVersion::compute(&Config::default(), &res);

        let mut thresholds = crate::config::Thresholds::default();
        thresholds.identifier += 0.01;
        let cfg = Config { thresholds, ..Config::default() };
        assert_ne!(base, PipelineVersion::compute(&cfg, &res));

        let cfg = Config { enable_topical: false, ..Config::default() };
        assert_ne!(base, PipelineVersion::compute(&cfg, &res));
    }

    #[test]
    fn changes_with_resources() {
        let cfg = Config::default();
        let base = PipelineVersion::compute(&cfg, &Resources::default());

        let mut res = Resources::default();
        res.wordlist_digest[0] ^= 0xff;
        assert_ne!(base, PipelineVersion::compute(&cfg, &res));

    }

    #[test]
    fn tracks_parser_versions() {
        // The build script must actually find the tracked crates in Cargo.lock, or the
        // fingerprint would silently stop covering parser upgrades.
        assert!(
            !PARSER_VERSIONS.is_empty(),
            "no tracked parser versions resolved; build.rs found nothing in Cargo.lock"
        );
    }
}
