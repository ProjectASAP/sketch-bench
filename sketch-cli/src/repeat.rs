//! `--repeats R` — the only way this tool can honestly publish a confidence
//! interval on an implementation's throughput.
//!
//! ## Why a fresh process, and not just more `--runs`
//!
//! `--runs N` measures N back-to-back iterations inside one process: one core,
//! one allocator arena, one address-space layout, one governor ramp, one
//! already-resident item slice. Those iterations estimate how much the last
//! few seconds of *that process* wobbled. They are not independent samples of
//! "the throughput of this implementation", so `mean ± 1.96·stddev/√N` over
//! them yields an interval far tighter than the command's own reproducibility.
//! The same command, four times, before this existed:
//!
//! ```text
//! 94.85 M/s  ci95 [94.69, 95.02]
//! 95.81 M/s  ci95 [95.62, 96.01]
//! 96.68 M/s  ci95 [96.60, 96.75]
//! 97.24 M/s  ci95 [97.08, 97.40]
//! ```
//!
//! Four mutually disjoint 95% intervals for one measurement. Comparing two
//! implementations by whether their intervals overlap — which is what an
//! interval is *for* — would have called differences significant that a re-run
//! inverts. `docs/DESIGN.md` §5.7 used to prescribe raising `--runs` as the
//! remedy for a wide interval; that makes it narrower and more wrong.
//!
//! So a repeat is a whole process. Each one gets a fresh arena, a fresh ASLR
//! layout, a fresh ramp, and fresh page-cache state — exactly the variance
//! that cancels within a process and shows up between invocations.
//!
//! Accuracy deliberately does not use this axis: a sketch's error is a
//! deterministic function of (data, parameters), with no process-level
//! variance to sample. Its repetitions vary the *data*, in-process.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use sketch_bench::aggregation::welford::Welford;
use sketch_core::report::{BenchSection, Record, RunStats};

/// Marks a child so it runs exactly one repeat and writes to stdout,
/// whatever `--repeats` / `--report` its argv says. The child's argv is
/// byte-identical to the parent's, which is what makes it provably the same
/// measurement — and reading this variable is also what stops it recursing.
const CHILD_ENV: &str = "SKETCHLIB_REPEAT_CHILD";

pub fn is_child() -> bool {
    std::env::var_os(CHILD_ENV).is_some()
}

/// Which pass produced a record. `MetricsMask::passes()` guarantees a record
/// carries at most one primary metric, so this is recoverable from content and
/// needs no schema field. Records must be grouped by it before merging:
/// throughput and latency records for the same impl describe different runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Pass {
    Throughput,
    Latency,
    Accuracy,
    Merge,
    Other,
}

fn pass_of(b: &BenchSection) -> Pass {
    // Merge first: the merge pass publishes post-merge accuracy, so it also
    // sets `accuracy` and would otherwise be indistinguishable from the
    // accuracy pass — the two would share a group key, `merge` would keep one
    // and discard the other, and `query_throughput` would average two
    // different populations under one `n`.
    if b.merge_shards.is_some() {
        Pass::Merge
    } else if b.throughput_items_per_sec.is_some() {
        Pass::Throughput
    } else if b.latency_ns.is_some() {
        Pass::Latency
    } else if b.accuracy.is_some() {
        Pass::Accuracy
    } else {
        Pass::Other
    }
}

/// Identifies the measurement a record belongs to, across repeats.
type GroupKey = (String, String, String, String, Pass);

fn group_key(r: &Record) -> GroupKey {
    (
        r.sketch.clone(),
        r.impl_name.clone(),
        r.sketch_config
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default(),
        serde_json::to_string(&r.workload).unwrap_or_default(),
        r.bench.as_ref().map(pass_of).unwrap_or(Pass::Other),
    )
}

/// Spawn `repeats` fresh processes over this exact argv, then return one
/// merged record per measurement. Grouping is ordered so output is stable.
pub fn run_repeats(repeats: usize) -> Result<Vec<Record>> {
    let exe = std::env::current_exe().context("locating own executable for --repeats")?;
    let argv: Vec<OsString> = std::env::args_os().skip(1).collect();
    let mut groups: BTreeMap<GroupKey, Vec<Record>> = BTreeMap::new();

    for i in 0..repeats {
        eprintln!("sketchlib: repeat {}/{repeats}", i + 1);
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
    // A single sample is not an interval. Repeats need not all emit the same
    // record set — an implementation can be skipped for one config, or a
    // pass can produce nothing — so `n == 1` is reachable, and `Welford`
    // would hand back a zero-width `[mean, mean]` under a field whose whole
    // premise is that its presence means it can be trusted.
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

/// Collapse one measurement's per-process records into one.
///
/// Timing statistics are recomputed **across** repeats: each repeat
/// contributes its own mean as one sample, so `n` becomes the repeat count and
/// the interval describes spread between invocations. Everything else —
/// memory, latency percentiles, accuracy — is taken from the first repeat: it
/// is either deterministic given the inputs, or a within-process summary that
/// averaging would misrepresent.
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
    if let (Some(q), _) = across(records.iter(), |b| b.query_throughput_items_per_sec) {
        bench.query_throughput_items_per_sec = Some(q);
    }
    if let (Some(wall), _) = across(records.iter(), |b| b.wall_time_ms) {
        bench.wall_time_ms = Some(wall);
    }

    base.runs = repeats;
    base
}
