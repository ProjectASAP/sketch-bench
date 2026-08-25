//! `approxbench flatten` — group raw (non-`--flat`) `Record` JSONL by
//! `(sketch, impl, sketch_config, dataset)` identity and fold each group into
//! one `MergedRecord` line with `flatten_record`, the same reduction one
//! `--flat` invocation applies to its own records. This is the "downstream
//! group-by" `main.rs`'s own doc comment on `--flat` describes — for records
//! that arrive from *several* invocations of the same cell (e.g. a cost pass
//! and a separate accuracy pass, which cannot share one invocation: `cost`
//! and `query`'s accuracy metric admit different operations) rather than
//! from one.

use std::io::Read;

use anyhow::{Context, Result};
use clap::Parser;

use aqpbm_core::benchmark_result::Record;

use crate::flatten_record;

#[derive(Parser, Debug)]
pub struct FlattenArgs {
    /// Raw (non-`--flat`) JSONL input, any number of `sketchbench`
    /// invocations concatenated. `-` for stdin.
    #[arg(default_value = "-")]
    input: String,
    /// Output path for the flattened JSONL. Defaults to stdout.
    #[arg(short, long)]
    output: Option<String>,
    #[arg(
        long,
        default_value_t = false,
        help = "Indent each row over several lines instead of one JSON object per line."
    )]
    pretty_print: bool,
}

/// The identity `flatten_record` folds by, serialised so grouping needs
/// neither `Eq` nor `Hash` on `Record`'s own field types (`sketch_config` is
/// loose JSON, `input_dataset` derives neither). Two records from the same
/// grid point serialise identically because both were built from the same
/// CLI args through the same code path, not because this sorts keys.
fn identity_of(record: &Record) -> Result<String> {
    let key = serde_json::json!({
        "sketch": record.sketch,
        "impl": record.library,
        "sketch_config": record.sketch_config,
        "workload": record.input_dataset,
    });
    serde_json::to_string(&key).context("serialising identity key")
}

pub fn run(args: FlattenArgs) -> Result<()> {
    let raw = if args.input == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading stdin")?;
        buf
    } else {
        std::fs::read_to_string(&args.input).with_context(|| format!("reading {}", args.input))?
    };

    let records: Vec<Record> = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).with_context(|| format!("parsing Record: {line}")))
        .collect::<Result<_>>()?;

    // Grouped by first-seen order, not a `HashMap`, so the output's row order
    // matches the input's — a diff between two exports stays readable.
    let mut order: Vec<String> = Vec::new();
    let mut groups: std::collections::HashMap<String, Vec<Record>> =
        std::collections::HashMap::new();
    for record in records {
        let key = identity_of(&record)?;
        if !groups.contains_key(&key) {
            order.push(key.clone());
        }
        groups.entry(key).or_default().push(record);
    }

    let mut sink = crate::report_sink::ReportSink::open(args.output.as_deref())?;
    for key in &order {
        let group = &groups[key];
        let merged = flatten_record::flatten_record(group).map_err(|e| anyhow::anyhow!("{e}"))?;
        if args.pretty_print {
            sink.write_pretty(&merged)?;
        } else {
            sink.write_line(&serde_json::to_string(&merged)?)?;
        }
    }

    eprintln!(
        "approxbench flatten: {} record(s), {} row(s)",
        groups.values().map(Vec::len).sum::<usize>(),
        order.len()
    );
    Ok(())
}
