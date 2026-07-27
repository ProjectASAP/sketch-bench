//! CLI surface: the clap argument structs. The logic that consumes them
//! lives in `main.rs` (`run_bench`, `workload_spec`); the sketch catalog it
//! dispatches through lives in `sketch_bench::catalog`.

use clap::{Parser, Subcommand};

use crate::workload_cmd;

#[derive(Parser, Debug)]
#[command(name = "sketchlib", version, about = "Unified sketchlib-tool CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Measure one `(impl, config)` cell of a sketch family.
    Bench(BenchArgs),
    /// List every `(family, impl)` pair the CLI can drive.
    ListImpls,
    /// Generate or inspect synthetic `.bin` workloads.
    Workload(workload_cmd::WorkloadArgs),
}

#[derive(Parser, Debug)]
pub struct BenchArgs {
    /// Accumulator family (hll, kll, cms, countsketch, dd, topk, elastic, nitro,
    /// univmon). `list-impls` prints every (family, impl) pair.
    #[arg(long)]
    pub sketch: String,
    /// Implementation within the family — exactly one (`oxide`).
    /// `list-impls` shows the choices. One invocation measures one
    /// (impl, config) cell; to race several, invoke once per impl.
    #[arg(long = "impl")]
    pub impl_name: String,
    /// Number of measured iterations per `(impl, config)` pair, inside one
    /// process. Summarised as mean / stddev / `throughput_samples`. These
    /// iterations share a process, so they do **not** support a confidence
    /// interval — see `--repeats`.
    #[arg(long, default_value_t = 10)]
    pub runs: usize,
    /// Re-execute the whole benchmark in this many **separate processes** and
    /// report the 95% confidence interval over their per-process means.
    ///
    /// This is the only setting that makes `ci95` appear in the output: a
    /// fresh process is what varies the allocator arena, address-space layout,
    /// governor ramp and page-cache state that `--runs` holds constant. Left
    /// at 1 (the default) no interval is claimed, because none can be computed
    /// honestly. Costs R times the wall clock.
    #[arg(long, default_value_t = 1)]
    pub repeats: usize,
    /// Warm-up runs before measurement.
    #[arg(long, default_value_t = 3)]
    pub warmup_runs: usize,
    /// Workload shape: "uniform" or "zipf". Ignored when
    /// `--input` is set (the file replaces the generator).
    #[arg(long, default_value = "uniform")]
    pub workload: String,
    /// Number of items in the workload.
    #[arg(long, default_value_t = 1_000_000)]
    pub size: usize,
    /// Cardinality (uniform: max key; zipf: key-space size).
    #[arg(long, default_value_t = 100_000)]
    pub cardinality: u64,
    /// Zipf `s` exponent (only used when `--workload zipf`).
    #[arg(long, default_value_t = 1.1)]
    pub zipf_s: f64,
    /// Numeric width for the ordered families (`kll`, `dd`): `i64`
    /// (default) or `f64`.
    ///
    /// Every other row's item type is fixed by its wrapper — the rows that
    /// take text always take text — so this is the one item-type choice
    /// left, and asking for `f64` anywhere else is refused by name before a
    /// workload is generated.
    ///
    /// The values themselves do not change with it, only their encoding.
    #[arg(long, default_value = "i64")]
    pub dtype: String,
    /// Alphabet for generated string keys, for the rows whose wrappers take
    /// text (`elastic`, `univmon`, `nitro/oxide`).
    ///
    /// Character order is the digit order of the positional encoding, so a
    /// rank always renders to the same key. Ignored by rows that ingest
    /// numbers. Overridden by `--spec`, which carries its own `string:`
    /// block.
    #[arg(long, default_value = "abcdefghijklmnopqrstuvwxyz0123456789")]
    pub alphabet: String,
    /// Inclusive length bounds for generated string keys. Equal values give
    /// fixed-length keys.
    ///
    /// Key length is the dominant cost on a hashing insert path, so it is
    /// the knob worth sweeping: `--key-len 1 7` approximates the
    /// decimal-rendered integers these rows used to be fed.
    #[arg(long = "key-len", num_args = 1..=2, default_values_t = [8usize, 24usize])]
    pub key_len: Vec<usize>,
    /// Seed for reproducibility.
    #[arg(long, default_value_t = 42)]
    pub seed: u64,
    /// Load the workload from a file instead of generating it.
    /// Format is auto-detected from the extension:
    /// `.bin` (little-endian i64 stream), `.pcap` (IPv4 src
    /// addr per packet), `.csv` (first column parsed as i64
    /// after a header row). Overrides `--workload/--size/
    /// --cardinality/--zipf-s/--seed`.
    #[arg(long)]
    pub input: Option<String>,
    /// Generate the workload in-process from a `datagen` spec file
    /// (`.yaml`/`.yml`/`.json`, same format `workload generate --spec`
    /// takes; examples in `configs/datagen/`). Unlocks every generator
    /// shape — categorical id domains, monotonic timestamp series —
    /// without a round-trip through disk. Overrides
    /// `--workload/--size/--cardinality/--zipf-s/--seed`; `--input`
    /// wins over it.
    #[arg(long)]
    pub spec: Option<String>,
    /// Path to append JSONL records to. `-` or omitted → stdout.
    #[arg(long)]
    pub report: Option<String>,
    /// Optional output directory for legacy long-format CSVs (one
    /// row per measured run). Files are named
    /// `<family>_throughput_results_rust.csv` and, when a query
    /// phase runs, `<family>_throughput_query_results_rust.csv`.
    /// Coexists with `--report`; intended for plot scripts that
    /// still consume the historical CSV shape.
    #[arg(long)]
    pub raw_csv: Option<String>,
    /// Worker threads for parallel-insert impls
    /// (`lib-fastpath-parallel` under cms / countsketch / hll).
    /// Other impls ignore it. Default `1` reproduces the
    /// single-threaded behaviour.
    #[arg(long, default_value_t = 1)]
    pub workers: usize,
    /// Split the stream into this many shards, build one sketch per shard,
    /// and time folding them into one — then compare the merged result
    /// against the whole stream. `1` (default) skips the merge pass.
    ///
    /// Mergeability is what lets a sketch be computed per shard, per node or
    /// per time window and combined later, and it is close to unmeasured in
    /// the literature: papers prove it and then evaluate insert and query.
    /// For linear sketches (Count-Min, Count Accumulator, HLL at equal lg_k) the
    /// merge is exact, so accuracy here must match the single-pass figure and
    /// a gap is a defect. For KLL it is lossy, and the gap is the result.
    #[arg(long, default_value_t = 1)]
    pub merge_shards: usize,
    /// Comma-separated metric flags: throughput,latency,cpu,memory,accuracy,merge.
    /// Default: all except merge (merge needs `--merge-shards`).
    #[arg(long)]
    pub metrics: Option<String>,
    /// Construction config for this cell: `'k1=v1 k2=v2'`, one value
    /// per key (e.g. `'rows=5 cols=2048'` for cms). Omitted → a
    /// parameterless point, which impls with tunable knobs reject at
    /// construction (naming the missing field). A comma list of values
    /// is an error: one invocation is one cell, not a grid.
    #[arg(long)]
    pub config: Option<String>,
    /// Compute ground-truth accuracy per run. Implies
    /// `MetricsMask::ACCURACY`. Per-family comparator: CMS /
    /// CountSketch → frequency (L1/L2/rel-err); HLL →
    /// cardinality (rel-err); KLL → quantile rank-err; DD →
    /// quantile relative-error; topk → precision/recall at k.
    /// Elastic, Nitro, UnivMon and the parallel-insert rows
    /// declare no query capability, so no comparator scores
    /// them: they still run, timed only, after a stderr note.
    #[arg(long, default_value_t = false)]
    pub accuracy: bool,
    /// Cap on the number of distinct keys probed by the
    /// frequency comparator (CMS / CountSketch). `0` = probe
    /// every distinct key. Ignored by the cardinality /
    /// quantile / top-k comparators.
    #[arg(long, default_value_t = 100_000)]
    pub accuracy_probes: usize,
}
