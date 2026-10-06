//! `approxbench atomic-costs` — reduce a `--flat` `MergedRecord` JSONL stream
//! (produced by looping `sketchbench` over a param grid, see
//! `scripts/export_atomic_costs.sh`) into the flat cost table ASAPQuery's
//! optimizer loads. The reduction itself lives in `aqpbm_core::atomic_costs`;
//! this is just the CLI's read-stdin-or-file / write-stdout-or-file plumbing.

use std::io::Read;

use anyhow::{Context, Result};
use clap::Parser;

use aqpbm_core::{reduce_all, same_cell, MergedRecord};

#[derive(Parser, Debug)]
pub struct AtomicCostsArgs {
    /// `--flat` JSONL input (one `MergedRecord` per line). `-` for stdin.
    #[arg(default_value = "-")]
    input: String,
    /// Output path for the reduced table (JSON array). Defaults to stdout.
    #[arg(short, long)]
    output: Option<String>,
    /// `--flat` merge-accuracy rows (`--operations merge --metrics accuracy`),
    /// one per cell and `--merge-shards`. Each lands in its cell's
    /// `merge_accuracy`; one that matches no input row is an error.
    #[arg(long)]
    merge_accuracy: Option<String>,
}

fn read_records(path: &str) -> Result<Vec<MergedRecord>> {
    let raw = if path == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading stdin")?;
        buf
    } else {
        std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?
    };
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).with_context(|| format!("parsing MergedRecord: {line}"))
        })
        .collect()
}

pub fn run(args: AtomicCostsArgs) -> Result<()> {
    let records = read_records(&args.input)?;
    let merge_runs = match args.merge_accuracy.as_deref() {
        Some(path) => read_records(path)?,
        None => Vec::new(),
    };
    if let Some(run) = merge_runs
        .iter()
        .find(|run| !records.iter().any(|record| same_cell(record, run)))
    {
        anyhow::bail!(
            "merge-accuracy row {} {} matches no input row",
            run.sketch,
            run.sketch_config
                .as_ref()
                .map(|c| c.to_string())
                .unwrap_or_default()
        );
    }

    let (table, skipped) = reduce_all(&records, &merge_runs);
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
            record.library,
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
