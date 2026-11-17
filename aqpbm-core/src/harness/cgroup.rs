//! CPU and memory of an external server that runs in its own cgroup v2
//! group, such as a docker container's
//! `/sys/fs/cgroup/system.slice/docker-<id>.scope`.
//!
//! The group's files are read either from a local directory or, when the
//! server runs on another host than the harness, through a small agent on
//! the server host ([`serve_agent`]) that answers `GET /<file>` with the
//! file's current text. Only [`CGROUP_FILES`] and the host's `/proc/stat`
//! ([`PROC_STAT`]) are served.
//!
//! The agent can also hold one server config file ([`SERVER_CONFIG`]), the
//! one file a harness on another host may replace, e.g. a VictoriaMetrics
//! `-streamAggr.config` that each install rewrites before reloading the
//! server.
//!
//! Example: `cpu.stat` with `usage_usec 2500` reads 2,500,000 ns; a group
//! whose `nr_throttled` grows during a step was throttled by a CPU quota,
//! and that step is not a sample.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The group files the agent serves: CPU, current and peak memory, memory
/// breakdown, and the cores the group may run on.
pub const CGROUP_FILES: [&str; 5] = [
    "cpu.stat",
    "memory.current",
    "memory.peak",
    "memory.stat",
    "cpuset.cpus.effective",
];

/// The name under which the agent serves its host's `/proc/stat`, so the
/// harness can check the server's cores are idle around a step.
pub const PROC_STAT: &str = "proc-stat";

/// The name under which the agent serves, and replaces on `PUT`, the
/// server config file it was started with (`--server-config`).
pub const SERVER_CONFIG: &str = "server-config";

/// The largest server config the agent accepts, in bytes.
pub const MAX_SERVER_CONFIG_BYTES: usize = 1024 * 1024;

/// Where a server's cgroup v2 files are read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CgroupSource {
    /// The group's directory on this host.
    Local(PathBuf),
    /// An agent on the server host, such as `http://10.0.0.2:9101`.
    Agent { url: String },
}

impl CgroupSource {
    /// The current text of one of [`CGROUP_FILES`], or of the server
    /// host's `/proc/stat` for [`PROC_STAT`].
    pub fn read(&self, file: &str) -> Result<String, CgroupError> {
        if !CGROUP_FILES.contains(&file) && file != PROC_STAT && file != SERVER_CONFIG {
            return Err(CgroupError::NotServed(file.into()));
        }
        match self {
            CgroupSource::Local(_) if file == SERVER_CONFIG => {
                Err(CgroupError::NotServed(file.into()))
            }
            CgroupSource::Local(dir) => {
                let path = if file == PROC_STAT {
                    PathBuf::from("/proc/stat")
                } else {
                    dir.join(file)
                };
                std::fs::read_to_string(&path).map_err(|error| {
                    if error.kind() == std::io::ErrorKind::NotFound {
                        CgroupError::NotFound(path.display().to_string())
                    } else {
                        CgroupError::Read {
                            file: path.display().to_string(),
                            message: error.to_string(),
                        }
                    }
                })
            }
            CgroupSource::Agent { url } => http_request(url, "GET", file, None),
        }
    }

    /// `cpu.stat` read once.
    pub fn cpu(&self) -> Result<CgroupCpu, CgroupError> {
        CgroupCpu::parse(&self.read("cpu.stat")?)
    }

    /// The memory files read once; `memory.peak` is absent before Linux 5.19.
    pub fn memory(&self) -> Result<CgroupMemory, CgroupError> {
        let current_bytes = parse_number("memory.current", self.read("memory.current")?.trim())?;
        let lifetime_peak_bytes = match self.read("memory.peak") {
            Ok(text) => Some(parse_number("memory.peak", text.trim())?),
            Err(CgroupError::NotFound(_)) => None,
            Err(error) => return Err(error),
        };
        let stat = self.read("memory.stat")?;
        Ok(CgroupMemory {
            current_bytes,
            anon_bytes: stat_field("memory.stat", &stat, "anon")?,
            file_bytes: stat_field("memory.stat", &stat, "file")?,
            lifetime_peak_bytes,
        })
    }
}

/// The CPU counters of `cpu.stat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CgroupCpu {
    /// `usage_usec` in ns: CPU of every thread and child of the group.
    pub usage_ns: u64,
    /// `nr_throttled`: periods a CPU quota stopped the group (0 without a
    /// `--cpus` quota; absent when the cpu controller is off, read as 0).
    pub nr_throttled: u64,
}

impl CgroupCpu {
    pub fn parse(cpu_stat: &str) -> Result<CgroupCpu, CgroupError> {
        let usage_usec = stat_field("cpu.stat", cpu_stat, "usage_usec")?;
        let nr_throttled = match stat_field("cpu.stat", cpu_stat, "nr_throttled") {
            Ok(value) => value,
            Err(CgroupError::MissingField { .. }) => 0,
            Err(error) => return Err(error),
        };
        Ok(CgroupCpu {
            usage_ns: usage_usec * 1_000,
            nr_throttled,
        })
    }
}

