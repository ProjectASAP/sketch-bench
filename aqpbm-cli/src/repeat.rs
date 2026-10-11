//! `--repeat-experiment R` — the only way this tool can honestly publish a confidence
//! interval on throughput. A repeat is a whole process, so each gets a fresh
//! arena, ASLR layout, governor ramp and page-cache state: exactly the variance
//! that cancels within one process. Accuracy does not use this axis — its error
//! is deterministic given (data, parameters), so it varies the data instead.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use aqpbm_core::benchmark_result::schema::{BenchSection, CpuTime, Record, RunStats};
use aqpbm_core::benchmark_result::welford::Welford;

/// Marks a child so it runs exactly one repeat and writes to stdout, whatever
/// its argv says. The argv is byte-identical to the parent's — provably the same
/// measurement — and reading this variable is what stops it recursing.
const CHILD_ENV: &str = "APPROXBENCH_REPEAT_CHILD";

pub fn is_child() -> bool {
    std::env::var_os(CHILD_ENV).is_some()
}

/// Identifies the measurement a record belongs to across repeats: algorithm,
/// impl, params, dataset, operation, metric.
/// The last two matter because one invocation emits several records sharing the
/// rest, and pooling two of them averages two populations.
/// Both are needed, since one metric over two operations is two measurements.
type GroupKey = (String, String, String, String, String, String);

fn group_key(r: &Record) -> GroupKey {
    (
        r.sketch.clone(),
        r.library.clone(),
        r.sketch_config
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default(),
        serde_json::to_string(&r.input_dataset).unwrap_or_default(),
        r.bench
            .as_ref()
            .and_then(|b| b.operation.clone())
            .unwrap_or_default(),
        r.bench
            .as_ref()
            .and_then(|b| b.metric.clone())
            .unwrap_or_default(),
    )
}

