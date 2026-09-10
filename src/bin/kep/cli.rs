//! Argument surface.
//!
//! Config is exposed two ways deliberately. `--config` is the reproducible path: a file
//! you can commit next to the numbers it produced. The individual overrides exist only
//! for the handful of fields that tuning actually touches, so a sweep does not have to
//! write a temporary file per point. Anything else is changed by editing a config file,
//! which is what `kep config` exists to hand you.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use keyword_extraction_pipeline::{Config, Kind};

#[derive(Parser, Debug)]
#[command(
    name = "kep",
    version,
    about = "Deterministic tier-1 keyword and identifier extraction.",
    long_about = "Raw documents in, ranked keywords out.\n\n\
        Every keyword carries a `kind` (Identifier, Technical, Topical) and scores are \
        comparable only within a kind. Output is uncapped above a per-kind threshold, so \
        a consumer imposes its own top-N with --top or by filtering on `rank`.\n\n\
        Extraction is a pure function of (bytes, config, resources), summarised by the \
        pipeline_version printed in each run's header. If that stamp changes, previously \
        stored keyword sets are stale."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Extract keywords from files or directories.
    Extract(ExtractArgs),
    /// Account for one document's scores, feature by feature, including near-misses.
    Explain(ExplainArgs),
    /// Print the effective configuration and the pipeline_version it produces.
    Config(ConfigArgs),
    /// Print version provenance: crate, logic revision, parsers, resources.
    Version,
}

#[derive(Args, Debug)]
pub struct ExtractArgs {
    /// Files or directories. Directories are walked depth-first.
    #[arg(required = true, value_name = "PATH")]
    pub paths: Vec<PathBuf>,

    /// Output format. Defaults to `table` on a terminal and `jsonl` when redirected.
    #[arg(long, value_enum)]
    pub format: Option<Format>,

    /// Show only these kinds. Repeatable.
    #[arg(long, value_enum, value_name = "KIND")]
    pub kind: Vec<KindArg>,

    /// Keep at most N keywords per kind per document.
    #[arg(long, value_name = "N")]
    pub top: Option<usize>,

    /// Include Lane 1's feature vector on every keyword.
    #[arg(long)]
    pub features: bool,

    /// Treat any non-Ok document status as a failure (exit 2).
    #[arg(long)]
    pub strict: bool,

    /// Suppress the run header and status trailer on stderr.
    #[arg(long, short)]
    pub quiet: bool,

    #[command(flatten)]
    pub config: ConfigOpts,
}

#[derive(Args, Debug)]
pub struct ExplainArgs {
    /// The document to account for.
    pub path: PathBuf,

    /// Also show candidates that fell below their kind's threshold.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub near_misses: bool,

    /// Show at most N rejected candidates.
    #[arg(long, value_name = "N", default_value_t = 15)]
    pub near_miss_limit: usize,

    #[command(flatten)]
    pub config: ConfigOpts,
}

#[derive(Args, Debug)]
pub struct ConfigArgs {
    #[command(flatten)]
    pub config: ConfigOpts,
}

/// Shared config resolution: a base file, then targeted overrides.
#[derive(Args, Debug, Clone)]
pub struct ConfigOpts {
    /// Base configuration as JSON. Fields may be omitted; defaults fill the gaps.
    /// Produce a starting point with `kep config`.
    #[arg(long, value_name = "FILE", global = true)]
    pub config_file: Option<PathBuf>,

    /// How many frequency-ranked wordlist entries count as ordinary English.
    #[arg(long, value_name = "N")]
    pub wordlist_size: Option<usize>,

    /// Minimum shape score for a keyword to be emitted as an Identifier.
    #[arg(long, value_name = "SCORE")]
    pub threshold_identifier: Option<f32>,

    /// Minimum shape score for a keyword to be emitted as Technical.
    #[arg(long, value_name = "SCORE")]
    pub threshold_technical: Option<f32>,
}

/// Where the effective config came from, for the run header.
pub struct Resolved {
    pub config: Config,
    pub provenance: String,
}

impl ConfigOpts {
    pub fn resolve(&self) -> Result<Resolved, String> {
        let mut notes: Vec<String> = Vec::new();

        let mut config = match &self.config_file {
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                notes.push(path.display().to_string());
                serde_json::from_str(&text)
                    .map_err(|e| format!("{}: {e}", path.display()))?
            }
            None => Config::default(),
        };

        if let Some(n) = self.wordlist_size {
            config.wordlist_size = n;
            notes.push(format!("wordlist_size={n}"));
        }
        if let Some(t) = self.threshold_identifier {
            config.thresholds.identifier = t;
            notes.push(format!("threshold_identifier={t}"));
        }
        if let Some(t) = self.threshold_technical {
            config.thresholds.technical = t;
            notes.push(format!("threshold_technical={t}"));
        }

        let provenance =
            if notes.is_empty() { "defaults".to_string() } else { notes.join(", ") };
        Ok(Resolved { config, provenance })
    }
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Aligned, human-readable, one block per document.
    Table,
    /// One JSON object per line. The batch interchange format.
    Jsonl,
    /// A single pretty-printed JSON array.
    Json,
    /// One row per keyword, with the document path repeated.
    Csv,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum KindArg {
    Identifier,
    Technical,
    Topical,
}

impl From<KindArg> for Kind {
    fn from(k: KindArg) -> Self {
        match k {
            KindArg::Identifier => Kind::Identifier,
            KindArg::Technical => Kind::Technical,
            KindArg::Topical => Kind::Topical,
        }
    }
}
