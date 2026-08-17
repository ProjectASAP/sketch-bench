//! `approxbench dataset generate|describe` — produce and inspect synthetic
//! `.bin` datasets: a raw little-endian value stream for `sketchbench --input`,
//! plus a `foo.bin.meta.json` sidecar that only `describe` reads back.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};

use aqpbm_core::binfile::{self, BinMeta};
use aqpbm_datagen::{
    ColumnSpec, DataDistribution, TableDescription, UniformParameter, ZipfParameter, RULE_NONE,
};

#[derive(Parser, Debug)]
pub struct DatasetArgs {
    #[command(subcommand)]
    cmd: DatasetCmd,
}

// The `Generate` variant is a large arg struct; this enum is parsed
// once at startup, so the size asymmetry is irrelevant.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand, Debug)]
enum DatasetCmd {
    /// Generate a synthetic `.bin` dataset (+ `.meta.json` sidecar).
    Generate(GenerateArgs),
    /// Print the provenance/stats of a generated `.bin` dataset.
    Describe(DescribeArgs),
}

#[derive(Parser, Debug)]
pub struct GenerateArgs {
    /// Distribution: uniform | zipf | normal. Ignored when `--spec` is set.
    #[arg(long, default_value = "uniform")]
    shape: String,
    /// Number of values to generate.
    #[arg(long, default_value_t = 1_000_000)]
    size: usize,
    /// Seed for reproducibility.
    #[arg(long, default_value_t = 42)]
    seed: u64,
    /// Output `.bin` path. Parent directories are created if missing.
    #[arg(long)]
    out: String,
    /// Physical output type: i64 | u64 | f64. `sketchbench --input` reads i64
    /// only; f64 is benchmarkable through `sketchbench --dtype f64`, which
    /// generates in-process. u64 has no consumer in this repo.
    #[arg(long, default_value = "i64")]
    dtype: String,
    /// Uniform: max key (exclusive). Zipf: key-space size.
    #[arg(long, default_value_t = 100_000)]
    cardinality: u64,
    /// Zipf `s` exponent (only used when `--shape zipf`).
    #[arg(long, default_value_t = 1.1)]
    zipf_s: f64,
    /// Normal: mean (only used when `--shape normal`).
    #[arg(long, default_value_t = 0.0)]
    mean: f64,
    /// Normal: standard deviation (only used when `--shape normal`).
    #[arg(long, default_value_t = 1.0)]
    stddev: f64,
    /// Added to every generated value, for a column that should not start where
    /// its distribution naturally does.
    #[arg(long)]
    shift: Option<f64>,
    /// Read the full description from a `.yaml`/`.yml`/`.json` file. Overrides
    /// `--shape` and its per-shape flags (`--size`/`--seed` still apply only
    /// when NOT set here; the description file is authoritative).
    #[arg(long)]
    spec: Option<String>,
    /// Skip writing the `.meta.json` sidecar.
    #[arg(long)]
    no_meta: bool,

    // ---- retired ----
    // Kept as arguments so an invocation naming one fails by name.
    /// Retired: monotonic-timestamp start value.
    #[arg(long, hide = true)]
    start: Option<i64>,
    /// Retired: monotonic-timestamp unit label.
    #[arg(long, hide = true)]
    unit: Option<String>,
    /// Retired: monotonic-timestamp inter-arrival gap.
    #[arg(long, hide = true)]
    gap: Option<String>,
    /// Retired: monotonic-timestamp minimum gap.
    #[arg(long, hide = true)]
    min_gap: Option<u64>,
    /// Retired: skewed-categorical domain size.
    #[arg(long, hide = true)]
    categories: Option<usize>,
    /// Retired: skewed-categorical weight scheme.
    #[arg(long, hide = true)]
    weights: Option<String>,
}

#[derive(Parser, Debug)]
pub struct DescribeArgs {
    /// Path to a generated `.bin` file. Reads `<path>.meta.json` if
    /// present, else falls back to loading the raw i64 stream.
    #[arg(value_name = "PATH")]
    path: String,
}

pub fn run(args: DatasetArgs) -> Result<()> {
    match args.cmd {
        DatasetCmd::Generate(a) => generate(a),
        DatasetCmd::Describe(a) => describe(a),
    }
}

/// Normalise the `--dtype` string.
fn parse_dtype(s: &str) -> Result<String> {
    match s.to_ascii_lowercase().as_str() {
        "i64" => Ok("i64".to_string()),
        "u64" => Ok("u64".to_string()),
        "f64" => Ok("f64".to_string()),
        "string" | "str" => Ok("string".to_string()),
        other => bail!("unknown dtype: {other} (expected i64|u64|f64|string)"),
    }
}

/// Fail an invocation naming a flag whose shape the description does not carry.
/// Refused one by one rather than ignored: generating a uniform column for a
/// request that asked for timestamps would look to a driver like a success.
fn reject_retired(a: &GenerateArgs) -> Result<()> {
    let retired: [(&str, bool); 8] = [
        ("--start", a.start.is_some()),
        ("--unit", a.unit.is_some()),
        ("--gap", a.gap.is_some()),
        ("--min-gap", a.min_gap.is_some()),
        ("--categories", a.categories.is_some()),
        ("--weights", a.weights.is_some()),
        (
            "--shape monotonic-timestamp",
            matches!(a.shape.as_str(), "monotonic-timestamp" | "timestamp"),
        ),
        (
            "--shape skewed-categorical",
            matches!(a.shape.as_str(), "skewed-categorical" | "categorical"),
        ),
    ];
    let named: Vec<&str> = retired
        .iter()
        .filter(|(_, used)| *used)
        .map(|(name, _)| *name)
        .collect();
    if !named.is_empty() {
        bail!(
            "{} names a generator shape this build does not have. The description \
             carries uniform, zipf and normal, plus the `special_rule` mask for a \
             monotonic series; the categorical and timestamp shapes have no form \
             in it yet.",
            named.join(", "),
        );
    }
    Ok(())
}