/// Spawn `repeats` fresh processes over this exact argv, then return one
/// merged record per measurement. Grouping is ordered so output is stable.
pub fn run_repeats(repeats: usize) -> Result<Vec<Record>> {
    let exe = std::env::current_exe().context("locating own executable for --repeat-experiment")?;
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
    pick: fn(&BenchSection) -> Option<&RunStats>,
) -> Option<RunStats> {
    let mut w = Welford::new();
    let mut samples = Vec::new();
    for r in records {
        if let Some(s) = r.bench.as_ref().and_then(pick) {
            w.push(s.mean);
            samples.push(s.mean);
        }
    }
    if w.n() == 0 {
        return None;
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
    Some(RunStats {
        mean: w.mean(),
        stddev: w.stddev(),
        ci95,
        n: w.n(),
        samples,
    })
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

    if let Some(tput) = across(records.iter(), |b| b.throughput_items_per_sec.as_ref()) {
        // The per-repeat means, not the within-process iterations: the samples
        // and the interval computed from them must describe one population.
        bench.throughput_items_per_sec = Some(tput);
    }
    if let Some(wall) = across(records.iter(), |b| b.wall_time_ms.as_ref()) {
        bench.wall_time_ms = Some(wall);
    }
    // The rate belongs with the time it is derived from, or a record says
    // `runs: R` over a folds/sec that describes one repeat.
    if let Some(m) = across(records.iter(), |b| b.merge_folds_per_sec.as_ref()) {
        bench.merge_folds_per_sec = Some(m);
    }
    // Same argument: CPU time is measured per process.
    if let (Some(user), Some(sys)) = (
        across(records.iter(), |b| {
            b.cpu_time_ms.as_ref().map(|c| &c.user_ms)
        }),
        across(records.iter(), |b| {
            b.cpu_time_ms.as_ref().map(|c| &c.sys_ms)
        }),
    ) {
        bench.cpu_time_ms = Some(CpuTime {
            user_ms: user,
            sys_ms: sys,
        });
    }

    base.runs = repeats;
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use aqpbm_core::benchmark_result::schema::{CpuTime, LatencySummary, Mode};
    use aqpbm_datagen::{ColumnSpec, DataDistribution, TableDescription, UniformParameter};

    fn stats(mean: f64) -> RunStats {
        // `n = 5` is the *within-process* count. After merging R processes every
        // one of these must read `n = R`, since the population changed.
        RunStats {
            mean,
            stddev: 1.0,
            ci95: None,
            n: 5,
            samples: vec![mean; 5],
        }
    }

    /// A record with every measurable field populated, so the walk below has
    /// something to find in each of them.
    fn record(mean: f64) -> Record {
        let wd = TableDescription::single(
            "key",
            ColumnSpec {
                distribution: DataDistribution::Uniform(UniformParameter {
                    lower_bound: 0.0,
                    upper_bound: 100.0,
                    seed: 1,
                }),
                shift: None,
                cardinality: None,
                special_rule: aqpbm_datagen::RULE_NONE,
                data_type: "i64".into(),
                string: None,
                child_of: None,
                fan_out: None,
            },
            1000,
        );
        let mut rec = Record::new("cms", "oxide", wd, Mode::Bench, 5);
        rec.bench = Some(BenchSection {
            metric: Some("latency".into()),
            operation: Some("merge".into()),
            throughput_items_per_sec: Some(stats(mean)),
            latency_ns: Some(LatencySummary {
                p50: 10,
                p95: 20,
                p99: 30,
                p999: 40,
                max: 50,
                count: 1000,
            }),
            cpu_time_ms: Some(CpuTime {
                user_ms: stats(12.0),
                sys_ms: stats(3.0),
            }),
            wall_time_ms: Some(stats(100.0)),
            rss_peak_kb: Some(2048),
            heap_allocated_kb: Some(1024),
            memory_bytes: Some(40960),
            heap_bytes_net: Some(40960),
            heap_bytes_peak: Some(65536),
            accuracy: Some(serde_json::json!({"are_all": 0.01, "accuracy_runs": 5})),
            merge_folds_per_sec: Some(stats(mean * 0.5)),
            merge_shards: Some(4),
            merge_supported: Some(true),
        });
        rec
    }

    /// Every `"n"` in a JSON tree, with the path that led to it.
    fn sample_counts(v: &serde_json::Value, path: &str, out: &mut Vec<(String, u64)>) {
        match v {
            serde_json::Value::Object(map) => {
                for (k, child) in map {
                    let child_path = if path.is_empty() {
                        k.clone()
                    } else {
                        format!("{path}.{k}")
                    };
                    if k == "n" {
                        if let Some(n) = child.as_u64() {
                            out.push((path.to_string(), n));
                            continue;
                        }
                    }
                    sample_counts(child, &child_path, out);
                }
            }
            serde_json::Value::Array(items) => {
                for (i, child) in items.iter().enumerate() {
                    sample_counts(child, &format!("{path}[{i}]"), out);
                }
            }
            _ => {}
        }
    }

    /// The invariant `--repeat-experiment` owes its reader: after merging R processes,
    /// **every** aggregate in the record describes those R processes. A walk over
    /// the serialised record, so a new `BenchSection` field is covered for free.
    #[test]
    fn every_aggregate_describes_the_repeat_population() {
        let repeats = 3;
        let merged = merge((0..repeats).map(|i| record(100.0 + i as f64)).collect());
        assert_eq!(merged.runs, repeats);

        let json = serde_json::to_value(merged.bench.expect("bench section")).expect("serialises");
        let mut found = Vec::new();
        sample_counts(&json, "", &mut found);

        assert!(
            found.len() >= 5,
            "expected an aggregate in every measured field, found {}: {found:?}",
            found.len()
        );
        for (field, n) in &found {
            assert_eq!(
                *n, repeats as u64,
                "`{field}` reports n={n} after {repeats} repeats, so it was not \
                 re-aggregated across processes"
            );
        }
    }

    /// Two operations read for one metric are two measurements, so the key that pools
    /// repeats has to carry the operation too. Keyed on the metric alone they
    /// pooled, and one record came back wearing the other's operation.
    #[test]
    fn one_metric_over_two_operations_stays_two_groups() {
        let mut insert = record(100.0);
        let mut query = record(200.0);
        for (r, op) in [(&mut insert, "insert"), (&mut query, "query")] {
            let b = r.bench.as_mut().expect("bench section");
            b.operation = Some(op.into());
            b.metric = Some("throughput".into());
        }
        let a = group_key(&insert);
        let b = group_key(&query);
        assert_ne!(a, b, "one key for two measurements pools two populations");
    }

    /// The reason the axis exists: R independent processes support an interval,
    /// and the merge measurement must get one like every other.
    #[test]
    fn merge_wall_time_gets_an_interval_across_repeats() {
        let merged = merge((0..3).map(|i| record(100.0 + i as f64)).collect());
        let bench = merged.bench.expect("bench section");
        let m = bench.wall_time_ms.expect("merge time survives the merge");
        assert_eq!(m.n, 3);
        assert!(
            m.ci95.is_some(),
            "three independent processes support an interval"
        );
    }

    /// One process is not an interval. `n == 1` is reachable because repeats
    /// need not all emit the same record set.
    #[test]
    fn a_single_repeat_claims_no_interval() {
        let merged = merge(vec![record(100.0)]);
        let bench = merged.bench.expect("bench section");
        let m = bench.wall_time_ms.expect("merge time survives");
        assert_eq!(m.n, 1);
        assert!(m.ci95.is_none());
    }
}
