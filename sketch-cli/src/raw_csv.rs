//! Per-run CSV emitter (long format), enabled by `sketchlib bench --raw-csv DIR`.
//!
//! Mirrors the historical headers under `throughput/<family>/output/`:
//! one row per measured run, with family-specific param columns the
//! existing `throughput/scripts/plot_*.py` scripts expect. Coexists with
//! the JSONL stream emitted via `--report`.
//!
//! HLL / KLL / DD additionally get a *per-call* query CSV
//! (`<family>_throughput_query_results_rust.csv` columns
//! `..call_index, nanoseconds, estimate` plus KLL/DD's
//! `repeat, percentile`) when `--accuracy` is also set — that
//! flag is what gates the comparator that owns the query phase
//! and now stashes per-call samples. Without `--accuracy`,
//! sketch-cli skips the query phase entirely and only the
//! insert CSV is written.
//!
//! Limitations:
//! - Impls outside the legacy harness (elastic, univmon, polars-,
//!   fastpath-parallel-) use a synthesised legacy-style name
//!   `rust_<impl>_<family>`.
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use anyhow::Result;
use sketch_bench::runner::BenchReport;
use sketch_bench::MetricsMask;
use sketch_core::config::ParamSet;

use crate::dispatch::ImplEntry;

pub fn write_runs(
    dir: &Path,
    entry: &ImplEntry,
    params: Option<&ParamSet>,
    seed: u64,
    workers: usize,
    report: &BenchReport,
) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    // Parallel ("octo") impls go to a separate combined file with
    // the legacy `sketch_type, implementation, num_workers,...`
    // header — `throughput/scripts/plot_octo_throughput.py` reads
    // exactly that shape from a single file across cms/cs/hll.
    if entry.impl_name == "lib-fastpath-parallel" {
        // Octo's CSV exists to track throughput across worker
        // counts; only emit from the THROUGHPUT pass so the
        // insert_wall_time_ns column reflects a clean hot path,
        // not the latency-pass timer overhead.
        if report.config.metrics.contains(MetricsMask::THROUGHPUT) {
            return write_octo_runs(dir, entry, workers, report);
        }
        return Ok(());
    }
    let legacy_impl = legacy_impl_name(entry.family, entry.impl_name);
    let param_cols = ParamCols::from(entry.family, params);

    // Only the THROUGHPUT pass produces a clean insert-phase wall
    // clock; rows from other passes (LATENCY pass is inflated by
    // per-op timing; ACCURACY pass is clean but conceptually
    // belongs to the query CSV) would muddle the throughput plot.
    if report.config.metrics.contains(MetricsMask::THROUGHPUT) {
        let insert_path = dir.join(format!("{}_throughput_results_rust.csv", entry.family));
        append_csv(
            &insert_path,
            &insert_header(entry.family),
            report.per_run.iter().enumerate().map(|(idx, run)| {
                format_insert_row(entry.family, &legacy_impl, &param_cols, seed, idx + 1, run)
            }),
        )?;
    }

    let has_per_call = report
        .per_run
        .iter()
        .any(|r| r.query_calls.as_ref().is_some_and(|v| !v.is_empty()));

    if has_per_call {
        // Per-call CSV — one row per (run, call). Matches the
        // legacy `throughput/{hll,kll,dd}/rust/src/bin/query.rs`
        // shape; takes precedence over the aggregate query CSV
        // for these three families because their plot scripts
        // read the per-call columns.
        let query_path = dir.join(format!(
            "{}_throughput_query_results_rust.csv",
            entry.family
        ));
        let header = per_call_query_header(entry.family);
        let mut rows: Vec<String> = Vec::new();
        for (run_idx, run) in report.per_run.iter().enumerate() {
            let run_no = run_idx + 1;
            if let Some(calls) = run.query_calls.as_ref() {
                for sample in calls {
                    rows.push(format_per_call_row(
                        entry.family,
                        &legacy_impl,
                        &param_cols,
                        run_no,
                        run.items_inserted,
                        sample,
                    ));
                }
            }
        }
        append_csv(&query_path, &header, rows.into_iter())?;

        // Also emit the aggregate (tight-loop) CSV: one row per run,
        // computed from `queries_executed` / `query_wall_time_ns`
        // (the comparator's outer Instant pair that wraps the whole
        // 101- or 4096-call tight loop). This is the apples-to-apples
        // peer of cpp-bench's `--query-csv` and lets the throughput
        // bar charts compare without per-call timer overhead.
        if report.per_run.iter().any(|r| r.queries_executed > 0) {
            let tight_path = dir.join(format!(
                "{}_throughput_query_tight_results_rust.csv",
                entry.family
            ));
            append_csv(
                &tight_path,
                &query_header(entry.family),
                report.per_run.iter().enumerate().map(|(idx, run)| {
                    format_query_row(entry.family, &legacy_impl, &param_cols, seed, idx + 1, run)
                }),
            )?;
        }
    } else if report.per_run.iter().any(|r| r.queries_executed > 0) {
        // Aggregate query CSV — CMS / CountSketch / Nitro style.
        let query_path = dir.join(format!(
            "{}_throughput_query_results_rust.csv",
            entry.family
        ));
        append_csv(
            &query_path,
            &query_header(entry.family),
            report.per_run.iter().enumerate().map(|(idx, run)| {
                format_query_row(entry.family, &legacy_impl, &param_cols, seed, idx + 1, run)
            }),
        )?;
    }
    Ok(())
}

