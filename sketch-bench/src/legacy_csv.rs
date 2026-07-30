//! Long-format CSV rendering for a [`BenchReport`] — the exact headers and
//! filenames `plot_*.py` reads with `csv.DictReader`, one row per measured run.
//! Content only: [`render`] returns a [`CsvFile`] per file and the frontend
//! appends each, mirroring the `Record` / `ReportSink` split for JSONL.
//! HLL / KLL / DD also get a per-call query CSV when `--accuracy` is set.

use crate::params::ParamSet;
use aqpbm_core::metrics::MetricsMask;
use aqpbm_core::runner::BenchReport;

/// One CSV file to write: its filename (relative to the `--raw-csv` dir), its
/// header line, and the rows beneath it. The frontend creates the file if
/// missing, writes the header once, and appends the rows.
pub struct CsvFile {
    pub name: String,
    pub header: String,
    pub rows: Vec<String>,
}

/// Render the CSV files this `(entry, report)` produces — one [`CsvFile`] per
/// file to append to, empty when no pass maps to one (a lone LATENCY pass,
/// whose per-op timer would muddle the throughput plot).
pub fn render(
    algorithm: &str,
    impl_name: &str,
    params: Option<&ParamSet>,
    seed: u64,
    workers: usize,
    report: &BenchReport,
) -> Vec<CsvFile> {
    let mut out = Vec::new();
    // These CSV headers are an external contract — `throughput/scripts/*.py`
    // read the exact column list, and the frontend appends, so a run lands in
    // whatever file already exists.
    let algo = algorithm.to_string();

    // Parallel ("octo") impls go to a separate combined file whose
    // `sketch_type, implementation, num_workers,...` header spans cms/cs/hll.
    // Only THROUGHPUT emits, so the wall column reflects a clean hot path.
    if impl_name == "lib-fastpath-parallel" {
        if report.config.metrics.contains(MetricsMask::THROUGHPUT) {
            out.push(octo_file(algorithm, workers, report));
        }
        return out;
    }

    let legacy_impl = legacy_impl_name(algorithm, impl_name);
    let param_cols = ParamCols::from(algorithm, params);

    // Only THROUGHPUT produces a clean insert-phase wall clock: LATENCY is
    // inflated by per-op timing, and ACCURACY belongs to the query CSV.
    if report.config.metrics.contains(MetricsMask::THROUGHPUT) {
        out.push(CsvFile {
            name: format!("{algo}_throughput_results_rust.csv"),
            header: insert_header(algorithm),
            rows: report
                .per_run
                .iter()
                .enumerate()
                .map(|(idx, run)| {
                    format_insert_row(algorithm, &legacy_impl, &param_cols, seed, idx + 1, run)
                })
                .collect(),
        });
    }

    let has_per_call = report
        .per_run
        .iter()
        .any(|r| r.query_calls.as_ref().is_some_and(|v| !v.is_empty()));

    if has_per_call {
        // Per-call CSV — one row per (run, call). Takes precedence over the
        // aggregate query CSV for these three algorithms, whose plot scripts
        // read the per-call columns.
        let mut rows: Vec<String> = Vec::new();
        for (run_idx, run) in report.per_run.iter().enumerate() {
            let run_no = run_idx + 1;
            if let Some(calls) = run.query_calls.as_ref() {
                for sample in calls {
                    rows.push(format_per_call_row(
                        algorithm,
                        &legacy_impl,
                        &param_cols,
                        run_no,
                        run.items_inserted,
                        sample,
                    ));
                }
            }
        }
        out.push(CsvFile {
            name: format!("{algo}_throughput_query_results_rust.csv"),
            header: per_call_query_header(algorithm),
            rows,
        });

        // Also emit the aggregate (tight-loop) CSV: one row per run from
        // `queries_executed` / `query_wall_time_ns`, so the bar charts can
        // compare without per-call timer overhead.
        if report.per_run.iter().any(|r| r.queries_executed > 0) {
            out.push(CsvFile {
                name: format!("{algo}_throughput_query_tight_results_rust.csv"),
                header: query_header(algorithm),
                rows: report
                    .per_run
                    .iter()
                    .enumerate()
                    .map(|(idx, run)| {
                        format_query_row(algorithm, &legacy_impl, &param_cols, seed, idx + 1, run)
                    })
                    .collect(),
            });
        }
    } else if report.per_run.iter().any(|r| r.queries_executed > 0) {
        // Aggregate query CSV — CMS / CountSketch / Nitro style.
        out.push(CsvFile {
            name: format!("{algo}_throughput_query_results_rust.csv"),
            header: query_header(algorithm),
            rows: report
                .per_run
                .iter()
                .enumerate()
                .map(|(idx, run)| {
                    format_query_row(algorithm, &legacy_impl, &param_cols, seed, idx + 1, run)
                })
                .collect(),
        });
    }
    out
}

