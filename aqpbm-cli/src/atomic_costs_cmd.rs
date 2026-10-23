//! `approxbench atomic-costs` — reduce a `--flat` `MergedRecord` JSONL stream
//! (produced by looping `sketchbench` over a param grid, see
//! `scripts/export_atomic_costs.sh`) into the flat cost table ASAPQuery's
//! optimizer loads. The reduction itself lives in `aqpbm_core::atomic_costs`;
//! this is just the CLI's read-stdin-or-file / write-stdout-or-file plumbing.

use std::io::Read;

use anyhow::{Context, Result};
use clap::Parser;

use std::collections::BTreeMap;

use aqpbm_core::{reduce_all, MergedRecord};

#[derive(Parser, Debug)]
pub struct AtomicCostsArgs {
    /// `--flat` JSONL input (one `MergedRecord` per line). `-` for stdin.
    #[arg(default_value = "-")]
    input: String,
    /// Output path for the reduced table (JSON array). Defaults to stdout.
    #[arg(short, long)]
    output: Option<String>,
    /// `VARIANT=METRIC`, once per variant: the `query_accuracy` key that is
    /// that variant's `accuracy_metric`. A row whose variant has none, or
    /// whose scores lack it, is skipped.
    #[arg(long = "accuracy-metric", value_name = "VARIANT=METRIC", required = true,
          value_parser = parse_accuracy_metric)]
    accuracy_metrics: Vec<(String, String)>,
}

fn parse_accuracy_metric(arg: &str) -> Result<(String, String), String> {
    match arg.split_once('=') {
        Some((variant, metric)) if !variant.is_empty() && !metric.is_empty() => {
            Ok((variant.to_string(), metric.to_string()))
        }
        _ => Err(format!("expected VARIANT=METRIC, got {arg}")),
    }
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
    let mut accuracy_metrics = BTreeMap::new();
    for (variant, metric) in args.accuracy_metrics {
        if let Some(earlier) = accuracy_metrics.insert(variant.clone(), metric.clone()) {
            anyhow::bail!("--accuracy-metric gives {variant} twice ({earlier}, {metric})");
        }
    }
    let (table, skipped) = reduce_all(&records, &accuracy_metrics);
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
