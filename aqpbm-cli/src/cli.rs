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
    /// Accumulator algorithm (hll, kll, cms, countsketch, dd, topk, elastic,
    /// nitro, univmon, hydra). `--list-impls` prints every (algorithm, impl)
    /// pair.
    #[arg(long, required_unless_present = "list_impls", help_heading = "Identity")]
    pub algorithm: Option<String>,
    /// Implementation within the algorithm, exactly one (`oxide`).
    /// `--list-impls` shows the choices. One invocation measures one
    /// (impl, config) cell; to race several, invoke once per impl.
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
    /// Omitted gives a parameterless point, which tunable impls reject by name.
    /// A comma list is an error: one invocation is one cell, never a grid.
    #[arg(long, help_heading = "Construction")]
    pub config: Option<String>,
    /// Worker threads for the parallel-insert impls (`lib-fastpath-parallel`).
    /// Other impls ignore it; `1` is single-threaded.
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
    /// labelled records (`hydra`) need one; every other row refuses it.
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
    /// Numeric width for the ordered algorithms (`kll`, `dd`): `i64` or `f64`.
    /// The one item-type choice left, since every other row's is fixed by its
    /// wrapper, and `f64` elsewhere is refused by name. Encoding only.
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

    /// Comma-separated: throughput,latency,cpu,memory,accuracy,merge. Each of
    /// throughput, latency, accuracy and merge is one pass over the workload;
    /// cpu and memory attach to whichever passes run. Default: all except
    /// merge.
    #[arg(long, help_heading = "Measurement content")]
    pub metrics: Option<String>,
    /// Compute ground-truth accuracy per run, one comparator per algorithm.
    /// Rows declaring no query capability are not scored: they still run, timed
    /// only, after a stderr note.
    #[arg(long, default_value_t = false, help_heading = "Measurement content")]
    pub accuracy: bool,
    /// Cap on distinct keys probed by the frequency comparator; `0` probes
    /// every one. Ignored by the cardinality / quantile / top-k comparators.
    #[arg(
        long,
        default_value_t = 100_000,
        help_heading = "Measurement content"
    )]
    pub accuracy_probes: usize,
    /// Split the stream into this many shards, time folding them into one, and
    /// compare against the whole stream; `1` skips the pass. Linear sketches
    /// merge exactly, so a gap is a defect; for KLL it is the result.
    #[arg(long, default_value_t = 1, help_heading = "Measurement content")]
    pub merge_shards: usize,

    /// Path to append JSONL records to. `-` or omitted sends them to stdout.
    #[arg(long, help_heading = "Output")]
    pub report: Option<String>,
    /// Output directory for long-format CSVs, one row per measured run, named
    /// `<algorithm>_throughput[_query]_results_rust.csv`. Coexists with
    /// `--report`; for plot scripts that consume that CSV shape.
    #[arg(long, help_heading = "Output")]
    pub raw_csv: Option<String>,

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