/// Legacy octo CSV: one combined file, header
/// `sketch_type,implementation,num_workers,run,total_items,total_nanoseconds,throughput_items_per_sec`.
/// Used by `throughput/scripts/plot_octo_throughput.py`. We emit
/// `implementation = "octo"` (the legacy label for the parallel
/// path) regardless of the sketch-cli impl name; the `sketch_type`
/// column carries the family.
fn write_octo_runs(
    dir: &Path,
    entry: &ImplEntry,
    workers: usize,
    report: &BenchReport,
) -> Result<()> {
    let path = dir.join("octo_throughput_results_rust.csv");
    let header = "sketch_type,implementation,num_workers,run,total_items,total_nanoseconds,throughput_items_per_sec";
    let sketch_type = legacy_sketch_type(entry.family);
    let rows = report.per_run.iter().enumerate().map(|(idx, run)| {
        let total_items = run.items_inserted.max(1);
        let total_ns = run.insert_wall_time_ns.max(1);
        let throughput = (total_items as f64) * 1_000_000_000.0 / (total_ns as f64);
        format!(
            "{sketch_type},octo,{workers},{run_no},{total_items},{total_ns},{throughput:.6}",
            run_no = idx + 1,
        )
    });
    append_csv(&path, header, rows)
}

/// Map sketch-cli's family name to the legacy `sketch_type`
/// column value used by `plot_octo_throughput.py`.
fn legacy_sketch_type(family: &str) -> &'static str {
    match family {
        "countsketch" => "cs",
        "cms" => "cms",
        "hll" => "hll",
        _ => "unknown",
    }
}

fn per_call_query_header(family: &str) -> String {
    let lead = leading_label(family);
    let params = param_header(family);
    // Family-specific per-call tail; KLL/DD carry `repeat,
    // percentile` ahead of `call_index` to match the legacy
    // header order.
    let tail = match family {
        "hll" => "call_index,nanoseconds,estimate",
        "kll" | "dd" => "repeat,percentile,call_index,nanoseconds,estimate",
        _ => "call_index,nanoseconds,estimate",
    };
    let params_segment = if params.is_empty() {
        String::new()
    } else {
        format!(",{params}")
    };
    format!("implementation,language,{lead}{params_segment},total_items,{tail}")
}

fn format_per_call_row(
    family: &str,
    legacy_impl: &str,
    params: &ParamCols,
    run_no: usize,
    total_items: u64,
    sample: &sketch_bench::accuracy::QueryCallSample,
) -> String {
    // Per-call rows always index by `run` (legacy convention),
    // even for cms-style families that label aggregate rows with
    // `seed` — only HLL / KLL / DD reach this path and they all
    // use `run`.
    let lead = run_no.to_string();
    let params_str = params.join_values();
    let middle = if params_str.is_empty() {
        String::new()
    } else {
        format!(",{params_str}")
    };
    let tail = match family {
        "hll" => format!(
            "{},{},{}",
            sample.call_index,
            sample.nanoseconds,
            // HLL's legacy CSV writes estimate as `{:.0}` — it's
            // a count; surface as f64 unrounded here for
            // precision, downstream scripts cast back.
            sample.estimate,
        ),
        "kll" | "dd" => format!(
            "{},{},{},{},{}",
            sample.repeat,
            // Legacy KLL writes percentile as integer 0..100;
            // emit the same so the plot scripts' `int(...)` cast
            // stays valid. We round the fractional percentile we
            // captured (e.g. 0.50 → 50).
            (sample.percentile * 100.0).round() as i64,
            sample.call_index,
            sample.nanoseconds,
            sample.estimate,
        ),
        _ => format!(
            "{},{},{}",
            sample.call_index, sample.nanoseconds, sample.estimate
        ),
    };
    format!("{legacy_impl},rust,{lead}{middle},{total_items},{tail}")
}

