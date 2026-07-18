//! `sketchlib workload generate|describe` — produce and inspect
//! synthetic `.bin` workloads.
//!
//! Generation writes a raw little-endian value stream plus a
//! `foo.bin.meta.json` provenance sidecar. The stream is consumed by
//! `sketchlib bench --input <path>`; the sidecar is ignored by the
//! benchmark and read back by `describe`.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};

use sketch_core::datagen::{self, DType, GapDist, GenMeta, GenSpec, Shape, TimeUnit, WeightSpec};

#[derive(Parser, Debug)]
pub struct WorkloadArgs {
    #[command(subcommand)]
    cmd: WorkloadCmd,
}

// The `Generate` variant is a large arg struct; this enum is parsed
// once at startup, so the size asymmetry is irrelevant.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand, Debug)]
enum WorkloadCmd {
    /// Generate a synthetic `.bin` workload (+ `.meta.json` sidecar).
    Generate(GenerateArgs),
    /// Print the provenance/stats of a generated `.bin` workload.
    Describe(DescribeArgs),
}

#[derive(Parser, Debug)]
pub struct GenerateArgs {
    /// Distribution: uniform | zipf | monotonic-timestamp |
    /// skewed-categorical. Ignored when `--spec` is set.
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
    /// Physical output type: i64 | u64 | f64. Only i64 is consumable by
    /// `bench` today; u64/f64 files are written for other tooling.
    #[arg(long, default_value = "i64")]
    dtype: String,
    /// Uniform: max key (exclusive). Zipf: key-space size.
    #[arg(long, default_value_t = 100_000)]
    cardinality: u64,
    /// Zipf `s` exponent (only used when `--shape zipf`).
    #[arg(long, default_value_t = 1.1)]
    zipf_s: f64,
    /// Monotonic-timestamp: starting value (first emitted value).
    #[arg(long, default_value_t = 0)]
    start: i64,
    /// Monotonic-timestamp: unit label — nanos | millis | secs.
    #[arg(long, default_value = "nanos")]
    unit: String,
    /// Monotonic-timestamp: inter-arrival gap as `kind:param`
    /// (const:1000 | geometric:0.01 | exp:0.5 | poisson:5). Required for
    /// `--shape monotonic-timestamp`.
    #[arg(long)]
    gap: Option<String>,
    /// Monotonic-timestamp: minimum gap (1 → strictly increasing,
    /// 0 → duplicates allowed).
    #[arg(long, default_value_t = 1)]
    min_gap: u64,
    /// Skewed-categorical: size of the id domain (ids `0..n`). Required
    /// for `--shape skewed-categorical` (use `--spec` for explicit ids).
    #[arg(long)]
    categories: Option<usize>,
    /// Skewed-categorical: weight scheme — `uniform` | `zipf:s`.
    #[arg(long, default_value = "zipf:1.1")]
    weights: String,
    /// Read the full spec from a `.yaml`/`.yml`/`.json` file. Overrides
    /// `--shape` and its per-shape flags (`--size`/`--seed` still apply
    /// only when NOT set here; the spec file is authoritative).
    #[arg(long)]
    spec: Option<String>,
    /// Skip writing the `.meta.json` sidecar.
    #[arg(long)]
    no_meta: bool,
}

#[derive(Parser, Debug)]
pub struct DescribeArgs {
    /// Path to a generated `.bin` file. Reads `<path>.meta.json` if
    /// present, else falls back to loading the raw i64 stream.
    #[arg(value_name = "PATH")]
    path: String,
}

pub fn run(args: WorkloadArgs) -> Result<()> {
    match args.cmd {
        WorkloadCmd::Generate(a) => generate(a),
        WorkloadCmd::Describe(a) => describe(a),
    }
}

fn parse_dtype(s: &str) -> Result<DType> {
    match s.to_ascii_lowercase().as_str() {
        "i64" => Ok(DType::I64),
        "u64" => Ok(DType::U64),
        "f64" => Ok(DType::F64),
        other => bail!("unknown dtype: {other} (expected i64|u64|f64)"),
    }
}

fn parse_unit(s: &str) -> Result<TimeUnit> {
    match s.to_ascii_lowercase().as_str() {
        "nanos" | "ns" => Ok(TimeUnit::Nanos),
        "millis" | "ms" => Ok(TimeUnit::Millis),
        "secs" | "s" => Ok(TimeUnit::Secs),
        other => bail!("unknown unit: {other} (expected nanos|millis|secs)"),
    }
}

