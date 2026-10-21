//! Child processes the harness starts (Prometheus, the data plane), killed
//! when dropped, and the per-process resource numbers Linux exposes for them
//! under `/proc/<pid>`.

use std::fs::File;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use serde::Serialize;

/// A running child, killed and reaped on drop so no process outlives the run.
pub struct Managed {
    pub name: &'static str,
    child: Child,
}

impl Managed {
    /// Starts `command` with stdout and stderr appended to `log`.
    pub fn spawn(name: &'static str, mut command: Command, log: &Path) -> anyhow::Result<Self> {
        let out = File::create(log).with_context(|| format!("creating {}", log.display()))?;
        let err = out.try_clone()?;
        let child = command
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .spawn()
            .with_context(|| format!("starting {name}: {command:?}"))?;
        Ok(Self { name, child })
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Fails if the child has already exited, naming its status.
    pub fn check_alive(&mut self) -> anyhow::Result<()> {
        if let Some(status) = self.child.try_wait()? {
            bail!("{} exited early: {status}", self.name);
        }
        Ok(())
    }

    /// Polls `url` until it answers 200, failing if the child exits first or
    /// `timeout` passes.
    pub fn wait_ready(
        &mut self,
        client: &reqwest::blocking::Client,
        url: &str,
        timeout: Duration,
    ) -> anyhow::Result<Duration> {
        let start = Instant::now();
        loop {
            self.check_alive()?;
            if let Ok(response) = client.get(url).send() {
                if response.status().is_success() {
                    return Ok(start.elapsed());
                }
            }
            if start.elapsed() > timeout {
                bail!("{} not ready at {url} after {timeout:?}", self.name);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for Managed {
    fn drop(&mut self) {
        // SIGTERM first so Prometheus and the data plane shut down cleanly,
        // then SIGKILL whatever is left after a grace period.
        // SAFETY: `kill` with a pid we own and a valid signal number.
        unsafe { libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM) };
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A free local TCP port: bind port 0, read what the kernel picked, release it.
pub fn free_port() -> anyhow::Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// One reading of a process's memory and CPU from `/proc/<pid>`.
#[derive(Debug, Clone, Serialize)]
pub struct ProcSample {
    /// Resident set size now (`VmRSS`), kB.
    pub rss_kb: Option<u64>,
    /// Peak resident set size since start (`VmHWM`), kB.
    pub rss_peak_kb: Option<u64>,
    /// User + system CPU time since start (`/proc/<pid>/stat` utime+stime), s.
    pub cpu_seconds: Option<f64>,
}

/// Reads `pid`'s current RSS, peak RSS and CPU time. Fields that cannot be
/// read are `None`, never guessed.
pub fn sample(pid: u32) -> ProcSample {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    let field = |key: &str| {
        status.lines().find_map(|line| {
            line.strip_prefix(key)
                .and_then(|rest| rest.split_whitespace().next())
                .and_then(|kb| kb.parse::<u64>().ok())
        })
    };
    ProcSample {
        rss_kb: field("VmRSS:"),
        rss_peak_kb: field("VmHWM:"),
        cpu_seconds: cpu_seconds(pid),
    }
}

fn cpu_seconds(pid: u32) -> Option<f64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Fields after the parenthesised command name; utime and stime are the
    // 14th and 15th fields overall, i.e. the 12th and 13th after `)`.
    let rest = &stat[stat.rfind(')')? + 2..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let utime: f64 = fields.get(11)?.parse().ok()?;
    let stime: f64 = fields.get(12)?.parse().ok()?;
    // SAFETY: sysconf with a valid name has no preconditions.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    (ticks > 0).then(|| (utime + stime) / ticks as f64)
}
