//! Legacy long-format CSV rendering for a [`BenchReport`].
//!
//! Produces the exact headers and filenames the historical
//! `throughput/scripts/plot_*.py` scripts read with `csv.DictReader`: one row
//! per measured run, with family-specific param columns. Coexists with the
//! JSONL stream emitted via `--report`.
//!
//! This is the *content*, not the sink: [`render`] returns a [`CsvFile`] per
//! output file (name, header, rows), and the frontend appends each to the
//! `--raw-csv DIR` it chose (see `aqpbm-cli`'s `raw_csv`). Same split as the
//! `Record`/`to_jsonl` (content) + `ReportSink` (write) pair for JSONL.
//!
//! HLL / KLL / DD additionally get a *per-call* query CSV
//! (`<family>_throughput_query_results_rust.csv` columns
//! `..call_index, nanoseconds, estimate` plus KLL/DD's `repeat, percentile`)
//! when `--accuracy` is also set — that flag is what gates the comparator that
//! owns the query phase and stashes per-call samples. Without `--accuracy` the
//! query phase is skipped entirely and only the insert CSV is produced.
//!
//! Limitations:
//! - Impls outside the legacy harness (elastic, univmon, polars-,
//!   fastpath-parallel-) use a synthesised legacy-style name
//!   `rust_<impl>_<family>`.

use aqpbm_datagen::DType;

use crate::params::ParamSet;
use crate::runner::BenchReport;
use crate::MetricsMask;

/// One CSV file to write: its filename (relative to the `--raw-csv` dir), its
/// header line, and the rows beneath it. The frontend creates the file if
/// missing, writes the header once, and appends the rows.
pub struct CsvFile {
    pub name: String,
    pub header: String,
    pub rows: Vec<String>,
}

