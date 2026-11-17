#!/usr/bin/env bash
# Start (or stop) one measured server for `approxbench bench-external` on
# this host: a docker container pinned to given cores, with the cgroup
# agent beside it.
#
#   scripts/extbench-up.sh clickhouse up   <data-dir> <cores> <numa-node> <http-port> <agent-port> [<name>]
#   scripts/extbench-up.sh clickhouse down <data-dir> [<name>]
#   scripts/extbench-up.sh check-listen-host <address>
#
# Example: LISTEN_HOST=10.0.0.2 scripts/extbench-up.sh clickhouse up /data/run-1/clickhouse 2,22 0 18123 19101
#
# Environment:
#   LISTEN_HOST        address the server and the agent listen on (required):
#                      a private (10/8, 172.16/12, 192.168/16) or loopback
#                      address, never 0.0.0.0 or a public one: the server
#                      has no password
#   CLICKHOUSE_BINARY  path of the ClickHouse binary to bind-mount (required)
#   CLICKHOUSE_SHA256  its expected sha256 (required); a mismatch refuses
#   BASE_IMAGE         the image the binary runs in, pinned by digest (required)
#   APPROXBENCH        path of the approxbench binary, for `cgroup-agent` (required)
#   AGENT_CORES        cores the agent may run on (default: every core); keep
#                      them off the server's cores
#
# The agent runs outside the container, so its reads are never charged to
# the server's group.
#
# Every start begins from an empty data dir. The script only deletes a data
# dir it created (it holds the marker file .extbench-data-dir). The
# container runs as the invoking user, with no CPU quota (--cpus unset) and
# no memory limit. ClickHouse runs on its embedded configuration: without a
# main config file it reads no config.d, so the few settings changed here
# are `--key=value` overrides. The embedded configuration also defines the
# native, MySQL, PostgreSQL and interserver ports (a port of 0 binds a random
# one), so they get the four ports after the HTTP port, on LISTEN_HOST too.
set -euo pipefail

marker=.extbench-data-dir

# Exit 0 when $1 is a private or loopback IPv4 address.
private_address() {
  local a b c d
  IFS=. read -r a b c d <<<"$1" || return 1
  for octet in "$a" "$b" "$c" "$d"; do
    [[ "$octet" =~ ^[0-9]+$ ]] && [ "$octet" -le 255 ] || return 1
  done
  [ "$a" = 10 ] && return 0
  [ "$a" = 127 ] && return 0
  [ "$a" = 172 ] && [ "$b" -ge 16 ] && [ "$b" -le 31 ] && return 0
  [ "$a" = 192 ] && [ "$b" = 168 ] && return 0
  return 1
}

if [ "${1:-}" = check-listen-host ]; then
  private_address "${2:?address}" || { echo "refused: ${2} is not a private address" >&2; exit 1; }
  exit 0
fi

engine=${1:?engine}; action=${2:?up|down}; data_dir=$(realpath -m "${3:?data dir}")

# Kill the agent recorded in $data_dir/agent.pid, if that pid is still it.
stop_agent() {
  local pid
  [ -f "$data_dir/agent.pid" ] || return 0
  pid=$(cat "$data_dir/agent.pid")
  if [ -r "/proc/$pid/cmdline" ] && tr '\0' ' ' < "/proc/$pid/cmdline" | grep -q cgroup-agent; then
    kill "$pid" 2>/dev/null || true
  fi
}
[ "$engine" = clickhouse ] || { echo "unknown engine $engine" >&2; exit 2; }

if [ "$action" = down ]; then
  name=${4:-extbench-clickhouse}
  stop_agent
  docker rm -f "$name" >/dev/null 2>&1 || true
  exit 0
fi

cores=${4:?cores}; numa=${5:?numa node}; http_port=${6:?http port}; agent_port=${7:?agent port}
name=${8:-extbench-clickhouse}
: "${LISTEN_HOST:?set LISTEN_HOST to a private address}" "${CLICKHOUSE_BINARY:?}" \
  "${CLICKHOUSE_SHA256:?}" "${BASE_IMAGE:?}" "${APPROXBENCH:?}"
