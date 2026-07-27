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
//! Naming: `lib` / `lib-*` impls keep their historical
//! `rust_sketchlib_<family>[_<variant>]` names; every impl with no legacy
//! counterpart (`oxide`, `datasketches`, `polars`, the topk trackers) gets a
//! synthesised `rust_<impl>_<family>`. The parallel-insert rows never reach
//! that path — they go to the octo file, labelled `octo`.

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
    report: &BenchReport,
) -> Vec<CsvFile> {
    let mut out = Vec::new();
    // These CSV headers are an external contract — `throughput/scripts/*.py`
    // read the exact column list, and the frontend appends, so a run lands in
    // whatever file already exists.
    let fam = family.to_string();

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
                        format_query_row(family, &legacy_impl, &param_cols, seed, idx + 1, run)
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
///
/// `total_nanoseconds` is the **build** wall — insert plus
/// `prepare` — not the insert wall the other legacy CSVs use.
/// The parallel rows buffer their partition in `update` and run the whole
/// parallel section in finalize (see `wrappers::parallel`), so dividing by
/// the insert wall alone would publish the cost of a `Vec::push` under a
/// header that legacy octo filled with the parallel insert time, and the
/// resulting plot would show these rows beating every sketch by two orders
/// of magnitude. The column shape is unchanged; only the row is correct now.
fn octo_file(family: &str, workers: usize, report: &BenchReport) -> CsvFile {
    let sketch_type = legacy_sketch_type(family);
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
    sample: &aqpbm_core::metrics::QueryCallSample,
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
    /// Build one row's worth of parameter values, **always** exactly as many
    /// as `legacy_param_columns(family)` names.
    ///
    /// That guarantee is the whole job. Plot scripts read these files with
    /// `csv.DictReader`, which pairs fields with header names positionally
    /// and does not notice a short row — it just shifts every later column
    /// left and reads the last one as `None`. A `kll/polars` row written
    /// without `--config` used to do exactly that: no value for `k`, so
    /// `total_items` landed under `k`, and the plot read
    /// `finalize_nanoseconds` as `throughput_items_per_sec`.
    ///
    /// So the branches below produce whatever they can, and a final pass
    /// reconciles the result against the header, filling anything missing
    /// with the sentinel `0` — "no sketch tuning involved here". Downstream
    /// grouping is by `implementation`, so the sentinel is not read for the
    /// baselines that need it.
    fn from(family: &str, params: Option<&ParamSet>) -> Self {
        let mut found: Vec<(&'static str, String)> = Vec::new();
        // Values come from the params object generically; only the two places
        // where the legacy CSV header is *not* a list of parameters need
        // naming. The rest used to be one match arm per family, kept in step
        // with `param_header` by hand.
        if let Some(params) = params {
            match family {
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
                    // not the params object, which is ordered alphabetically.
                    // Driving the loop from `fields()` emitted cms as `2048,5`
                    // under a header reading `rows,cols`: a silent
                    // column/value swap in a file that plot scripts read
                    // positionally.
                    let fields = params.fields();
                    for col in legacy_param_columns(family) {
                        if let Some((_, v)) = fields.iter().find(|(k, _)| k == col) {
                            found.push((col, legacy_float_format(v)));
                        }
                    }
                }
            }
        }
        let cols = legacy_param_columns(family)
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
        // Same matrix shape as cms/countsketch plus the tracked-key count; `k`
        // is the axis a topk sweep varies, so omitting it pooled every k into
        // one group.
        "topk" => "rows,cols,k",
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
        "topk" => &["rows", "cols", "k"],
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
        for (family, params, expected) in cases {
            let cols = ParamCols::from(family, Some(&params));
            let names: Vec<&str> = cols.cols.iter().map(|(n, _)| *n).collect();
            let header: Vec<&str> = param_header(family).split(',').collect();
            assert_eq!(names, header, "{family}: column order must match header");
            let values: Vec<&str> = cols.cols.iter().map(|(_, v)| v.as_str()).collect();
            assert_eq!(values, expected, "{family}: values misaligned");
        }
    }

    /// Every family, every params object, one value per header column.
    ///
    /// The failure this pins is silent by construction. `csv.DictReader`
    /// zips a row against the header positionally and pads the tail with
    /// `None`, so a row one field short reads *every* later column shifted
    /// by one and raises nothing. `kll/polars` run without `--config` did
    /// this: the polars baselines ignore their config, `k` had no value, and
    /// the emitted row was `impl,lang,run,<total_items>,<total_ns>,…` under
    /// a header starting `implementation,language,run,k,total_items,…` — so
    /// the plot script read `finalize_nanoseconds` as the throughput column
    /// and charted it.
    ///
    /// The parameterless `ParamSet` is the case that mattered: the CLI
    /// always hands over a params object, so a "no params at all" guard
    /// never fired for it.
    #[test]
    fn every_row_has_one_value_per_header_column() {
        let empty = ParamSet {
            family: String::new(),
            params: serde_json::json!({}),
        };
        let wrong_keys = ParamSet {
            family: String::new(),
            params: serde_json::json!({ "nonsense": 1 }),
        };
        for family in [
            "hll",
            "kll",
            "cms",
            "countsketch",
            "topk",
            "dd",
            "nitro",
            "elastic",
            "univmon",
        ] {
            let width = param_header(family).split(',').count();
            for (label, params) in [
                ("none", None),
                ("empty", Some(&empty)),
                ("wrong keys", Some(&wrong_keys)),
            ] {
                let cols = ParamCols::from(family, params);
                assert_eq!(
                    cols.join_values().split(',').count(),
                    width,
                    "{family} with {label} params: row width must match header"
                );
                let names: Vec<&str> = cols.cols.iter().map(|(n, _)| *n).collect();
                let header: Vec<&str> = param_header(family).split(',').collect();
                assert_eq!(names, header, "{family} with {label} params");
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
