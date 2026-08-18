//! `approxbench atomic-costs` — reduce a `--flat` `MergedRecord` JSONL stream
//! (produced by looping `sketchbench` over a param grid, see
//! `scripts/export_atomic_costs.sh`) into the flat cost table ASAPQuery's
//! optimizer loads. The reduction itself lives in `aqpbm_core::atomic_costs`;
//! this is just the CLI's read-stdin-or-file / write-stdout-or-file plumbing.

use std::io::Read;

use anyhow::{Context, Result};
use clap::Parser;

use aqpbm_core::{reduce_all, MergedRecord};

#[derive(Parser, Debug)]
pub struct AtomicCostsArgs {
    /// `--flat` JSONL input (one `MergedRecord` per line). `-` for stdin.
    #[arg(default_value = "-")]
    input: String,
    /// Output path for the reduced table (JSON array). Defaults to stdout.
    #[arg(short, long)]
    output: Option<String>,
}

pub fn run(args: AtomicCostsArgs) -> Result<()> {
    let raw = if args.input == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading stdin")?;
        buf
    } else {
        std::fs::read_to_string(&args.input)
            .with_context(|| format!("reading {}", args.input))?
    };

    let records: Vec<MergedRecord> = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).with_context(|| format!("parsing MergedRecord: {line}"))
        })
        .collect::<Result<_>>()?;

    let (table, skipped) = reduce_all(&records);
    for (i, reason) in &skipped {
        let record = &records[*i];
        eprintln!(
            "approxbench atomic-costs: skipping {} {} ({}): {reason}",
            record.sketch,
            record
                .sketch_config
                .as_ref()
                .map(|c| c.to_string())
                .unwrap_or_default(),
            record.impl_name,
        );
    }

    let json = serde_json::to_string_pretty(&table)?;
    match args.output.as_deref() {
        None | Some("-") => println!("{json}"),
        Some(path) => std::fs::write(path, json).with_context(|| format!("writing {path}"))?,
    }

    eprintln!(
        "approxbench atomic-costs: {} row(s), {} skipped",
        table.len(),
        skipped.len()
    );
    Ok(())
}