private_address "$LISTEN_HOST" || { echo "refused: LISTEN_HOST $LISTEN_HOST is not a private address" >&2; exit 1; }
CLICKHOUSE_BINARY=$(realpath "$CLICKHOUSE_BINARY")
actual=$(sha256sum "$CLICKHOUSE_BINARY" | cut -d' ' -f1)
[ "$actual" = "$CLICKHOUSE_SHA256" ] || { echo "binary sha256 $actual != $CLICKHOUSE_SHA256" >&2; exit 1; }

if [ -e "$data_dir" ]; then
  [ -f "$data_dir/$marker" ] || { echo "refused: $data_dir exists and was not made by this script" >&2; exit 1; }
  stop_agent
  docker rm -f "$name" >/dev/null 2>&1 || true
  rm -rf "$data_dir"
fi
mkdir -p "$data_dir/db"; touch "$data_dir/$marker"

docker rm -f "$name" >/dev/null 2>&1 || true
docker run -d --name "$name" --network host \
  --cpuset-cpus "$cores" --cpuset-mems "$numa" --user "$(id -u):$(id -g)" \
  -v "$CLICKHOUSE_BINARY:/usr/bin/clickhouse:ro" -v "$data_dir/db:/var/lib/clickhouse" \
  -w /var/lib/clickhouse \
  "$BASE_IMAGE" /usr/bin/clickhouse server -- --path=/var/lib/clickhouse/ \
  --http_port="$http_port" --tcp_port=$((http_port + 1)) --mysql_port=$((http_port + 2)) \
  --postgresql_port=$((http_port + 3)) --interserver_http_port=$((http_port + 4)) \
  --listen_host="$LISTEN_HOST" --logger.level=warning --display_name="$name" >/dev/null
for _ in $(seq 1 60); do
  curl -sf "http://$LISTEN_HOST:$http_port/ping" >/dev/null && break; sleep 1
done
curl -sf "http://$LISTEN_HOST:$http_port/ping" >/dev/null || { echo "the server did not start" >&2; docker logs "$name" >&2; exit 1; }

id=$(docker inspect -f '{{.Id}}' "$name")
group="/sys/fs/cgroup/system.slice/docker-$id.scope"
[ -d "$group" ] || { echo "no cgroup at $group (is docker on the systemd cgroup driver?)" >&2; exit 1; }
pin=()
[ -n "${AGENT_CORES:-}" ] && pin=(taskset -c "$AGENT_CORES")
nohup "${pin[@]}" "$APPROXBENCH" cgroup-agent --cgroup-dir "$group" --listen "$LISTEN_HOST:$agent_port" \
  > "$data_dir/agent.log" 2>&1 & echo $! > "$data_dir/agent.pid"
for _ in $(seq 1 20); do
  curl -sf "http://$LISTEN_HOST:$agent_port/cpu.stat" >/dev/null && break; sleep 0.5
done
curl -sf "http://$LISTEN_HOST:$agent_port/cpu.stat" >/dev/null \
  || { echo "the cgroup agent did not start:" >&2; cat "$data_dir/agent.log" >&2; exit 1; }

query() { curl -sf "http://$LISTEN_HOST:$http_port/" --data-binary "$1"; }
siblings() {
  for core in ${cores//,/ }; do
    printf '%s:%s ' "$core" "$(cat "/sys/devices/system/cpu/cpu$core/topology/thread_siblings_list")"
  done
}
{ echo "binary_sha256 $actual"
  echo "version $(query "SELECT version()")"
  echo "git_hash $(query "SELECT value FROM system.build_options WHERE name = 'VERSION_GITHASH'")"
  echo "base_image $BASE_IMAGE"
  echo "cpuset $cores mems $numa"
  echo "thread_siblings $(siblings)"
  echo "cpu_model $(lscpu | sed -n 's/^Model name: *//p')"
  echo "cgroup $group"
  echo "agent_cores ${AGENT_CORES:-all}"
  echo "kernel $(uname -r)"; } > "$data_dir/versions.txt"
cat "$data_dir/versions.txt"
