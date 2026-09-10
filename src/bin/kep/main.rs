//! Batch extraction CLI.
//!
//! Exists so a corpus can be re-extracted offline without wiring the library into a
//! pipeline first, and so output can be eyeballed while tuning.
//!
//! Two conventions run through it. **Data goes to stdout, commentary goes to stderr** —
//! the run header, the status trailer and every warning — so `kep extract … > out.jsonl`
//! is a clean file and the operator still sees what happened. And **the run header always
//! names the pipeline_version**, because a stamp change silently invalidates every stored
//! keyword set, and a change nobody notices is the failure mode this crate is built to
//! avoid.

mod cli;
mod explain;
mod record;
mod render;

use std::{
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::Parser;
use keyword_extraction_pipeline::{
    extract, version::LOGIC_REVISION, Config, DocumentStatus, FormatHint,
    Kind, PipelineVersion, Resources,
};

use cli::{Cli, Command, ExtractArgs, Format};
use record::{DocumentRecord, View};

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("kep: {message}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<u8, String> {
    let cli = Cli::parse();
    match &cli.command {
        Command::Extract(args) => run_extract(args),
        Command::Explain(args) => {
            let resolved = args.config.resolve()?;
            let stdout = std::io::stdout();
            let mut out = stdout.lock();
            explain::run(args, &resolved.config, &mut out)
                .map(|c| c as u8)
                .map_err(|e| e.to_string())
        }
        Command::Config(args) => {
            let resolved = args.config.resolve()?;
            let res = Resources::for_config(&resolved.config);
            let json = serde_json::to_string_pretty(&resolved.config)
                .map_err(|e| e.to_string())?;
            println!("{json}");
            eprintln!(
                "\npipeline_version {}\nSave this to a file and pass it with --config-file.",
                PipelineVersion::compute(&resolved.config, &res)
            );
            Ok(0)
        }
        Command::Version => {
            let cfg = Config::default();
            let res = Resources::for_config(&cfg);
            let (cutoff, depth) = res.wordlist_extent();
            println!("kep {}", env!("CARGO_PKG_VERSION"));
            println!("logic_revision   {LOGIC_REVISION}");
            println!("wordlist         {cutoff} of {depth} entries active");
            println!("pipeline_version {}", PipelineVersion::compute(&cfg, &res));
            println!("  (with default config; any config change moves this stamp)");
            Ok(0)
        }
    }
}

fn run_extract(args: &ExtractArgs) -> Result<u8, String> {
    let resolved = args.config.resolve()?;
    let cfg = Config { retain_features: args.features, ..resolved.config };
    let res = Resources::for_config(&cfg);
    let version = PipelineVersion::compute(&cfg, &res);

    let stdout = std::io::stdout();
    // `--format` is authoritative; the terminal check only picks a default. Rendering is
    // the only thing it affects, so piping and redirecting never change what was
    // extracted, only how it is written down.
    let (format, inferred) = match args.format {
        Some(f) => (f, false),
        None if stdout.is_terminal() => (Format::Table, true),
        None => (Format::Jsonl, true),
    };

    let mut paths: Vec<PathBuf> = Vec::new();
    for arg in &args.paths {
        if !arg.exists() {
            return Err(format!("{}: no such file or directory", arg.display()));
        }
        collect(arg, &mut paths);
    }
    paths.sort();

    if !args.quiet {
        eprintln!(
            "kep {} document{} | format {:?}{} | config {} | pipeline_version {}",
            paths.len(),
            if paths.len() == 1 { "" } else { "s" },
            format,
            if inferred { " (inferred)" } else { "" },
            resolved.provenance,
            version.short(),
        );
    }

    let view = View { kinds: kind_mask(args), top: args.top };
    let mut out = stdout.lock();
    let mut tally = Tally::default();
    let mut records: Vec<serde_json::Value> = Vec::new();

    if format == Format::Csv {
        render::csv_header(&mut out).map_err(|e| e.to_string())?;
    }

    for path in &paths {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("kep: {}: {e}", path.display());
                tally.unreadable += 1;
                continue;
            }
        };
        let result = extract(&bytes, FormatHint::Sniff, &cfg, &res);
        tally.count(&result.status);

        let display = path.display().to_string();
        let record = DocumentRecord::new(&display, &result, view);
        let written = match format {
            Format::Jsonl => render::jsonl(&mut out, &record),
            Format::Csv => render::csv(&mut out, &record),
            Format::Table => render::table(&mut out, &record, &result, &cfg),
            Format::Json => {
                records.push(serde_json::to_value(&record).expect("record is serialisable"));
                Ok(())
            }
        };
        // A closed pipe is `head` doing its job, not a failure. Anything else is.
        if let Err(e) = written {
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                return Ok(0);
            }
            return Err(e.to_string());
        }
    }

    if format == Format::Json {
        let json = serde_json::to_string_pretty(&records).map_err(|e| e.to_string())?;
        if let Err(e) = writeln!(out, "{json}") {
            if e.kind() != std::io::ErrorKind::BrokenPipe {
                return Err(e.to_string());
            }
        }
    }
    let _ = out.flush();

    if !args.quiet {
        eprintln!("{}", tally.summary());
    }

    // An unreadable file is an error on our side of the contract. A non-Ok status is
    // not: `NoTextLayer` on a scanned PDF is the pipeline working correctly, and failing
    // a corpus pass over it would make the exit code useless. `--strict` is for callers
    // who really do want every document to have produced keywords.
    Ok(if tally.unreadable > 0 {
        1
    } else if args.strict && tally.ok != paths.len() {
        2
    } else {
        0
    })
}

