//! `approxbench bench-external`: run one lane over a run directory, or join
//! every lane's cells into the report; and `approxbench cgroup-agent`, which
//! serves a server's cgroup v2 files to a harness on another host.
//!
//! Example: `approxbench bench-external run --run-dir R --lane fake`, then
//! `approxbench bench-external report --run-dir R --truth fake/v1`.

use std::net::TcpListener;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use aqpbm_engines_external::clickhouse_server::{ClickHouseEngine, ClickHouseServer};
use aqpbm_core::harness::bench_external::{report, run_lane, write_report, LaneSetup};
use aqpbm_core::harness::cgroup::{agent_in_served_group, serve_agent};
use aqpbm_core::harness::engine::EngineUnderTest;
use aqpbm_core::harness::fake::FakeEngine;
use aqpbm_core::harness::protocol::{
    load_probe_for, MeasurementClocks, OsCpuClock, ProcessCpuFiles, Protocol,
};
use aqpbm_core::harness::rundir::RunDir;
use clap::{Parser, Subcommand};

/// The lanes this binary can run.
pub const LANES: [&str; 2] = ["fake", "clickhouse"];

#[derive(Parser, Debug)]
pub struct BenchExternalArgs {
    #[command(subcommand)]
    pub command: BenchExternalCmd,
}

#[derive(Subcommand, Debug)]
pub enum BenchExternalCmd {
    /// Run every (workload, variant) cell of the run directory on one lane.
    Run(RunArgs),
    /// Join every lane's cells into summary.csv, summary.md and accuracy.csv.
    Report(ReportArgs),
}

#[derive(Parser, Debug)]
pub struct RunArgs {
    /// The run directory: workloads/, plans/<variant>/, results/.
    #[arg(long)]
    pub run_dir: PathBuf,
    /// The lane: `fake` (the harness's own fake engine) or `clickhouse`.
    #[arg(long)]
    pub lane: String,
    /// The ClickHouse server's HTTP interface, e.g. `http://10.0.0.1:8123`
    /// (lane `clickhouse`).
    #[arg(long)]
    pub clickhouse_url: Option<String>,
    /// The cgroup agent beside the server, e.g. `http://10.0.0.1:9101`
    /// (lane `clickhouse`).
    #[arg(long)]
    pub cgroup_agent_url: Option<String>,
    /// `max_threads` of every ClickHouse statement; 1 for the main figures.
    #[arg(long, default_value_t = 1)]
    pub clickhouse_max_threads: u32,
    /// How long one ClickHouse statement may run, in seconds.
    #[arg(long, default_value_t = 600)]
    pub clickhouse_statement_timeout_s: u64,
    /// Only these variants (repeatable); every variant under plans/ by default.
    #[arg(long = "variant")]
    pub variants: Vec<String>,
    /// The core the harness pins each step to.
    #[arg(long, default_value_t = 0)]
    pub core: usize,
    /// Runs discarded before measuring, per step.
    #[arg(long, default_value_t = Protocol::default().warmup_runs)]
    pub warmup: u32,
    /// Runs measured per step; odd.
    #[arg(long, default_value_t = Protocol::default().measured_runs)]
    pub runs: u32,
    /// A core busier than this fraction before a step refuses it.
    #[arg(long, default_value_t = Protocol::default().max_core_busy_fraction)]
    pub max_core_busy: f64,
}

#[derive(Parser, Debug)]
pub struct ReportArgs {
    #[arg(long)]
    pub run_dir: PathBuf,
    /// The cells answers are compared with: `<lane>/<variant>`.
    #[arg(long)]
    pub truth: String,
}

/// Serve a server's cgroup v2 files (cpu.stat, memory.current, memory.peak,
/// memory.stat, cpuset.cpus.effective) and the host's /proc/stat over HTTP
/// to a harness on another host.
#[derive(Parser, Debug)]
pub struct CgroupAgentArgs {
    /// The server's group, such as
    /// `/sys/fs/cgroup/system.slice/docker-<id>.scope`.
    #[arg(long)]
    pub cgroup_dir: PathBuf,
    /// Address to listen on, such as `0.0.0.0:9101`.
    #[arg(long)]
    pub listen: String,
}

pub fn run(args: BenchExternalArgs) -> Result<()> {
    match args.command {
        BenchExternalCmd::Run(args) => run_lane_cmd(args),
        BenchExternalCmd::Report(args) => report_cmd(args),
    }
}

fn run_lane_cmd(args: RunArgs) -> Result<()> {
    let engine: Box<dyn EngineUnderTest> = match args.lane.as_str() {
        "fake" => {
            if args.clickhouse_url.is_some() || args.cgroup_agent_url.is_some() {
                bail!("--clickhouse-url and --cgroup-agent-url are for lane clickhouse only");
            }
            Box::new(FakeEngine::default())
        }
        "clickhouse" => {
            let (Some(url), Some(cgroup_agent_url)) =
                (args.clickhouse_url.clone(), args.cgroup_agent_url.clone())
            else {
                bail!("lane clickhouse needs --clickhouse-url and --cgroup-agent-url");
            };
            Box::new(ClickHouseEngine::new(ClickHouseServer {
                url,
                cgroup_agent_url,
                max_threads: args.clickhouse_max_threads,
                statement_timeout: std::time::Duration::from_secs(
                    args.clickhouse_statement_timeout_s,
                ),
            })?)
        }
        other => bail!("unknown lane '{other}'; known lanes: {}", LANES.join(", ")),
    };
    let run_dir = RunDir::open(&args.run_dir)?;
    let protocol = Protocol {
        warmup_runs: args.warmup,
        measured_runs: args.runs,
        max_core_busy_fraction: args.max_core_busy,
    };
    let clocks = MeasurementClocks::with_calling_thread(&OsCpuClock);
    // The idle check watches the server's cores for an external server, the
    // harness core for an in-process engine.
    let mut probe = load_probe_for(&engine.cpu_accounting(), &ProcessCpuFiles::os())?;
    let only = (!args.variants.is_empty()).then_some(args.variants.as_slice());
    let cells = run_lane(
        &run_dir,
        engine.as_ref(),
        only,
        LaneSetup {
            protocol: &protocol,
            core: args.core,
            probe: probe.as_mut(),
            clocks: &clocks,
            args: std::env::args().collect(),
        },
    )?;
    for cell in cells {
        println!("{} {} {:?}", cell.workload, cell.variant, cell.outcome);
    }
    Ok(())
}

fn report_cmd(args: ReportArgs) -> Result<()> {
    let (lane, variant) = args
        .truth
        .split_once('/')
        .context("--truth must be <lane>/<variant>")?;
    let run_dir = RunDir::open(&args.run_dir)?;
    let report = report(&run_dir, lane, variant)?;
    write_report(&run_dir, &report)?;
    print!("{}", report.summary_markdown());
    Ok(())
}

pub fn run_cgroup_agent(args: CgroupAgentArgs) -> Result<()> {
    let own = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    if agent_in_served_group(&own, Path::new("/sys/fs/cgroup"), &args.cgroup_dir) {
        bail!(
            "this agent runs inside {}, the group it serves: its reads would be charged to the server",
            args.cgroup_dir.display()
        );
    }
    let listener =
        TcpListener::bind(&args.listen).with_context(|| format!("listening on {}", args.listen))?;
    serve_agent(listener, &args.cgroup_dir, Path::new("/proc/stat"))?;
    Ok(())
}
