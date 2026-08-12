//! CLI surface: the clap argument structs. The logic that consumes them
//! lives in `main.rs` (`run_sketchbench`, `workload_spec`); the sketch catalog
//! it dispatches through lives in `sketch_bench::catalog`.
//!
//! Option help and grouping track `docs/aqpbm-cli-reference.md`, which is
//! hand-authored and authoritative. `scripts/dump_cli_reference.sh` diffs the
//! built binary against it.

use clap::{Parser, Subcommand};

use crate::workload_cmd;

#[derive(Parser, Debug)]
// `max_term_width` is pinned so `--help` renders identically under any
// terminal. `docs/aqpbm-cli-reference.md` is a checked-in expectation, and a
// reference that reflowed with the operator's window could never be diffed.
#[command(
    name = "approxbench",
    version,
    about = "Approximate query processing benchmark suite",
    max_term_width = 96
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Measure one cell of the sketch bundle.
    Sketchbench(SketchbenchArgs),
    /// Generate or inspect synthetic `.bin` workloads.
    Workload(workload_cmd::WorkloadArgs),
}

/// One cell of the sketch bundle: one algorithm, one impl, one construction
/// point, one workload. `--list-impls` is the one mode that selects no cell,
/// which is why the identity options are optional in the type and required by
/// clap only when it is absent.
// The auto-generated help flag is inserted ahead of every declared arg, which
// would put its `Options:` block above `Identity:`. Declaring it by hand at the
// end of the struct is what puts it where the reference has it, last.
#[derive(Parser, Debug)]
#[command(
    disable_help_flag = true,
    override_usage = "approxbench sketchbench [OPTIONS] --algorithm <ALGORITHM> --impl <IMPL_NAME>\n       approxbench sketchbench --list-impls"
)]
pub struct SketchbenchArgs {
    /// Accumulator algorithm, structural variant included: `cms` and
    /// `cms-fastpath-vector2d` are two of them, because a different hash
    /// strategy gives different estimates. Matched exactly, since one
    /// invocation measures one cell. `--list-impls` prints every
    /// (algorithm, impl) pair, grouped by the family they share knobs with.
    #[arg(long, required_unless_present = "list_impls", help_heading = "Identity")]
    pub algorithm: Option<String>,
    /// Implementing library, and only that: `oxide`, `datasketches`, `lib` or
    /// `polars`. `--list-impls` shows which the algorithm offers. One
    /// invocation measures one (impl, config) cell; to race several, invoke
    /// once per impl.
    #[arg(
        long = "impl",
        required_unless_present = "list_impls",
        help_heading = "Identity"
    )]
    pub impl_name: Option<String>,
    /// Print every (algorithm, impl) pair this bundle offers, then exit.
    /// Selects no cell and writes no record, so it ignores every other option.
    #[arg(long, help_heading = "Identity")]
    pub list_impls: bool,

    /// Construction config for this cell: `'k1=v1 k2=v2'`, one value per key.
    /// Omitted gives a parameterless point, which every row rejects by name.
    /// A comma list is an error: one invocation is one cell, never a grid.
    ///
    /// A row builds at exactly the values given or refuses them, naming the
    /// bound it has. Nothing is clamped, rounded or ignored, so the
    /// `sketch_config` in the record is always the config that ran.
    #[arg(long, help_heading = "Construction")]
    pub config: Option<String>,
    /// Worker threads for the parallel-insert algorithms (`*-parallel`). Every
    /// other row ignores it; `1` is single-threaded.
    #[arg(long, default_value_t = 1, help_heading = "Construction")]
    pub workers: usize,

    /// Load the workload from a file instead of generating it. Format comes
    /// from the extension: `.bin` (little-endian i64), `.pcap` (IPv4 src addr),
    /// `.csv` (column 0 below a header). Wins over `--spec` and over the inline
    /// options.
    #[arg(long, help_heading = "Workload")]
    pub input: Option<String>,
    /// Generate in-process from a `datagen` spec file (examples in
    /// `configs/datagen/`), unlocking every generator shape without a disk
    /// round-trip. Wins over the inline options; `--input` wins over it.
    ///
    /// A file holding a *list* of specs is a multi-column stream: the last
    /// column is the value, the ones before it are labels. The rows ingesting
    /// labelled records (the `hydra-*` algorithms) need one; every other row
    /// refuses it. `hydra-kll` reads the value column as `f64`, the other two
    /// as `i64`.
    #[arg(long, help_heading = "Workload")]
    pub spec: Option<String>,
    /// Inline shape: "uniform" or "zipf". Ignored when `--input` or `--spec` is
    /// set.
    #[arg(long, default_value = "uniform", help_heading = "Workload")]
    pub workload: String,
    /// Number of items in the workload.
    #[arg(long, default_value_t = 1_000_000, help_heading = "Workload")]
    pub size: usize,
    /// Cardinality (uniform: max key; zipf: key-space size).
    #[arg(long, default_value_t = 100_000, help_heading = "Workload")]
    pub cardinality: u64,
    /// Zipf `s` exponent (only used when `--workload zipf`).
    #[arg(long, default_value_t = 1.1, help_heading = "Workload")]
    pub zipf_s: f64,
    /// Numeric width for the ordered algorithms (`kll-percall`, `kll-cdf`,
    /// `dd`): `i64` or `f64`. The one item-type choice left, since every other
    /// row's is fixed by its wrapper, and `f64` elsewhere is refused by name.
    /// Encoding only.
    #[arg(long, default_value = "i64", help_heading = "Workload")]
    pub dtype: String,
    /// Alphabet for generated string keys, for the rows whose wrappers take
    /// text. Character order is the digit order of the positional encoding, so
    /// a rank always renders the same key. Overridden by `--spec`'s own
    /// `string:` block.
    #[arg(
        long,
        default_value = "abcdefghijklmnopqrstuvwxyz0123456789",
        help_heading = "Workload"
    )]
    pub alphabet: String,
    /// Inclusive length bounds for generated string keys; equal values give a
    /// fixed length. Key length dominates the cost of a hashing insert path, so
    /// it is the knob worth sweeping.
    #[arg(
        long = "key-len",
        num_args = 1..=2,
        default_values_t = [8usize, 24usize],
        help_heading = "Workload"
    )]
    pub key_len: Vec<usize>,
    /// Seed for reproducibility.
    #[arg(long, default_value_t = 42, help_heading = "Workload")]
    pub seed: u64,

    /// Measured runs inside one process, summarised as mean / stddev /
    /// `throughput_samples`. They share a process, so they support no
    /// confidence interval. See `--repeats`.
    #[arg(long, default_value_t = 10, help_heading = "Repetition")]
    pub runs: usize,
    /// Runs discarded before measurement begins. A cold allocator and a cold
    /// cache measure the wrong thing.
    #[arg(long, default_value_t = 3, help_heading = "Repetition")]
    pub warmup_runs: usize,
    /// Re-execute the whole invocation in this many separate processes and
    /// report the 95% CI over their means. The only setting that makes `ci95`
    /// appear. At 1 no interval is claimed. Costs R times the wall clock.
    #[arg(long, default_value_t = 1, help_heading = "Repetition")]
    pub repeats: usize,

    /// Comma-separated: throughput,latency,accuracy,cpu,memory. What is
    /// measured. Crossed with `--operations` to give the measurements this run
    /// takes; cpu and memory attach to every one instead of forming their own.
    /// Required: nothing is measured that was not asked for.
    #[arg(
        long,
        required_unless_present = "list_impls",
        help_heading = "Measurement content"
    )]
    pub metrics: Option<String>,
    /// Which ground truth scores this run, by name. A sketch registered against
    /// several is a row per ground truth, and `--list-impls` is what lists them;
    /// omitted takes the first row for the (algorithm, impl) pair.
    #[arg(long, help_heading = "Measurement content")]
    pub ground_truth: Option<String>,
    /// Comma-separated: insert,query,merge,prepare. What each metric is
    /// measured over. Required, like `--metrics`: a square nothing measures is
    /// refused by name, so a request states which squares it wants rather than
    /// inheriting a guess.
    #[arg(
        long,
        required_unless_present = "list_impls",
        help_heading = "Measurement content"
    )]
    pub operations: Option<String>,
    /// How many shards the merge operation folds. A knob, never a selector:
    /// measuring merge is asked for with `--operations merge`. Linear sketches
    /// merge exactly, so a gap is a defect; for KLL it is the result.
    #[arg(long, default_value_t = 2, help_heading = "Measurement content")]
    pub merge_shards: usize,

    /// Path to append JSONL records to. `-` or omitted sends them to stdout.
    #[arg(long, help_heading = "Output")]
    pub report: Option<String>,
    /// Output directory for long-format CSVs, one row per measured run, named
    /// `<family>_throughput[_query]_results_rust.csv` — one file per family, so
    /// a structural variant does not fork the file a plot script reads; the
    /// variant lands in the `implementation` column. Coexists with `--report`.
    #[arg(long, help_heading = "Output")]
    pub raw_csv: Option<String>,
    /// Write one line for the whole cell instead of one per square, folding
    /// the cell's records into a single flattened row: one slot per operation,
    /// one field per metric. One record per square is the default; this is the
    /// shape a leaderboard reads.
    #[arg(long, default_value_t = false, help_heading = "Output")]
    pub flat: bool,

    #[arg(
        short = 'h',
        long = "help",
        action = clap::ArgAction::Help,
        help = "Print help",
        long_help = "Print help (see a summary with '-h')",
        help_heading = "Options"
    )]
    pub help: Option<bool>,
}