/// Build a [`TableDescription`] from CLI flags, or load it from `--spec`.
fn resolve_spec(a: &GenerateArgs) -> Result<TableDescription> {
    if let Some(path) = a.spec.as_deref() {
        return TableDescription::from_path(Path::new(path))
            .with_context(|| format!("loading spec from {path}"));
    }
    reject_retired(a)?;
    let distribution = match a.shape.as_str() {
        "uniform" => DataDistribution::Uniform(UniformParameter {
            lower_bound: 0.0,
            upper_bound: a.cardinality as f64,
            seed: a.seed,
        }),
        "zipf" => DataDistribution::Zipf(ZipfParameter {
            skewness: a.zipf_s,
            population_size: a.cardinality,
            seed: a.seed,
        }),
        "normal" => DataDistribution::Normal(aqpbm_datagen::NormalParameter {
            mean: a.mean,
            standard_deviation: a.stddev,
            seed: a.seed,
        }),
        other => bail!("unknown shape: {other} (expected uniform|zipf|normal)"),
    };
    Ok(TableDescription::single(
        "value",
        ColumnSpec {
            distribution,
            shift: a.shift,
            cardinality: None,
            special_rule: RULE_NONE,
            data_type: parse_dtype(&a.dtype)?,
            // No flags for the string options: the only destination here is a
            // `.bin`, which cannot hold strings, so they would configure a path
            // that always errors. `--spec` already sets them for library callers.
            string: None,
        },
        a.size as u64,
    ))
}

fn generate(a: GenerateArgs) -> Result<()> {
    let spec = resolve_spec(&a)?;
    let out = Path::new(&a.out);
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating output dir {}", parent.display()))?;
        }
    }

    if spec.column_spec.len() != 1 {
        bail!(
            "a `.bin` holds one column, but the description has {}; \
             `sketchbench --spec` reads a multi-column description in-process",
            spec.column_spec.len()
        );
    }
    let shape = spec.column_spec[0].distribution.tag();
    let column = spec
        .generate()
        .with_context(|| format!("generating into {}", out.display()))?
        .into_column(0)
        .map_err(|e| anyhow!("{e}"))?;

    binfile::write_bin(out, &column).with_context(|| format!("writing {}", out.display()))?;
    let meta = BinMeta::new(&spec, &column);

    let sidecar = if a.no_meta {
        None
    } else {
        Some(binfile::write_meta(out, &meta).context("writing sidecar")?)
    };

    eprintln!(
        "approxbench: generated shape={} dtype={} count={} -> {}",
        shape, meta.dtype, meta.count, a.out,
    );
    if let Some(p) = sidecar {
        eprintln!("approxbench: sidecar -> {}", p.display());
    }
    Ok(())
}

fn describe(a: DescribeArgs) -> Result<()> {
    let path = Path::new(&a.path);
    match binfile::read_meta(path)? {
        Some(meta) => {
            let lead = meta.description.column_spec.first();
            println!("path:              {}", a.path);
            println!("schema_version:    {}", meta.schema_version);
            println!("generator_version: {}", meta.generator_version);
            println!(
                "shape:             {}",
                lead.map(|c| c.distribution.tag()).unwrap_or("-")
            );
            println!(
                "shape_params:      {}",
                serde_json::to_string(&meta.description)?
            );
            println!("dtype:             {}", meta.dtype);
            println!("count:             {}", meta.count);
            println!(
                "seed:              {}",
                lead.map(|c| c.distribution.seed().to_string())
                    .unwrap_or_else(|| "-".into())
            );
            print_stats(&meta.stats);
        }
        None => {
            // No sidecar: fall back to the raw i64 loader.
            use aqpbm_core::dataset::{Dataset, I64Dataset};
            let wk = I64Dataset::load(path)
                .with_context(|| format!("loading {} (no sidecar found)", a.path))?;
            let items = wk.items();
            println!("path:   {}", a.path);
            println!("dtype:  i64 (assumed — no .meta.json sidecar)");
            println!("count:  {}", items.len());
            if let (Some(min), Some(max)) = (items.iter().min(), items.iter().max()) {
                println!("min:    {min}");
                println!("max:    {max}");
                println!("first:  {}", items[0]);
                println!("last:   {}", items[items.len() - 1]);
            }
        }
    }
    Ok(())
}

fn print_stats(s: &aqpbm_core::binfile::BasicStats) {
    let fmt = |v: Option<f64>| v.map(|x| x.to_string()).unwrap_or_else(|| "-".into());
    println!("min:               {}", fmt(s.min));
    println!("max:               {}", fmt(s.max));
    println!("first:             {}", fmt(s.first));
    println!("last:              {}", fmt(s.last));
}
