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

use aqpbm_datagen::{self as datagen, Distribution, GenSpec, Shape, TimeUnit};

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
    /// Physical output type: i64 | u64 | f64.
    ///
    /// `bench --input` reads i64 files only. f64 is benchmarkable, but
    /// through `bench --dtype f64`, which generates in-process rather than
    /// reading a file. u64 has no consumer in this repo at all.
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

/// Normalise the `--dtype` string. `generate` is the one command with no
/// sketch to read an item type off, so it keeps a string here and turns it
/// into a type parameter at the single `match` in [`generate`].
fn parse_dtype(s: &str) -> Result<String> {
    match s.to_ascii_lowercase().as_str() {
        "i64" => Ok("i64".to_string()),
        "u64" => Ok("u64".to_string()),
        "f64" => Ok("f64".to_string()),
        "string" | "str" => Ok("string".to_string()),
        other => bail!("unknown dtype: {other} (expected i64|u64|f64|string)"),
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

/// Parse a `kind:param` gap distribution, e.g. `geometric:0.01`,
/// `const:1000`.
fn parse_gap(s: &str) -> Result<Distribution> {
    let (kind, param) = s
        .split_once(':')
        .ok_or_else(|| anyhow!("--gap must be `kind:param`, e.g. geometric:0.01"))?;
    let num = || -> Result<f64> {
        param
            .parse::<f64>()
            .map_err(|_| anyhow!("invalid gap param '{param}' for '{kind}'"))
    };
    match kind.to_ascii_lowercase().as_str() {
        "const" | "constant" => Ok(Distribution::Constant {
            value: param
                .parse::<u64>()
                .map_err(|_| anyhow!("invalid const step '{param}'"))?,
        }),
        "geometric" | "geo" => Ok(Distribution::Geometric { p: num()? }),
        "exp" | "exponential" => Ok(Distribution::Exponential { lambda: num()? }),
        "poisson" => Ok(Distribution::Poisson { lambda: num()? }),
        other => bail!("unknown gap kind: {other} (expected const|geometric|exp|poisson)"),
    }
}

/// Parse a categorical weight scheme: `uniform` or `zipf:s`. Explicit
/// weights are only available via `--spec`.
fn parse_weights(s: &str) -> Result<Distribution> {
    if s.eq_ignore_ascii_case("uniform") {
        return Ok(Distribution::Uniform);
    }
    if let Some(param) = s.strip_prefix("zipf:") {
        return Ok(Distribution::Zipf {
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
    let shape = match a.shape.as_str() {
        "uniform" => Shape::Keys {
            cardinality: a.cardinality,
            dist: Distribution::Uniform,
        },
        "zipf" => Shape::Keys {
            cardinality: a.cardinality,
            dist: Distribution::Zipf { s: a.zipf_s },
        },
        "monotonic-timestamp" | "timestamp" => {
            let gap = a
                .gap
                .as_deref()
                .ok_or_else(|| {
                    anyhow!("--gap is required for monotonic-timestamp (e.g. --gap geometric:0.01)")
                })
                .and_then(parse_gap)?;
            Shape::Monotonic {
                start: a.start,
                unit: parse_unit(&a.unit)?,
                gap,
                min_gap: a.min_gap,
            }
        }
        "skewed-categorical" | "categorical" => {
            let n = a
                .categories
                .ok_or_else(|| anyhow!("--categories <n> is required for skewed-categorical"))?;
            if n == 0 {
                bail!("--categories must be > 0");
            }
            Shape::Categorical {
                categories: (0..n as i64).collect(),
                dist: parse_weights(&a.weights)?,
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
        // No flags for the string options: the only sink that exists is
        // `.bin`, which cannot hold strings, so a `--alphabet` flag would
        // configure a path that always errors. `--spec` can already set
        // them for callers using the library.
        string: None,
    })
}

fn generate(a: GenerateArgs) -> Result<()> {
    let dtype = parse_dtype(&a.dtype)?;
    let spec = resolve_spec(&a)?;
    let out = Path::new(&a.out);
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating output dir {}", parent.display()))?;
        }
    }

    // The one place left in the tool where a string has to become a type
    // parameter. Everywhere else the item type is read off a catalog row's
    // `Accumulator::Item`; `generate` has no row, so it matches on the flag
    // and everything downstream of the match is monomorphic.
    fn stream<T: datagen::GenValue + datagen::FixedWidth>(
        spec: &datagen::GenSpec,
        out: &Path,
    ) -> Result<datagen::GenMeta> {
        // Stream through a BinSink: peak memory is one chunk, not the whole
        // dataset, so `--size` is bounded by disk rather than RAM.
        let mut sink = datagen::BinSink::<T>::create(out)
            .with_context(|| format!("creating {}", out.display()))?;
        spec.generate_into(&mut sink, datagen::DEFAULT_CHUNK)
            .with_context(|| format!("generating into {}", out.display()))
    }
    let meta = match dtype.as_str() {
        "i64" => stream::<i64>(&spec, out)?,
        "u64" => stream::<u64>(&spec, out)?,
        "f64" => stream::<f64>(&spec, out)?,
        // Not an oversight: `String` is not `FixedWidth`, so `stream::<String>`
        // would not compile. The `.bin` layout is a bare sequence of
        // equal-width values with nowhere to record a length. Strings
        // generate fine in-process; what is missing is a sink for them.
        "string" => bail!(
            "dtype string cannot be written to a .bin file: the format has no length field. \
             Strings are generated in-process today; a CSV sink is what would give them a file"
        ),
        other => bail!("unknown dtype: {other}"),
    };

    let sidecar = if a.no_meta {
        None
    } else {
        Some(datagen::io::write_meta(out, &meta).context("writing sidecar")?)
    };

    eprintln!(
        "sketchlib: generated shape={} dtype={} count={} -> {}",
        spec.shape.report_label(),
        meta.dtype.as_str(),
        meta.count,
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
            println!("shape:             {}", meta.shape.report_label());
            println!("shape_params:      {}", serde_json::to_string(&meta.shape)?);
            println!("dtype:             {}", meta.dtype.as_str());
            println!("count:             {}", meta.count);
            println!("seed:              {}", meta.seed);
            print_stats(&meta.stats);
        }
        None => {
            // No sidecar: fall back to the raw i64 loader.
            use aqpbm_core::workload::{I64Workload, Workload};
            let wk = I64Workload::load(path)
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
