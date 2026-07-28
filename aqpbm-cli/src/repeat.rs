//! `--repeats R` — the only way this tool can honestly publish a confidence
//! interval on throughput. A repeat is a whole process, so each gets a fresh
//! arena, ASLR layout, governor ramp and page-cache state: exactly the variance
//! that cancels within one process. Accuracy does not use this axis — its error
//! is deterministic given (data, parameters), so it varies the data instead.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use aqpbm_core::aggregation::welford::Welford;
use aqpbm_core::report::{BenchSection, Record, RunStats};

/// Marks a child so it runs exactly one repeat and writes to stdout, whatever
/// its argv says. The argv is byte-identical to the parent's — provably the same
/// measurement — and reading this variable is what stops it recursing.
const CHILD_ENV: &str = "APPROXBENCH_REPEAT_CHILD";

pub fn is_child() -> bool {
    std::env::var_os(CHILD_ENV).is_some()
}

/// Identifies the measurement a record belongs to across repeats: algorithm, impl,
/// params, workload, pass. The pass matters because one invocation emits several
/// records sharing the rest, and pooling them averages two populations.
type GroupKey = (String, String, String, String, String);

fn group_key(r: &Record) -> GroupKey {
    (
        r.sketch.clone(),
        r.impl_name.clone(),
        r.sketch_config
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default(),
        serde_json::to_string(&r.workload).unwrap_or_default(),
        r.bench
            .as_ref()
            .and_then(|b| b.pass.clone())
            .unwrap_or_default(),
    )
}

/// Spawn `repeats` fresh processes over this exact argv, then return one
/// merged record per measurement. Grouping is ordered so output is stable.
pub fn run_repeats(repeats: usize) -> Result<Vec<Record>> {
    let exe = std::env::current_exe().context("locating own executable for --repeats")?;
    let argv: Vec<OsString> = std::env::args_os().skip(1).collect();
    let mut groups: BTreeMap<GroupKey, Vec<Record>> = BTreeMap::new();

    for i in 0..repeats {
        eprintln!("approxbench: repeat {}/{repeats}", i + 1);
        let out = Command::new(&exe)
            .args(&argv)
            .env(CHILD_ENV, "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .output()
            .with_context(|| format!("spawning repeat {}", i + 1))?;
        if !out.status.success() {
            bail!("repeat {} exited with {}", i + 1, out.status);
        }
        let text = String::from_utf8(out.stdout).context("repeat emitted non-UTF-8")?;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let rec: Record = serde_json::from_str(line)
                .with_context(|| format!("parsing a record from repeat {}", i + 1))?;
            groups.entry(group_key(&rec)).or_default().push(rec);
        }
    }

    if groups.is_empty() {
        bail!("no repeat produced any record");
    }
    Ok(groups.into_values().map(merge).collect())
}

/// Per-repeat means of one statistic, and the interval they support.
fn across<'a>(
    records: impl Iterator<Item = &'a Record>,
    pick: fn(&BenchSection) -> Option<RunStats>,
) -> (Option<RunStats>, Vec<f64>) {
    let mut w = Welford::new();
    let mut samples = Vec::new();
    for r in records {
        if let Some(s) = r.bench.as_ref().and_then(pick) {
            w.push(s.mean);
            samples.push(s.mean);
        }
    }
    if w.n() == 0 {
        return (None, samples);
    }
    // A single sample is not an interval, and `n == 1` is reachable because
    // repeats need not all emit the same record set. `Welford` would hand back
    // a zero-width `[mean, mean]` under a field that promises trustworthiness.
    let ci95 = if w.n() >= 2 {
        let (lo, hi) = w.ci95();
        Some([lo, hi])
    } else {
        None
    };
    (
        Some(RunStats {
            mean: w.mean(),
            stddev: w.stddev(),
            ci95,
            n: w.n(),
        }),
        samples,
    )
}

/// Collapse one measurement's per-process records into one. Timings recompute
/// **across** repeats, each contributing its mean as one sample. Everything else
/// comes from the first: deterministic, or a summary averaging would distort.
fn merge(records: Vec<Record>) -> Record {
    let repeats = records.len();
    let mut base = records[0].clone();
    let Some(bench) = base.bench.as_mut() else {
        return base;
    };

    let (tput, samples) = across(records.iter(), |b| b.throughput_items_per_sec);
    if tput.is_some() {
        bench.throughput_items_per_sec = tput;
        // The per-repeat means, not the within-process iterations: the samples
        // and the interval computed from them must describe one population.
        bench.throughput_samples = Some(samples);
    }
    // Pooled on its own rather than derived from the pooled ingest rate: the
    // two columns are means of ratios, and `items / (insert + finalize)` is
    // not recoverable from `items / insert`.
    if let (Some(b), _) = across(records.iter(), |b| b.build_throughput_items_per_sec) {
        bench.build_throughput_items_per_sec = Some(b);
    }
    if let (Some(f), _) = across(records.iter(), |b| b.finalize_time_ms) {
        bench.finalize_time_ms = Some(f);
    }
    if let (Some(q), _) = across(records.iter(), |b| b.query_throughput_items_per_sec) {
        bench.query_throughput_items_per_sec = Some(q);
    }
    if let (Some(wall), _) = across(records.iter(), |b| b.wall_time_ms) {
        bench.wall_time_ms = Some(wall);
    }

    base.runs = repeats;
    base
}