fn collect(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else { return };
        let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        children.sort();
        for child in children {
            collect(&child, out);
        }
    } else if path.is_file() {
        out.push(path.to_path_buf());
    }
}

fn kind_mask(args: &ExtractArgs) -> Option<[bool; 3]> {
    if args.kind.is_empty() {
        return None;
    }
    let mut mask = [false; 3];
    for k in &args.kind {
        mask[Kind::from(*k) as usize] = true;
    }
    Some(mask)
}

/// Per-status counts for the trailer.
///
/// Printed on every run, including the all-clear. A corpus pass that quietly dropped
/// half its documents and a corpus pass that succeeded look identical without it.
#[derive(Default)]
struct Tally {
    ok: usize,
    no_text_layer: usize,
    encrypted: usize,
    unsupported: usize,
    parse_error: usize,
    too_short: usize,
    unreadable: usize,
}

impl Tally {
    fn count(&mut self, status: &DocumentStatus) {
        match status {
            DocumentStatus::Ok => self.ok += 1,
            DocumentStatus::NoTextLayer => self.no_text_layer += 1,
            DocumentStatus::Encrypted => self.encrypted += 1,
            DocumentStatus::UnsupportedFormat => self.unsupported += 1,
            DocumentStatus::ParseError(_) => self.parse_error += 1,
            DocumentStatus::TooShort => self.too_short += 1,
        }
    }

    fn summary(&self) -> String {
        let total = self.ok
            + self.no_text_layer
            + self.encrypted
            + self.unsupported
            + self.parse_error
            + self.too_short
            + self.unreadable;
        let parts = [
            ("ok", self.ok),
            ("no-text-layer", self.no_text_layer),
            ("encrypted", self.encrypted),
            ("unsupported", self.unsupported),
            ("parse-error", self.parse_error),
            ("too-short", self.too_short),
            ("unreadable", self.unreadable),
        ];
        let detail: Vec<String> = parts
            .iter()
            .filter(|(_, n)| *n > 0)
            .map(|(name, n)| format!("{n} {name}"))
            .collect();
        format!(
            "kep {total} document{}: {}",
            if total == 1 { "" } else { "s" },
            if detail.is_empty() { "nothing to do".to_string() } else { detail.join(", ") }
        )
    }
}