/// A group's memory, read at one moment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CgroupMemory {
    /// `memory.current`: every page the group holds, page cache included.
    pub current_bytes: u64,
    /// `memory.stat` `anon`: heap and stacks, without page cache.
    pub anon_bytes: u64,
    /// `memory.stat` `file`: page cache.
    pub file_bytes: u64,
    /// `memory.peak`: the most `memory.current` has been since the group
    /// started. Before Linux 6.12 it cannot be reset, so it spans the
    /// container's whole life, every earlier arm included. `None` before 5.19.
    pub lifetime_peak_bytes: Option<u64>,
}

fn parse_number(file: &str, text: &str) -> Result<u64, CgroupError> {
    text.parse().map_err(|_| CgroupError::Parse {
        file: file.into(),
        message: format!("'{text}' is not a number"),
    })
}

/// The value of the `<field> <value>` line of a flat-keyed cgroup file.
fn stat_field(file: &str, text: &str, field: &str) -> Result<u64, CgroupError> {
    let value = text
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(' ')?;
            (key == field).then_some(value)
        })
        .ok_or_else(|| CgroupError::MissingField {
            file: file.into(),
            field: field.into(),
        })?;
    parse_number(file, value.trim())
}

/// Replaces the server config the agent at `agent_url` holds with `text`,
/// e.g. a new `-streamAggr.config` before the server reloads it.
pub fn put_server_config(agent_url: &str, text: &str) -> Result<(), CgroupError> {
    http_request(agent_url, "PUT", SERVER_CONFIG, Some(text)).map(|_| ())
}

/// `<method> <url>/<file>` over HTTP/1.0 with an optional body; the
/// answer's body on status 200 or 204.
fn http_request(
    url: &str,
    method: &str,
    file: &str,
    body: Option<&str>,
) -> Result<String, CgroupError> {
    let failed = |message: String| CgroupError::Agent {
        url: format!("{url}/{file}"),
        message,
    };
    let address = url
        .strip_prefix("http://")
        .ok_or_else(|| failed("the agent url must start with http://".into()))?
        .trim_end_matches('/');
    let socket = address
        .to_socket_addrs()
        .map_err(|error| failed(error.to_string()))?
        .next()
        .ok_or_else(|| failed("the agent address resolves to nothing".into()))?;
    let mut stream = TcpStream::connect_timeout(&socket, AGENT_CONNECT_TIMEOUT)
        .map_err(|error| failed(error.to_string()))?;
    stream
        .set_read_timeout(Some(AGENT_IO_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(AGENT_IO_TIMEOUT)))
        .map_err(|error| failed(error.to_string()))?;
    let sent = match body {
        None => write!(
            stream,
            "{method} /{file} HTTP/1.0\r\nHost: {address}\r\n\r\n"
        ),
        Some(body) => write!(
            stream,
            "{method} /{file} HTTP/1.0\r\nHost: {address}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ),
    };
    sent.map_err(|error| failed(error.to_string()))?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| failed(error.to_string()))?;
    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| failed("no header end in the response".into()))?;
    let status = head.lines().next().unwrap_or("");
    match status.split_whitespace().nth(1) {
        Some("200") => {}
        Some("204") => return Ok(String::new()),
        // The client asks only for served names, so a 404 is a missing file,
        // such as `memory.peak` before Linux 5.19.
        Some("404") => return Err(CgroupError::NotFound(format!("{url}/{file}"))),
        _ => return Err(failed(format!("status '{status}'"))),
    }
    let content_length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .ok_or_else(|| failed("no Content-Length".into()))?;
    if body.len() != content_length {
        return Err(failed(format!(
            "body of {} bytes, Content-Length {content_length}: truncated",
            body.len()
        )));
    }
    Ok(body.to_string())
}

/// Whether a process whose `/proc/self/cgroup` reads `proc_self_cgroup`
/// runs inside the group at `served_dir` under the cgroup v2 mount
/// `cgroup_root`. The agent refuses to serve its own group: its reads would
/// be charged to the server it measures.
/// Example: `0::/system.slice/docker-1a2b.scope` with root `/sys/fs/cgroup`
/// is inside `/sys/fs/cgroup/system.slice/docker-1a2b.scope`.
pub fn agent_in_served_group(
    proc_self_cgroup: &str,
    cgroup_root: &Path,
    served_dir: &Path,
) -> bool {
    let Some(own) = proc_self_cgroup
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
    else {
        return false;
    };
    let own_dir = cgroup_root.join(own.trim().trim_start_matches('/'));
    own_dir.starts_with(served_dir)
}

