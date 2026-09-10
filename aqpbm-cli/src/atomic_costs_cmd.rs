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
use serde::Deserialize;

use aqpbm_core::{reduce_all, MergedRecord, ScenarioIdentity, SCHEMA_VERSION};

#[derive(Parser, Debug)]
pub struct AtomicCostsArgs {
    /// `--flat` JSONL input (one `MergedRecord` per line). `-` for stdin.
    #[arg(default_value = "-")]
    input: String,
    /// Output path for the reduced document (JSON). Defaults to stdout.
    #[arg(short, long)]
    output: Option<String>,
    /// Dataset-wrangler scenario manifest identifying the exact CSV measured.
    #[arg(long)]
    scenario_manifest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioManifest {
    scenario_spec: ScenarioSpec,
    output_payload_sha256: String,
    source_sha256: String,
    output_sha256: String,
    records_loaded: u64,
    grouping_state_count: usize,
    arrival_rate_hz: f64,
    duplicate_policy: String,
    schema_version: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScenarioSpec {
    exported_metric: String,
    grouping_labels: Vec<String>,
    source_time_range_us: [i64; 2],
    source_file: String,
    rebase_to_offset: bool,
}

fn load_scenario_identity(path: &str) -> Result<ScenarioIdentity> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
    let manifest: ScenarioManifest =
        serde_json::from_str(&raw).with_context(|| format!("parsing scenario manifest {path}"))?;
    if manifest.schema_version != 1 {
        bail!(
            "scenario manifest {path} has unsupported schema_version {}",
            manifest.schema_version
        );
    }
    for (field, hash) in [
        ("source_sha256", &manifest.source_sha256),
        ("output_sha256", &manifest.output_sha256),
        ("output_payload_sha256", &manifest.output_payload_sha256),
    ] {
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("scenario manifest {path} has invalid {field}");
        }
    }
    if manifest.scenario_spec.source_file.is_empty()
        || manifest.scenario_spec.exported_metric.is_empty()
        || manifest.scenario_spec.grouping_labels.is_empty()
    {
        bail!("scenario manifest {path} has no grouping_labels");
    }
    if manifest.scenario_spec.source_time_range_us[0]
        >= manifest.scenario_spec.source_time_range_us[1]
    {
        bail!("scenario manifest {path} has an empty or inverted source_time_range_us");
    }
    if manifest.records_loaded == 0
        || manifest.grouping_state_count == 0
        || !manifest.arrival_rate_hz.is_finite()
        || manifest.arrival_rate_hz <= 0.0
        || manifest.duplicate_policy != "fail"
    {
        bail!("scenario manifest {path} violates the required wrangler invariants");
    }
    let _timestamp_rebased = manifest.scenario_spec.rebase_to_offset;
    Ok(ScenarioIdentity {
        payload_sha256: manifest.output_payload_sha256,
        exported_metric: manifest.scenario_spec.exported_metric,
        grouping_labels: manifest.scenario_spec.grouping_labels,
        source_time_range_us: manifest.scenario_spec.source_time_range_us,
    })
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
    let scenario = load_scenario_identity(&args.scenario_manifest)?;

    let (document, skipped) = reduce_all(&records, scenario);
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
    use std::time::{SystemTime, UNIX_EPOCH};

    fn manifest(payload_sha256: &str) -> String {
        serde_json::json!({
            "schema_version": 1,
            "scenario_spec": {
                "source_file": "google/task_usage.csv.gz",
                "source_time_range_us": [1313535000000_i64, 1313715000000_i64],
                "exported_metric": "google_mean_cpu_usage_rate_0",
                "grouping_labels": ["job_id", "task_index", "machine_id"],
                "rebase_to_offset": true
            },
            "source_sha256": "b".repeat(64),
            "output_sha256": "c".repeat(64),
            "output_payload_sha256": payload_sha256,
            "records_loaded": 20051,
            "grouping_state_count": 10379,
            "arrival_rate_hz": 111.394,
            "duplicate_policy": "fail"
        })
        .to_string()
    }

    fn write_manifest(contents: String) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("aqpbm-scenario-{nonce}.json"));
        std::fs::write(&path, contents).unwrap();
        path
    }

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

    #[test]
    fn scenario_manifest_becomes_profile_identity() {
        let path = write_manifest(manifest(&"a".repeat(64)));

        let identity = load_scenario_identity(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(path).unwrap();

        assert_eq!(identity.payload_sha256, "a".repeat(64));
        assert_eq!(identity.exported_metric, "google_mean_cpu_usage_rate_0");
        assert_eq!(
            identity.grouping_labels,
            ["job_id", "task_index", "machine_id"]
        );
    }

    #[test]
    fn scenario_manifest_rejects_invalid_payload_hash() {
        let path = write_manifest(manifest("not-a-sha"));

        let error = load_scenario_identity(path.to_str().unwrap()).unwrap_err();
        std::fs::remove_file(path).unwrap();

        assert!(error.to_string().contains("output_payload_sha256"));
    }
}