/// The octo CSV: one combined file, `implementation = "octo"` with `sketch_type`
/// carrying the algorithm. `total_nanoseconds` is the **build** wall, not the insert
/// wall — these rows do their real work in finalize.
fn octo_file(algorithm: &str, workers: usize, report: &BenchReport) -> CsvFile {
    let sketch_type = legacy_sketch_type(algorithm);
    let rows = report
        .per_run
        .iter()
        .enumerate()
        .map(|(idx, run)| {
            let total_items = run.items_inserted.max(1);
            let total_ns = run.build_wall_time_ns().max(1);
            let throughput = (total_items as f64) * 1_000_000_000.0 / (total_ns as f64);
            format!(
                "{sketch_type},octo,{workers},{run_no},{total_items},{total_ns},{throughput:.6}",
                run_no = idx + 1,
            )
        })
        .collect();
    CsvFile {
        name: "octo_throughput_results_rust.csv".to_string(),
        header: "sketch_type,implementation,num_workers,run,total_items,total_nanoseconds,throughput_items_per_sec".to_string(),
        rows,
    }
}

/// Map an algorithm name to the legacy `sketch_type` column value used by
/// `plot_octo_throughput.py`.
fn legacy_sketch_type(algorithm: &str) -> &'static str {
    match algorithm {
        "countsketch" => "cs",
        "cms" => "cms",
        "hll" => "hll",
        _ => "unknown",
    }
}