/// Serves `GET /<file>` for each of [`CGROUP_FILES`] in `dir`, and
/// `GET /proc-stat` from `proc_stat` (the host's `/proc/stat`), read afresh
/// per request. With `server_config`, also `GET /server-config` and
/// `PUT /server-config`, which replaces that file atomically (a temporary
/// file in its directory, then a rename) with a body of at most
/// [`MAX_SERVER_CONFIG_BYTES`]. Every other path is 404. Runs until the
/// listener fails. One request per connection (HTTP/1.0), answered on the
/// accepting thread.
pub fn serve_agent(
    listener: TcpListener,
    dir: &Path,
    proc_stat: &Path,
    server_config: Option<&Path>,
) -> std::io::Result<()> {
    for stream in listener.incoming() {
        let mut stream = stream?;
        // A broken client never stops the agent.
        let _ = answer(&mut stream, dir, proc_stat, server_config);
    }
    Ok(())
}

fn answer(
    stream: &mut TcpStream,
    dir: &Path,
    proc_stat: &Path,
    server_config: Option<&Path>,
) -> std::io::Result<()> {
    // A client that connects and sends nothing must not stall the agent,
    // which answers one connection at a time.
    stream.set_read_timeout(Some(AGENT_REQUEST_TIMEOUT))?;
    stream.set_write_timeout(Some(AGENT_IO_TIMEOUT))?;
    let mut reader = BufReader::new(&*stream);
    let mut head = (&mut reader).take(MAX_REQUEST_BYTES);
    let mut request_line = String::new();
    head.read_line(&mut request_line)?;
    // Read the headers up to the blank line: closing a socket with unread
    // input resets the connection, and the client loses the answer.
    let mut content_length: Option<usize> = None;
    let mut header = String::new();
    while head.read_line(&mut header)? > 0 && header != "\r\n" && header != "\n" {
        if let Some((name, value)) = header.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().ok();
            }
        }
        header.clear();
    }
    let mut words = request_line.split_whitespace();
    let method = words.next().unwrap_or("");
    let file = words.next().unwrap_or("").strip_prefix('/').unwrap_or("");
    if method == "PUT" {
        // A refused body is still read (up to the size limit), so the client
        // gets the answer instead of a reset connection.
        fn drain(reader: &mut impl Read, length: usize) {
            let limit = length.min(MAX_SERVER_CONFIG_BYTES + 1) as u64;
            let _ = std::io::copy(&mut reader.take(limit), &mut std::io::sink());
        }
        let status = match (
            server_config.filter(|_| file == SERVER_CONFIG),
            content_length,
        ) {
            (None, length) => {
                drain(&mut reader, length.unwrap_or(0));
                "404 Not Found"
            }
            (Some(_), None) => "411 Length Required",
            (Some(_), Some(length)) if length > MAX_SERVER_CONFIG_BYTES => {
                drain(&mut reader, length);
                "413 Content Too Large"
            }
            (Some(path), Some(length)) => {
                let mut body = Vec::with_capacity(length);
                (&mut reader).take(length as u64).read_to_end(&mut body)?;
                if body.len() == length && replace_file(path, &body).is_ok() {
                    "204 No Content"
                } else {
                    "400 Bad Request"
                }
            }
        };
        return write!(stream, "HTTP/1.0 {status}\r\nContent-Length: 0\r\n\r\n");
    }
    let body = if CGROUP_FILES.contains(&file) {
        std::fs::read_to_string(dir.join(file)).ok()
    } else if file == PROC_STAT {
        std::fs::read_to_string(proc_stat).ok()
    } else if file == SERVER_CONFIG {
        server_config.and_then(|path| std::fs::read_to_string(path).ok())
    } else {
        None
    };
    match body {
        Some(body) => write!(
            stream,
            "HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ),
        None => write!(
            stream,
            "HTTP/1.0 404 Not Found\r\nContent-Length: 0\r\n\r\n"
        ),
    }
}

/// Writes `contents` to `path` through a temporary file in its directory and
/// a rename, so a reader never sees half a file.
fn replace_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("the server config path has no file name"))?;
    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(name);
    temporary_name.push(".partial");
    let temporary = path.with_file_name(temporary_name);
    std::fs::write(&temporary, contents)?;
    std::fs::rename(&temporary, path)
}

/// How long the client waits to connect to the agent.
const AGENT_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
/// How long the agent waits for a request: shorter than the client's wait,
/// so an idle connection delays a waiting client by at most this long.
const AGENT_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
/// How long either side waits on one read or write.
const AGENT_IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// The most of a request the agent reads: a request line and a few headers.
const MAX_REQUEST_BYTES: u64 = 8192;

/// A cgroup file that cannot be read or parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CgroupError {
    #[error("cannot read {file}: {message}")]
    Read { file: String, message: String },
    #[error("{file} has no '{field}' line")]
    MissingField { file: String, field: String },
    #[error("cannot parse {file}: {message}")]
    Parse { file: String, message: String },
    #[error("cgroup agent {url}: {message}")]
    Agent { url: String, message: String },
    /// The file does not exist, such as `memory.peak` before Linux 5.19.
    #[error("{0} does not exist")]
    NotFound(String),
    #[error("{0} is not one of the served cgroup files")]
    NotServed(String),
}
