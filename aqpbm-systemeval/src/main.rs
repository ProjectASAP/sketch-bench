mod query;
mod remote_write;
mod write;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use aqpbm_planeval::data::generate;
use aqpbm_planeval::score::read_answers;
use aqpbm_planeval::spec::Spec;
use clap::{Parser, ValueEnum};
use serde_json::json;

use crate::query::{ask, Collected};
use crate::write::{Options, Writer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum System {
    AsapqueryBackend,
    Victoriametrics,
}

impl System {
    fn name(self) -> &'static str {
        match self {
            System::AsapqueryBackend => "ASAPQuery-backend",
            System::Victoriametrics => "VictoriaMetrics",
        }
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "systemeval",
    about = "Write a spec's samples to a time series system by Prometheus Remote Write in time order and, at each \
             evaluation time, make the samples written so far visible and ask the queries due then"
)]
struct Args {
    #[arg(long)]
    spec: PathBuf,

    #[arg(long, value_enum)]
    system: System,

    #[arg(long, help = "Base URL of the system, e.g. http://127.0.0.1:8428")]
    url: String,

    #[arg(
        long,
        default_value_t = 10_000,
        help = "Most samples in one Remote Write request"
    )]
    batch_samples: usize,

    #[arg(
        long,
        help = "Replay at this many times the data's own rate; omitted writes as fast as the system accepts"
    )]
    speed: Option<f64>,

    #[arg(
        long,
        default_value_t = 1000,
        help = "With --speed, the most data time one request spans, in ms"
    )]
    flush_ms: i64,

    #[arg(
        long,
        help = "Output of an exact run of the same spec; each answer is then also scored against it"
    )]
    exact: Option<PathBuf>,

    #[arg(long = "query", help = "Only these query ids; repeatable")]
    queries: Vec<String>,

    #[arg(
        long,
        env = "ASAP_DATASETS",
        default_value = "datasets",
        help = "Directory that imported tables' dataset paths are relative to"
    )]
    data_root: PathBuf,

    #[arg(long)]
    out: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let spec = Spec::read(&args.spec)?;
    let generated = Instant::now();
    let series = generate(&spec, &args.data_root)?;
    eprintln!(
        "generated {} samples in {} series in {:.1} s",
        series.iter().map(|s| s.points.len()).sum::<usize>(),
        series.len(),
        generated.elapsed().as_secs_f64()
    );
    let exact = args.exact.as_deref().map(read_answers).transpose()?;
    let asked: Vec<_> = spec
        .queries
        .iter()
        .filter(|q| args.queries.is_empty() || args.queries.contains(&q.id))
        .collect();
    let mut due: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for (i, query) in asked.iter().enumerate() {
        for t in spec.evaluation_times(query) {
            due.entry(t).or_default().push(i);
        }
    }
    let options = Options {
        batch_samples: args.batch_samples,
        speed: args.speed,
        flush_ms: args.flush_ms,
    };
    let base = args.url.trim_end_matches('/');
    let mut writer = Writer::new(&series, args.system, base, spec.start_ms, &options);
    let agent = ureq::AgentBuilder::new().build();
    let mut collected: Vec<Collected> = asked.iter().map(|_| Collected::default()).collect();
    let started = Instant::now();
    writer.start_clock();
    for (&t, queries) in &due {
        writer.write_through(t)?;
        writer.make_visible(t)?;
        for &i in queries {
            collected[i].record(t, ask(&agent, base, &asked[i].promql, t));
        }
    }
    writer.write_through(spec.end_ms())?;
    writer.make_visible(spec.end_ms())?;
    let run_seconds = started.elapsed().as_secs_f64();
    let write = writer.summary()?;
    eprintln!(
        "{}: {} samples at {:.0} samples/s, {} evaluation times, {:.1} s",
        args.system.name(),
        writer.samples(),
        write["samples_per_second"].as_f64().unwrap_or_default(),
        due.len(),
        run_seconds
    );
    let mut queries = Vec::new();
    for (query, collected) in asked.iter().zip(collected) {
        let out = collected
            .finish(query, &series, exact.as_ref())
            .with_context(|| format!("answers of {}", query.id))?;
        eprintln!(
            "{}: {} evaluations, {} failed",
            query.id, out["evaluations"], out["failed_evaluations"]
        );
        queries.push(out);
    }
    let document = json!({
        "use_case": spec.use_case,
        "system": args.system.name(),
        "samples": writer.samples(),
        "run_seconds": run_seconds,
        "write": write,
        "queries": queries,
    });
    std::fs::write(&args.out, serde_json::to_string(&document)?)
        .with_context(|| format!("writing {}", args.out.display()))?;
    Ok(())
}