/// Parse a `kind:param` gap spec, e.g. `geometric:0.01`, `const:1000`.
fn parse_gap(s: &str) -> Result<GapDist> {
    let (kind, param) = s
        .split_once(':')
        .ok_or_else(|| anyhow!("--gap must be `kind:param`, e.g. geometric:0.01"))?;
    let num = || -> Result<f64> {
        param
            .parse::<f64>()
            .map_err(|_| anyhow!("invalid gap param '{param}' for '{kind}'"))
    };
    match kind.to_ascii_lowercase().as_str() {
        "const" | "constant" => Ok(GapDist::Constant {
            step: param
                .parse::<u64>()
                .map_err(|_| anyhow!("invalid const step '{param}'"))?,
        }),
        "geometric" | "geo" => Ok(GapDist::Geometric { p: num()? }),
        "exp" | "exponential" => Ok(GapDist::Exponential { lambda: num()? }),
        "poisson" => Ok(GapDist::Poisson { lambda: num()? }),
        other => bail!("unknown gap kind: {other} (expected const|geometric|exp|poisson)"),
    }
}

/// Parse a weight scheme: `uniform` or `zipf:s`. Explicit weights are
/// only available via `--spec`.
fn parse_weights(s: &str) -> Result<WeightSpec> {
    if s.eq_ignore_ascii_case("uniform") {
        return Ok(WeightSpec::Uniform);
    }
    if let Some(param) = s.strip_prefix("zipf:") {
        return Ok(WeightSpec::Zipf {
            s: param
                .parse::<f64>()
                .map_err(|_| anyhow!("invalid zipf weight exponent '{param}'"))?,
        });
    }
    bail!("unknown weights: {s} (expected uniform|zipf:s, or use --spec for explicit)")
}

/// Build a [`GenSpec`] from CLI flags, or load it from `--spec`.
fn resolve_spec(a: &GenerateArgs) -> Result<GenSpec> {
    if let Some(path) = a.spec.as_deref() {
        return GenSpec::from_path(Path::new(path))
            .with_context(|| format!("loading spec from {path}"));
    }
    let dtype = parse_dtype(&a.dtype)?;
    let shape = match a.shape.as_str() {
        "uniform" => Shape::Uniform {
            cardinality: a.cardinality,
            dtype,
        },
        "zipf" => Shape::Zipf {
            cardinality: a.cardinality,
            s: a.zipf_s,
            dtype,
        },
        "monotonic-timestamp" | "timestamp" => {
            let gap = a
                .gap
                .as_deref()
                .ok_or_else(|| {
                    anyhow!("--gap is required for monotonic-timestamp (e.g. --gap geometric:0.01)")
                })
                .and_then(parse_gap)?;
            Shape::MonotonicTimestamp {
                start: a.start,
                unit: parse_unit(&a.unit)?,
                gap,
                min_gap: a.min_gap,
                dtype,
            }
        }
        "skewed-categorical" | "categorical" => {
            let n = a.categories.ok_or_else(|| {
                anyhow!("--categories <n> is required for skewed-categorical")
            })?;
            if n == 0 {
                bail!("--categories must be > 0");
            }
            Shape::SkewedCategorical {
                categories: (0..n as i64).collect(),
                weights: parse_weights(&a.weights)?,
            }
        }
        other => bail!(
            "unknown shape: {other} \
             (expected uniform|zipf|monotonic-timestamp|skewed-categorical)"
        ),
    };
    Ok(GenSpec {
        shape,
        size: a.size,
        seed: a.seed,
    })
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

    let col = spec.generate().context("generating column")?;
    datagen::io::write_bin(out, &col).with_context(|| format!("writing {}", a.out))?;

    let meta = GenMeta::new(&spec, &col);
    let sidecar = if a.no_meta {
        None
    } else {
        Some(datagen::io::write_meta(out, &meta).context("writing sidecar")?)
    };

    eprintln!(
        "sketchlib: generated shape={} dtype={} count={} -> {}",
        spec.shape.tag(),
        col.dtype().as_str(),
        col.len(),
        a.out,
    );
    if let Some(p) = sidecar {
        eprintln!("sketchlib: sidecar -> {}", p.display());
    }
    Ok(())
}

fn describe(a: DescribeArgs) -> Result<()> {
    let path = Path::new(&a.path);
    match datagen::io::read_meta(path)? {
        Some(meta) => {
            println!("path:              {}", a.path);
            println!("schema_version:    {}", meta.schema_version);
            println!("generator_version: {}", meta.generator_version);
            println!("shape:             {}", meta.shape.tag());
            println!("shape_params:      {}", serde_json::to_string(&meta.shape)?);
            println!("dtype:             {}", meta.dtype.as_str());
            println!("count:             {}", meta.count);
            println!("seed:              {}", meta.seed);
            print_stats(&meta.stats);
        }
        None => {
            // No sidecar: fall back to the raw i64 loader.
            use sketch_core::workload::{FileI64, Workload};
            let wk = FileI64::load(path)
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

fn print_stats(s: &datagen::BasicStats) {
    let fmt = |v: Option<f64>| v.map(|x| x.to_string()).unwrap_or_else(|| "-".into());
    println!("min:               {}", fmt(s.min));
    println!("max:               {}", fmt(s.max));
    println!("first:             {}", fmt(s.first));
    println!("last:              {}", fmt(s.last));
}
