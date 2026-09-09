//! `approxbench erp` — reduce flat benchmark JSONL to an ERP v1 artifact.

use std::io::Read;

use anyhow::{Context, Result};
use aqpbm_core::{erp_artifact, MergedRecord};
use clap::Parser;

#[derive(Parser, Debug)]
pub struct ErpArgs {
    /// `--flat` JSONL input (one MergedRecord per line). `-` for stdin.
    #[arg(default_value = "-")]
    input: String,
    /// Stable producer revision (normally the sketch-bench git SHA).
    #[arg(long)]
    producer_version: String,
    /// Output path. Defaults to stdout.
    #[arg(short, long)]
    output: Option<String>,
}

pub fn run(args: ErpArgs) -> Result<()> {
    let raw = if args.input == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading stdin")?;
        buf
    } else {
        std::fs::read_to_string(&args.input).with_context(|| format!("reading {}", args.input))?
    };
    let records: Vec<MergedRecord> = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).context("parsing MergedRecord"))
        .collect::<Result<_>>()?;
    let (artifact, skipped) = erp_artifact(&records, args.producer_version);
    for (index, reason) in &skipped {
        eprintln!("approxbench erp: skipping row {index}: {reason}");
    }
    let json = serde_json::to_string_pretty(&artifact)?;
    match args.output.as_deref() {
        None | Some("-") => println!("{json}"),
        Some(path) => std::fs::write(path, json).with_context(|| format!("writing {path}"))?,
    }
    eprintln!(
        "approxbench erp: {} row(s), {} skipped",
        artifact.records.len(),
        skipped.len()
    );
    Ok(())
}