fn per_call_query_header(algorithm: &str) -> String {
    let lead = leading_label(algorithm);
    let params = param_header(algorithm);
    // Algorithm-specific per-call tail; KLL/DD carry `repeat, percentile` ahead
    // of `call_index` to match the legacy header order.
    let tail = match algorithm {
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
    algorithm: &str,
    legacy_impl: &str,
    params: &ParamCols,
    run_no: usize,
    total_items: u64,
    sample: &aqpbm_core::metrics::QueryCallSample,
) -> String {
    // Per-call rows always index by `run` (legacy convention), even for
    // cms-style algorithms that label aggregate rows with `seed` — only HLL /
    // KLL / DD reach this path and they all use `run`.
    let lead = run_no.to_string();
    let params_str = params.join_values();
    let middle = if params_str.is_empty() {
        String::new()
    } else {
        format!(",{params_str}")
    };
    let tail = match algorithm {
        "hll" => format!(
            "{},{},{}",
            sample.call_index,
            sample.nanoseconds,
            // HLL's legacy CSV writes estimate as `{:.0}` — it's a count;
            // surface as f64 unrounded here for precision, downstream scripts
            // cast back.
            sample.estimate,
        ),
        "kll" | "dd" => format!(
            "{},{},{},{},{}",
            sample.repeat,
            // Legacy KLL writes percentile as integer 0..100; emit the same so
            // the plot scripts' `int(...)` cast stays valid. We round the
            // fractional percentile we captured (e.g. 0.50 → 50).
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

/// Per-algorithm construction-param columns extracted from a `ParamSet`.
/// Each algorithm's columns are listed in the order they appear in the CSV header
/// (after the leading bookkeeping columns).
#[derive(Clone)]
struct ParamCols {
    cols: Vec<(&'static str, String)>,
}

impl ParamCols {
    /// Build one row's parameter values, **always** exactly as many as
    /// `legacy_param_columns(algorithm)` names — a short row silently shifts every
    /// later column. A final pass reconciles against the header, filling with `0`.
    fn from(algorithm: &str, params: Option<&ParamSet>) -> Self {
        let mut found: Vec<(&'static str, String)> = Vec::new();
        // Values come from the params object generically; only the two places
        // where the header is *not* a list of parameters need naming.
        if let Some(params) = params {
            match algorithm {
                // `registers` is derived from `lg_k`, not a parameter.
                "hll" => {
                    let lg_k = params.fields().into_iter().find(|(k, _)| k == "lg_k");
                    if let Some((_, v)) = lg_k {
                        let bits: u32 = v.parse().unwrap_or(0);
                        found.push(("lg_k", v));
                        found.push(("registers", (1usize << bits).to_string()));
                    }
                }
                // Nitro's legacy CSV carries rows/cols, but the params only own
                // `rate` — the matrix shape is baked into each impl, so those
                // two fall through to the sentinel below.
                "nitro" => {
                    for (_, v) in params.fields() {
                        found.push(("rate", legacy_float_format(&v)));
                    }
                }
                _ => {
                    // Iterate the **header**, looking each column's value up —
                    // not the params object, which is alphabetical. Driving from
                    // `fields()` swaps cms to `2048,5` under `rows,cols`.
                    let fields = params.fields();
                    for col in legacy_param_columns(algorithm) {
                        if let Some((_, v)) = fields.iter().find(|(k, _)| k == col) {
                            found.push((col, legacy_float_format(v)));
                        }
                    }
                }
            }
        }
        let cols = legacy_param_columns(algorithm)
            .iter()
            .map(|&col| {
                let v = found
                    .iter()
                    .find(|(k, _)| *k == col)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_else(|| "0".to_string());
                (col, v)
            })
            .collect();
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

fn legacy_impl_name(algorithm: &str, impl_name: &str) -> String {
    if impl_name.starts_with("lib-") {
        let suffix = impl_name.trim_start_matches("lib-").replace('-', "_");
        format!("rust_sketchlib_{algorithm}_{suffix}")
    } else if impl_name == "lib" {
        format!("rust_sketchlib_{algorithm}")
    } else {
        format!("rust_{}_{}", impl_name.replace('-', "_"), algorithm)
    }
}

/// File stem for an algorithm's CSVs. `i64` keeps the historical name so existing
/// files keep accumulating and existing scripts keep resolving; anything else
/// is suffixed.

fn insert_header(algorithm: &str) -> String {
    let lead = leading_label(algorithm);
    let params = param_header(algorithm);
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

fn query_header(algorithm: &str) -> String {
    let lead = leading_label(algorithm);
    let params = param_header(algorithm);
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

/// CMS / CountSketch index their legacy rows by `seed` (one row per re-seeded
/// run); the other algorithms use a `run` ordinal. Match the legacy header so
/// plot scripts that look up `row["seed"]` / `row["run"]` still parse.
fn leading_label(algorithm: &str) -> &'static str {
    match algorithm {
        "cms" | "countsketch" => "seed",
        _ => "run",
    }
}

/// Render a value in the pinned CSV formatting: integers plainly, floats with
/// six decimals. `--raw-csv` **appends**, so changing this splits one series in
/// two — `0.01` and `0.010000` group as different keys.
fn legacy_float_format(v: &str) -> String {
    match v.parse::<f64>() {
        Ok(f) if v.contains('.') || v.contains('e') || v.contains('E') => format!("{f:.6}"),
        _ => v.to_string(),
    }
}

/// The CSV header, verbatim. A per-algorithm table on purpose: it encodes an
/// **external file format**, not an abstraction over algorithms — `registers` is
/// derived and nitro's `rows`/`cols` are sentinels, so deriving it would break.
fn param_header(algorithm: &str) -> &'static str {
    match algorithm {
        "hll" => "lg_k,registers",
        "kll" => "k",
        "cms" | "countsketch" => "rows,cols",
        // Same matrix shape as cms/countsketch plus the tracked-key count; `k`
        // is the axis a topk sweep varies, so omitting it pooled every k into
        // one group.
        "topk" => "rows,cols,k",
        "dd" => "alpha",
        "nitro" => "rows,cols,rate",
        "elastic" => "buckets,depth",
        "univmon" => "layers,max_stream",
        // Must stay in step with `legacy_param_columns`; the width assertion in
        // `every_row_has_one_value_per_header_column` is what holds the two lists
        // together.
        "hydra-cms" => "rows,cols,cell_rows,cell_cols",
        _ => "",
    }
}

/// The same set of columns as `param_header` but as separate names, used to
/// populate sentinel `0`s for unparameterized impls so their CSV rows match
/// the legacy width.
fn legacy_param_columns(algorithm: &str) -> &'static [&'static str] {
    match algorithm {
        "hll" => &["lg_k", "registers"],
        "kll" => &["k"],
        "cms" | "countsketch" => &["rows", "cols"],
        "topk" => &["rows", "cols", "k"],
        "dd" => &["alpha"],
        "nitro" => &["rows", "cols", "rate"],
        "elastic" => &["buckets", "depth"],
        "univmon" => &["layers", "max_stream"],
        // Both shapes, because Hydra's cost is their product: a row carrying
        // only the grid would read as a far smaller sketch than it is.
        "hydra-cms" => &["rows", "cols", "cell_rows", "cell_cols"],
        _ => &[],
    }
}

fn format_insert_row(
    algorithm: &str,
    legacy_impl: &str,
    params: &ParamCols,
    seed: u64,
    run_idx: usize,
    run: &aqpbm_core::metrics::RunMetrics,
) -> String {
    let lead = leading_value(algorithm, seed, run_idx);
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
    algorithm: &str,
    legacy_impl: &str,
    params: &ParamCols,
    seed: u64,
    run_idx: usize,
    run: &aqpbm_core::metrics::RunMetrics,
) -> String {
    let lead = leading_value(algorithm, seed, run_idx);
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

fn leading_value(algorithm: &str, seed: u64, run_idx: usize) -> String {
    match algorithm {
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

    /// topk keeps the `run` label but carries three param columns; a missing
    /// `k` made runs at different k byte-identical apart from timing noise.
    #[test]
    fn topk_header_carries_its_param_columns() {
        assert_eq!(
            insert_header("topk"),
            "implementation,language,run,rows,cols,k,total_items,total_nanoseconds,throughput_items_per_sec,finalize_nanoseconds"
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

#[cfg(test)]
mod param_column_order_tests {
    use super::*;
    use crate::params::{CmsParams, ElasticParams, HllParams, TopkParams, UnivMonParams};

    /// Values must line up with the header, which is *not* alphabetical: driving
    /// the loop from the key-ordered params object writes cms as `2048,5` under
    /// `rows,cols`, and a positionally-read CSV cannot notice.
    #[test]
    fn values_follow_the_header_not_the_key_order() {
        let cases: Vec<(&str, ParamSet, Vec<&str>)> = vec![
            (
                "cms",
                ParamSet::of(&CmsParams {
                    rows: 5,
                    cols: 2048,
                }),
                vec!["5", "2048"],
            ),
            (
                "countsketch",
                ParamSet::of(&crate::params::CountSketchParams {
                    rows: 3,
                    cols: 4096,
                }),
                vec!["3", "4096"],
            ),
            (
                // Alphabetical `fields()` order here is `cols,k,rows` — the
                // widest gap yet between key order and header order.
                "topk",
                ParamSet::of(&TopkParams {
                    rows: 5,
                    cols: 2048,
                    k: 100,
                }),
                vec!["5", "2048", "100"],
            ),
            (
                "elastic",
                ParamSet::of(&ElasticParams {
                    buckets: 1024,
                    depth: 3,
                }),
                vec!["1024", "3"],
            ),
            (
                "univmon",
                ParamSet::of(&UnivMonParams {
                    layers: 8,
                    max_stream: 256,
                }),
                vec!["8", "256"],
            ),
        ];
        for (algorithm, params, expected) in cases {
            let cols = ParamCols::from(algorithm, Some(&params));
            let names: Vec<&str> = cols.cols.iter().map(|(n, _)| *n).collect();
            let header: Vec<&str> = param_header(algorithm).split(',').collect();
            assert_eq!(names, header, "{algorithm}: column order must match header");
            let values: Vec<&str> = cols.cols.iter().map(|(_, v)| v.as_str()).collect();
            assert_eq!(values, expected, "{algorithm}: values misaligned");
        }
    }

    /// Every algorithm, every params object, one value per header column. Silent by
    /// construction: `csv.DictReader` zips positionally and pads the tail, so a
    /// short row shifts every later column and raises nothing.
    #[test]
    fn every_row_has_one_value_per_header_column() {
        let empty = ParamSet {
            algorithm: String::new(),
            params: serde_json::json!({}),
        };
        let wrong_keys = ParamSet {
            algorithm: String::new(),
            params: serde_json::json!({ "nonsense": 1 }),
        };
        for algorithm in [
            "hll",
            "kll",
            "cms",
            "countsketch",
            "topk",
            "dd",
            "nitro",
            "elastic",
            "univmon",
            "hydra-cms",
        ] {
            let width = param_header(algorithm).split(',').count();
            for (label, params) in [
                ("none", None),
                ("empty", Some(&empty)),
                ("wrong keys", Some(&wrong_keys)),
            ] {
                let cols = ParamCols::from(algorithm, params);
                assert_eq!(
                    cols.join_values().split(',').count(),
                    width,
                    "{algorithm} with {label} params: row width must match header"
                );
                let names: Vec<&str> = cols.cols.iter().map(|(n, _)| *n).collect();
                let header: Vec<&str> = param_header(algorithm).split(',').collect();
                assert_eq!(names, header, "{algorithm} with {label} params");
            }
        }
    }

    /// `registers` is derived from `lg_k`, not a parameter — the one place the
    /// header is not simply a list of fields.
    #[test]
    fn hll_still_emits_the_derived_register_count() {
        let cols = ParamCols::from("hll", Some(&ParamSet::of(&HllParams { lg_k: 14 })));
        assert_eq!(
            cols.cols,
            vec![
                ("lg_k", "14".to_string()),
                ("registers", "16384".to_string())
            ]
        );
    }
}
