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
    /// univmon, hydra). `list-impls` prints every (family, impl) pair.
    #[arg(long)]
    pub sketch: String,
    /// Implementation within the family — exactly one (`oxide`).
    /// `list-impls` shows the choices. One invocation measures one
    /// (impl, config) cell; to race several, invoke once per impl.
    #[arg(long = "impl")]
    pub impl_name: String,
    /// Measured iterations per `(impl, config)` pair, inside one process,
    /// summarised as mean / stddev / `throughput_samples`. They share a process,
    /// so they do **not** support a confidence interval — see `--repeats`.
    #[arg(long, default_value_t = 10)]
    pub runs: usize,
    /// Re-execute the benchmark in this many **separate processes** and report
    /// the 95% CI over their means — the only setting that makes `ci95` appear.
    /// At 1 (default) no interval is claimed. Costs R× the wall clock.
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
    /// Numeric width for the ordered families (`kll`, `dd`): `i64` (default) or
    /// `f64`. The one item-type choice left — every other row's is fixed by its
    /// wrapper, and `f64` elsewhere is refused by name. Encoding only.
    #[arg(long, default_value = "i64")]
    pub dtype: String,
    /// Alphabet for generated string keys, for the rows whose wrappers take text.
    /// Character order is the digit order of the positional encoding, so a rank
    /// always renders the same key. Overridden by `--spec`'s own `string:` block.
    #[arg(long, default_value = "abcdefghijklmnopqrstuvwxyz0123456789")]
    pub alphabet: String,
    /// Inclusive length bounds for generated string keys; equal values give a
    /// fixed length. Key length dominates the cost of a hashing insert path, so
    /// it is the knob worth sweeping.
    #[arg(long = "key-len", num_args = 1..=2, default_values_t = [8usize, 24usize])]
    pub key_len: Vec<usize>,
    /// Seed for reproducibility.
    #[arg(long, default_value_t = 42)]
    pub seed: u64,
    /// Load the workload from a file instead of generating it; format from the
    /// extension — `.bin` (little-endian i64), `.pcap` (IPv4 src addr), `.csv`
    /// (column 0 below a header). Overrides the `--workload` flags.
    #[arg(long)]
    pub input: Option<String>,
    /// Generate in-process from a `datagen` spec file (examples in
    /// `configs/datagen/`), unlocking every generator shape without a disk
    /// round-trip. Overrides the `--workload` flags; `--input` wins over it.
    ///
    /// A file holding a *list* of specs is a multi-column stream: the last
    /// column is the value, the ones before it are labels. The rows ingesting
    /// labelled records (`hydra`) need one; every other row refuses it.
    #[arg(long)]
    pub spec: Option<String>,
    /// Path to append JSONL records to. `-` or omitted → stdout.
    #[arg(long)]
    pub report: Option<String>,
    /// Output directory for long-format CSVs, one row per measured run, named
    /// `<family>_throughput[_query]_results_rust.csv`. Coexists with `--report`;
    /// for plot scripts that consume that CSV shape.
    #[arg(long)]
    pub raw_csv: Option<String>,
    /// Worker threads for the parallel-insert impls (`lib-fastpath-parallel`).
    /// Other impls ignore it; `1` (default) is single-threaded.
    #[arg(long, default_value_t = 1)]
    pub workers: usize,
    /// Split the stream into this many shards, time folding them into one, and
    /// compare against the whole stream; `1` (default) skips the pass. Linear
    /// sketches merge exactly, so a gap is a defect; for KLL it is the result.
    #[arg(long, default_value_t = 1)]
    pub merge_shards: usize,
    /// Comma-separated metric flags: throughput,latency,cpu,memory,accuracy,merge.
    /// Default: all except merge (merge needs `--merge-shards`).
    #[arg(long)]
    pub metrics: Option<String>,
    /// Construction config for this cell: `'k1=v1 k2=v2'`, one value per key.
    /// Omitted → a parameterless point, which tunable impls reject by name. A
    /// comma list is an error: one invocation is one cell, not a grid.
    #[arg(long)]
    pub config: Option<String>,
    /// Compute ground-truth accuracy per run, one comparator per family. Rows
    /// declaring no query capability are not scored — they still run, timed
    /// only, after a stderr note.
    #[arg(long, default_value_t = false)]
    pub accuracy: bool,
    /// Cap on distinct keys probed by the frequency comparator; `0` probes every
    /// one. Ignored by the cardinality / quantile / top-k comparators.
    #[arg(long, default_value_t = 100_000)]
    pub accuracy_probes: usize,
}