/// Render the legacy CSV files this `(entry, report)` produces.
///
/// Returns one [`CsvFile`] per file the run should append to — empty when this
/// report carries no pass that maps to a legacy CSV (e.g. a lone LATENCY pass,
/// whose per-op timer would muddle the throughput plot).
pub fn render(
    family: &str,
    impl_name: &str,
    params: Option<&ParamSet>,
    seed: u64,
    workers: usize,
    dtype: DType,
    report: &BenchReport,
) -> Vec<CsvFile> {
    let mut out = Vec::new();
    // Non-`i64` runs go to their own files rather than into the shared ones.
    //
    // These CSV headers are an external contract — `throughput/scripts/*.py`
    // read the exact column list, and the frontend appends, so a run lands in
    // whatever file already exists. Adding a `dtype` column would change the
    // header for every existing consumer, including pure-`i64` users who did
    // not ask for the axis; leaving it out would let an `f64` run interleave
    // with `i64` rows that are indistinguishable from it, which is the exact
    // pooling `WorkloadDesc::dtype` exists to prevent. A separate file breaks
    // neither contract.
    let fam = family_file_stem(family, dtype);

    // Parallel ("octo") impls go to a separate combined file with the legacy
    // `sketch_type, implementation, num_workers,...` header —
    // `throughput/scripts/plot_octo_throughput.py` reads exactly that shape
    // from a single file across cms/cs/hll. Only the THROUGHPUT pass emits, so
    // the insert_wall_time_ns column reflects a clean hot path, not the
    // latency-pass timer overhead.
    if impl_name == "lib-fastpath-parallel" {
        if report.config.metrics.contains(MetricsMask::THROUGHPUT) {
            out.push(octo_file(family, workers, report));
        }
        return out;
    }

    let legacy_impl = legacy_impl_name(family, impl_name);
    let param_cols = ParamCols::from(family, params);

    // Only the THROUGHPUT pass produces a clean insert-phase wall clock; rows
    // from other passes (LATENCY is inflated by per-op timing; ACCURACY is
    // clean but conceptually belongs to the query CSV) would muddle the
    // throughput plot.
    if report.config.metrics.contains(MetricsMask::THROUGHPUT) {
        out.push(CsvFile {
            name: format!("{fam}_throughput_results_rust.csv"),
            header: insert_header(family),
            rows: report
                .per_run
                .iter()
                .enumerate()
                .map(|(idx, run)| {
                    format_insert_row(family, &legacy_impl, &param_cols, seed, idx + 1, run)
                })
                .collect(),
        });
    }

    let has_per_call = report
        .per_run
        .iter()
        .any(|r| r.query_calls.as_ref().is_some_and(|v| !v.is_empty()));

    if has_per_call {
        // Per-call CSV — one row per (run, call). Matches the legacy
        // `throughput/{hll,kll,dd}/rust/src/bin/query.rs` shape; takes
        // precedence over the aggregate query CSV for these three families
        // because their plot scripts read the per-call columns.
        let mut rows: Vec<String> = Vec::new();
        for (run_idx, run) in report.per_run.iter().enumerate() {
            let run_no = run_idx + 1;
            if let Some(calls) = run.query_calls.as_ref() {
                for sample in calls {
                    rows.push(format_per_call_row(
                        family,
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
            name: format!("{fam}_throughput_query_results_rust.csv"),
            header: per_call_query_header(family),
            rows,
        });

        // Also emit the aggregate (tight-loop) CSV: one row per run, computed
        // from `queries_executed` / `query_wall_time_ns` (the comparator's
        // outer Instant pair that wraps the whole 101- or 4096-call tight
        // loop). This is the apples-to-apples peer of cpp-bench's
        // `--query-csv` and lets the throughput bar charts compare without
        // per-call timer overhead.
        if report.per_run.iter().any(|r| r.queries_executed > 0) {
            out.push(CsvFile {
                name: format!("{fam}_throughput_query_tight_results_rust.csv"),
                header: query_header(family),
                rows: report
                    .per_run
                    .iter()
                    .enumerate()
                    .map(|(idx, run)| {
                        format_query_row(
                            family,
                            &legacy_impl,
                            &param_cols,
                            seed,
                            idx + 1,
                            run,
                        )
                    })
                    .collect(),
            });
        }
    } else if report.per_run.iter().any(|r| r.queries_executed > 0) {
        // Aggregate query CSV — CMS / CountSketch / Nitro style.
        out.push(CsvFile {
            name: format!("{fam}_throughput_query_results_rust.csv"),
            header: query_header(family),
            rows: report
                .per_run
                .iter()
                .enumerate()
                .map(|(idx, run)| {
                    format_query_row(family, &legacy_impl, &param_cols, seed, idx + 1, run)
                })
                .collect(),
        });
    }
    out
}

/// Legacy octo CSV: one combined file, header
/// `sketch_type,implementation,num_workers,run,total_items,total_nanoseconds,throughput_items_per_sec`.
/// Used by `throughput/scripts/plot_octo_throughput.py`. We emit
/// `implementation = "octo"` (the legacy label for the parallel path)
/// regardless of the impl name; the `sketch_type` column carries the family.
fn octo_file(family: &str, workers: usize, report: &BenchReport) -> CsvFile {
    let sketch_type = legacy_sketch_type(family);
    let rows = report
        .per_run
        .iter()
        .enumerate()
        .map(|(idx, run)| {
            let total_items = run.items_inserted.max(1);
            let total_ns = run.insert_wall_time_ns.max(1);
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

/// Map a family name to the legacy `sketch_type` column value used by
/// `plot_octo_throughput.py`.
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
    // Family-specific per-call tail; KLL/DD carry `repeat, percentile` ahead
    // of `call_index` to match the legacy header order.
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
    sample: &crate::accuracy::QueryCallSample,
) -> String {
    // Per-call rows always index by `run` (legacy convention), even for
    // cms-style families that label aggregate rows with `seed` — only HLL /
    // KLL / DD reach this path and they all use `run`.
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

/// Per-family construction-param columns extracted from a `ParamSet`.
/// Each family's columns are listed in the order they appear in the CSV header
/// (after the leading bookkeeping columns).
#[derive(Clone)]
struct ParamCols {
    cols: Vec<(&'static str, String)>,
}

impl ParamCols {
    fn from(family: &str, params: Option<&ParamSet>) -> Self {
        let mut cols: Vec<(&'static str, String)> = Vec::new();
        // Unparameterized impls (polars) still need to emit values for the
        // family's param columns so each row matches the legacy header width —
        // plot scripts call `csv.DictReader` and choke on a short row.
        // Sentinel `0` values mark "no sketch tuning involved here";
        // downstream grouping is by `implementation` so the value isn't read
        // for these baselines.
        if params.is_none() {
            for name in legacy_param_columns(family) {
                cols.push((name, "0".to_string()));
            }
            return Self { cols };
        }
        // Values come from the params object generically; only the two places
        // where the legacy CSV header is *not* a list of parameters need
        // naming. The rest used to be one match arm per family, kept in step
        // with `param_header` by hand.
        let params = params.expect("None handled above");
        match family {
            // `registers` is derived from `lg_k`, not a parameter.
            "hll" => {
                let lg_k = params.fields().into_iter().find(|(k, _)| k == "lg_k");
                if let Some((_, v)) = lg_k {
                    let bits: u32 = v.parse().unwrap_or(0);
                    cols.push(("lg_k", v));
                    cols.push(("registers", (1usize << bits).to_string()));
                }
            }
            // Nitro's legacy CSV carries rows/cols, but the params only own
            // `rate` — the matrix shape is baked into each impl. Sentinel 0s
            // keep the row width legal.
            "nitro" => {
                cols.push(("rows", "0".to_string()));
                cols.push(("cols", "0".to_string()));
                for (_, v) in params.fields() {
                    cols.push(("rate", legacy_float_format(&v)));
                }
            }
            _ => {
                // Iterate the **header**, looking each column's value up — not
                // the params object, which is ordered alphabetically. Driving
                // the loop from `fields()` emitted cms as `2048,5` under a
                // header reading `rows,cols`: a silent column/value swap in a
                // file that plot scripts read positionally.
                let fields = params.fields();
                for col in legacy_param_columns(family) {
                    if let Some((_, v)) = fields.iter().find(|(k, _)| k == col) {
                        cols.push((col, legacy_float_format(v)));
                    }
                }
            }
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

/// File stem for a family's CSVs. `i64` keeps the historical name so existing
/// files keep accumulating and existing scripts keep resolving; anything else
/// is suffixed.
fn family_file_stem(family: &str, dtype: DType) -> String {
    if dtype.is_i64() {
        family.to_string()
    } else {
        format!("{family}_{}", dtype.as_str())
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

/// CMS / CountSketch index their legacy rows by `seed` (one row per re-seeded
/// run); the other families use a `run` ordinal. Match the legacy header so
/// plot scripts that look up `row["seed"]` / `row["run"]` still parse.
fn leading_label(family: &str) -> &'static str {
    match family {
        "cms" | "countsketch" => "seed",
        _ => "run",
    }
}

/// Render a value the way the pre-generic writer did.
///
/// `--raw-csv` **appends**, so a formatting change splits one series in two:
/// `alpha = 0.01` written as `0.010000` by earlier runs and `0.01` by later
/// ones makes `groupby(row["alpha"])` produce two points with half the samples
/// each. Integers were always rendered plainly; floats always with six
/// decimals, and they still are.
fn legacy_float_format(v: &str) -> String {
    match v.parse::<f64>() {
        Ok(f) if v.contains('.') || v.contains('e') || v.contains('E') => format!("{f:.6}"),
        _ => v.to_string(),
    }
}

/// The legacy CSV header, verbatim.
///
/// This stays a per-family table on purpose: it encodes an **external file
/// format** that plot scripts read with `csv.DictReader`, not an abstraction
/// over sketch families. Deriving it from the params object would silently
/// change the header — `registers` is derived rather than a parameter, and
/// nitro's `rows`/`cols` are sentinels — and break those readers. The values
/// beneath it are produced generically; only the column names are pinned.
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

/// The same set of columns as `param_header` but as separate names, used to
/// populate sentinel `0`s for unparameterized impls so their CSV rows match
/// the legacy width.
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
    run: &aqpbm_core::metrics::RunMetrics,
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
    run: &aqpbm_core::metrics::RunMetrics,
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

#[cfg(test)]
mod param_column_order_tests {
    use super::*;
    use crate::params::{CmsParams, ElasticParams, HllParams, UnivMonParams};

    /// Values must line up with the header, which is *not* alphabetical.
    ///
    /// The generic value path once iterated the params object — ordered by
    /// key — while the header stayed in its legacy order, so cms wrote
    /// `2048,5` under `rows,cols`. The CSV is read positionally, so nothing
    /// downstream could have noticed.
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
        for (family, params, expected) in cases {
            let cols = ParamCols::from(family, Some(&params));
            let names: Vec<&str> = cols.cols.iter().map(|(n, _)| *n).collect();
            let header: Vec<&str> = param_header(family).split(',').collect();
            assert_eq!(names, header, "{family}: column order must match header");
            let values: Vec<&str> = cols.cols.iter().map(|(_, v)| v.as_str()).collect();
            assert_eq!(values, expected, "{family}: values misaligned");
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
