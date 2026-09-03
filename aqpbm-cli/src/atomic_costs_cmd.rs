//! `approxbench atomic-costs` — reduce a `--flat` `MergedRecord` JSONL stream
//! (produced by looping `sketchbench` over a param grid, see
//! `scripts/export_atomic_costs.sh`) into the versioned, workload-grouped
//! atomic-cost document ASAPQuery's optimizer loads. The reduction itself
//! lives in `aqpbm_core::atomic_costs`; this is just the CLI's
//! read-stdin-or-file / write-stdout-or-file plumbing, plus the input-side
//! schema gate a JSONL stream needs that a typed [`MergedRecord`] alone can't
//! express.

use std::io::Read;

use anyhow::{bail, Context, Result};
use clap::Parser;

use aqpbm_core::{reduce_all, MergedRecord, SCHEMA_VERSION};

#[derive(Parser, Debug)]
pub struct AtomicCostsArgs {
    /// `--flat` JSONL input (one `MergedRecord` per line). `-` for stdin.
    #[arg(default_value = "-")]
    input: String,
    /// Output path for the reduced document (JSON). Defaults to stdout.
    #[arg(short, long)]
    output: Option<String>,
}

/// Parse a `--flat` JSONL stream into `MergedRecord`s, refusing any line
/// whose `schema_version` doesn't match the report schema this build reads
/// (`benchmark_result::SCHEMA_VERSION`) — a version mismatch is a producer
/// running a different sketch-bench than this reducer, and silently reducing
/// it anyway risks reading fields that changed meaning across the bump. Named
/// separately from `run` so it can be exercised without a file or stdin.
fn parse_records(raw: &str) -> Result<Vec<MergedRecord>> {
    raw.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(i, line)| {
            let line_no = i + 1;
            let record: MergedRecord = serde_json::from_str(line)
                .with_context(|| format!("line {line_no}: parsing MergedRecord: {line}"))?;
            if record.schema_version != SCHEMA_VERSION {
                bail!(
                    "line {line_no}: unsupported MergedRecord schema_version {} (this build \
                     supports schema_version {SCHEMA_VERSION})",
                    record.schema_version,
                );
            }
            Ok(record)
        })
        .collect()
}

pub fn run(args: AtomicCostsArgs) -> Result<()> {
    let raw = if args.input == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading stdin")?;
        buf
    } else {
        std::fs::read_to_string(&args.input).with_context(|| format!("reading {}", args.input))?
    };

    let records = parse_records(&raw)?;

    let (document, skipped) = reduce_all(&records);
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
    let document = document.context("reducing to atomic-cost document")?;

    let json = serde_json::to_string_pretty(&document)?;
    match args.output.as_deref() {
        None | Some("-") => println!("{json}"),
        Some(path) => std::fs::write(path, json).with_context(|| format!("writing {path}"))?,
    }

    eprintln!(
        "approxbench atomic-costs: {} profile(s), {} entrie(s), {} skipped",
        document.profiles.len(),
        document
            .profiles
            .iter()
            .map(|p| p.entries.len())
            .sum::<usize>(),
        skipped.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(schema_version: u32) -> String {
        serde_json::json!({
            "schema_version": schema_version,
            "sketch": "cms",
            "impl": "lib",
            "language": "rust",
            "mode": "bench",
            "runs": 1,
            "source": "cli",
            "sketch_config": null,
            "workload": {"synthetic": {"description": {
                "column_num": 1,
                "column_label": ["key"],
                "column_spec": [],
                "row_num": 1,
            }}},
            "memory_bytes": null,
            "heap_bytes_net": null,
            "heap_bytes_peak": null,
        })
        .to_string()
    }

    #[test]
    fn parses_matching_schema_version() {
        let records = parse_records(&line(SCHEMA_VERSION)).unwrap();
        assert_eq!(records.len(), 1);
    }

    #[test]
    fn rejects_mismatched_schema_version_naming_line_and_versions() {
        let raw = format!("{}\n{}", line(SCHEMA_VERSION), line(SCHEMA_VERSION - 1));
        let err = parse_records(&raw).expect_err("mismatched schema_version must be rejected");
        let msg = err.to_string();
        assert!(msg.contains("line 2"), "{msg}");
        assert!(msg.contains(&(SCHEMA_VERSION - 1).to_string()), "{msg}");
        assert!(msg.contains(&SCHEMA_VERSION.to_string()), "{msg}");
    }
}