fn append_csv<I: IntoIterator<Item = String>>(path: &Path, header: &str, rows: I) -> Result<()> {
    let new_file = !path.exists();
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    if new_file {
        writeln!(f, "{header}")?;
    }
    for row in rows {
        writeln!(f, "{row}")?;
    }
    Ok(())
}

/// Per-family construction-param columns extracted from a `ParamSet`.
/// Each family's columns are listed in the order they appear in the
/// CSV header (after the leading bookkeeping columns).
#[derive(Clone)]
struct ParamCols {
    cols: Vec<(&'static str, String)>,
}

impl ParamCols {
    fn from(family: &str, params: Option<&ParamSet>) -> Self {
        let mut cols: Vec<(&'static str, String)> = Vec::new();
        // Unparameterized impls (exact / polars) still need to
        // emit values for the family's param columns so each row
        // matches the legacy header width — plot scripts call
        // `csv.DictReader` and choke on a short row. Sentinel `0`
        // values mark "no sketch tuning involved here"; downstream
        // grouping is by `implementation` so the value isn't read
        // for these baselines.
        if params.is_none() {
            for name in legacy_param_columns(family) {
                cols.push((name, "0".to_string()));
            }
            return Self { cols };
        }
        match (family, params) {
            ("hll", Some(ParamSet::Hll(p))) => {
                cols.push(("lg_k", p.lg_k.to_string()));
                cols.push(("registers", (1usize << p.lg_k).to_string()));
            }
            ("kll", Some(ParamSet::Kll(p))) => {
                cols.push(("k", p.k.to_string()));
            }
            ("cms", Some(ParamSet::Cms(p))) => {
                cols.push(("rows", p.rows.to_string()));
                cols.push(("cols", p.cols.to_string()));
            }
            ("countsketch", Some(ParamSet::Countsketch(p))) => {
                cols.push(("rows", p.rows.to_string()));
                cols.push(("cols", p.cols.to_string()));
            }
            ("dd", Some(ParamSet::Dd(p))) => {
                cols.push(("alpha", format!("{:.6}", p.alpha)));
            }
            ("nitro", Some(ParamSet::Nitro(p))) => {
                // Nitro's legacy CSV carries rows/cols too, but the
                // sketch-cli ParamSet only owns the rate knob — the
                // rows/cols are baked into each impl. Emit 0 for them
                // and let the rate column carry the swept dimension.
                cols.push(("rows", "0".to_string()));
                cols.push(("cols", "0".to_string()));
                cols.push(("rate", format!("{:.6}", p.rate)));
            }
            ("elastic", Some(ParamSet::Elastic(p))) => {
                cols.push(("buckets", p.buckets.to_string()));
                cols.push(("depth", p.depth.to_string()));
            }
            ("univmon", Some(ParamSet::Univmon(p))) => {
                cols.push(("layers", p.layers.to_string()));
                cols.push(("max_stream", p.max_stream.to_string()));
            }
            _ => {}
        }
        Self { cols }
    }

    fn join_values(&self) -> String {
        self.cols
            .iter()
            .map(|(_, v)| v.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn legacy_impl_name(family: &str, impl_name: &str) -> String {
    if impl_name.starts_with("lib-") {
        let suffix = impl_name.trim_start_matches("lib-").replace('-', "_");
        format!("rust_sketchlib_{family}_{suffix}")
    } else if impl_name == "lib" {
        format!("rust_sketchlib_{family}")
    } else {
        format!("rust_{}_{}", impl_name.replace('-', "_"), family)
    }
}

fn insert_header(family: &str) -> String {
    let lead = leading_label(family);
    let params = param_header(family);
    if params.is_empty() {
        format!(
            "implementation,language,{lead},total_items,total_nanoseconds,throughput_items_per_sec,finalize_nanoseconds"
        )
    } else {
        format!(
            "implementation,language,{lead},{params},total_items,total_nanoseconds,throughput_items_per_sec,finalize_nanoseconds"
        )
    }
}

fn query_header(family: &str) -> String {
    let lead = leading_label(family);
    let params = param_header(family);
    if params.is_empty() {
        format!(
            "implementation,language,{lead},total_items,total_queries,total_nanoseconds,throughput_queries_per_sec"
        )
    } else {
        format!(
            "implementation,language,{lead},{params},total_items,total_queries,total_nanoseconds,throughput_queries_per_sec"
        )
    }
}

/// CMS / CountSketch index their legacy rows by `seed` (one row per
/// re-seeded run); the other families use a `run` ordinal. Match the
/// legacy header so plot scripts that look up `row["seed"]` /
/// `row["run"]` still parse.
fn leading_label(family: &str) -> &'static str {
    match family {
        "cms" | "countsketch" => "seed",
        _ => "run",
    }
}

fn param_header(family: &str) -> &'static str {
    match family {
        "hll" => "lg_k,registers",
        "kll" => "k",
        "cms" | "countsketch" => "rows,cols",
        "dd" => "alpha",
        "nitro" => "rows,cols,rate",
        "elastic" => "buckets,depth",
        "univmon" => "layers,max_stream",
        _ => "",
    }
}

/// The same set of columns as `param_header` but as separate
/// names, used to populate sentinel `0`s for unparameterized
/// impls so their CSV rows match the legacy width.
fn legacy_param_columns(family: &str) -> &'static [&'static str] {
    match family {
        "hll" => &["lg_k", "registers"],
        "kll" => &["k"],
        "cms" | "countsketch" => &["rows", "cols"],
        "dd" => &["alpha"],
        "nitro" => &["rows", "cols", "rate"],
        "elastic" => &["buckets", "depth"],
        "univmon" => &["layers", "max_stream"],
        _ => &[],
    }
}

fn format_insert_row(
    family: &str,
    legacy_impl: &str,
    params: &ParamCols,
    seed: u64,
    run_idx: usize,
    run: &sketch_bench::metrics::RunMetrics,
) -> String {
    let lead = leading_value(family, seed, run_idx);
    let total_items = run.items_inserted.max(1);
    let total_ns = run.insert_wall_time_ns.max(1);
    let throughput = (total_items as f64) * 1_000_000_000.0 / (total_ns as f64);
    let params_str = params.join_values();
    let middle = if params_str.is_empty() {
        String::new()
    } else {
        format!(",{params_str}")
    };
    let finalize_ns = run.finalize_wall_time_ns;
    format!(
        "{legacy_impl},rust,{lead}{middle},{total_items},{total_ns},{throughput:.6},{finalize_ns}"
    )
}

fn format_query_row(
    family: &str,
    legacy_impl: &str,
    params: &ParamCols,
    seed: u64,
    run_idx: usize,
    run: &sketch_bench::metrics::RunMetrics,
) -> String {
    let lead = leading_value(family, seed, run_idx);
    let total_items = run.items_inserted;
    let total_queries = run.queries_executed.max(1);
    let total_ns = run.query_wall_time_ns.max(1);
    let throughput = (total_queries as f64) * 1_000_000_000.0 / (total_ns as f64);
    let params_str = params.join_values();
    let middle = if params_str.is_empty() {
        String::new()
    } else {
        format!(",{params_str}")
    };
    format!(
        "{legacy_impl},rust,{lead}{middle},{total_items},{total_queries},{total_ns},{throughput:.6}"
    )
}

fn leading_value(family: &str, seed: u64, run_idx: usize) -> String {
    match family {
        "cms" | "countsketch" => seed.to_string(),
        _ => run_idx.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_impl_names_match_history() {
        assert_eq!(legacy_impl_name("hll", "oxide"), "rust_oxide_hll");
        assert_eq!(
            legacy_impl_name("hll", "datasketches"),
            "rust_datasketches_hll"
        );
        assert_eq!(legacy_impl_name("hll", "lib"), "rust_sketchlib_hll");
        assert_eq!(
            legacy_impl_name("cms", "lib-fixedmatrix-custom-fast"),
            "rust_sketchlib_cms_fixedmatrix_custom_fast"
        );
        assert_eq!(legacy_impl_name("nitro", "lib"), "rust_sketchlib_nitro");
    }

    #[test]
    fn hll_header_matches_legacy() {
        assert_eq!(
            insert_header("hll"),
            "implementation,language,run,lg_k,registers,total_items,total_nanoseconds,throughput_items_per_sec,finalize_nanoseconds"
        );
    }

    #[test]
    fn cms_header_uses_seed_label() {
        assert_eq!(
            insert_header("cms"),
            "implementation,language,seed,rows,cols,total_items,total_nanoseconds,throughput_items_per_sec,finalize_nanoseconds"
        );
    }

    #[test]
    fn kll_query_header_aggregate_shape() {
        assert_eq!(
            query_header("kll"),
            "implementation,language,run,k,total_items,total_queries,total_nanoseconds,throughput_queries_per_sec"
        );
    }
}
