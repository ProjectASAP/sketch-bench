#!/usr/bin/env bash
# Start (or stop) a single-node VictoriaMetrics server for
# `approxbench bench-external --lane victoriametrics` on this host: a docker
# container pinned to given cores, with the cgroup agent beside it. The
# agent also holds the server's stream-aggregation config, which the lane
# replaces (PUT /server-config) and reloads for each arm.
#
#   scripts/extbench-up-victoriametrics.sh up   <data-dir> <cores> <numa-node> <http-port> <agent-port> [<name>]
#   scripts/extbench-up-victoriametrics.sh down <data-dir> [<name>]
#
# Example: LISTEN_HOST=10.0.0.2 APPROXBENCH=/opt/approxbench \
#   scripts/extbench-up-victoriametrics.sh up /data/run-1/victoriametrics 2,22 0 18428 19101
#
# Environment:
#   LISTEN_HOST   address the server and the agent listen on (required): a
#                 private (10/8, 172.16/12, 192.168/16) or loopback address,
#                 never 0.0.0.0 or a public one: the server has no password
#   APPROXBENCH   path of the approxbench binary, for `cgroup-agent` (required)
#   IMAGE         the image, pinned by digest (default: v1.154.1 below)
#
# Every start begins from an empty data dir. The script only deletes a data
# dir it created (it holds the marker file .extbench-data-dir). The
# container runs as the invoking user, with no CPU quota and no memory limit.
set -euo pipefail

marker=.extbench-data-dir
default_image=victoriametrics/victoria-metrics@sha256:c7884f32920dd45a9d86542a717227e63af03e060e53bbf8031e6e6fd0e1e05d

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

action=${1:?up|down}; data_dir=${2:?data dir}

if [ "$action" = down ]; then
  name=${3:-extbench-victoriametrics}
  [ -f "$data_dir/agent.pid" ] && kill "$(cat "$data_dir/agent.pid")" 2>/dev/null || true
  docker rm -f "$name" >/dev/null 2>&1 || true
  exit 0
fi
[ "$action" = up ] || { echo "unknown action $action" >&2; exit 2; }

cores=${3:?cores}; numa=${4:?numa node}; http_port=${5:?http port}; agent_port=${6:?agent port}
name=${7:-extbench-victoriametrics}
image=${IMAGE:-$default_image}
: "${LISTEN_HOST:?set LISTEN_HOST to a private address}" "${APPROXBENCH:?}"
private_address "$LISTEN_HOST" || { echo "refused: LISTEN_HOST $LISTEN_HOST is not a private address" >&2; exit 1; }
[[ "$image" == *@sha256:* ]] || { echo "refused: IMAGE must be pinned by digest" >&2; exit 1; }

if [ -e "$data_dir" ]; then
  [ -f "$data_dir/$marker" ] || { echo "refused: $data_dir exists and was not made by this script" >&2; exit 1; }
  [ -f "$data_dir/agent.pid" ] && kill "$(cat "$data_dir/agent.pid")" 2>/dev/null || true
  docker rm -f "$name" >/dev/null 2>&1 || true
  rm -rf "$data_dir"
fi
mkdir -p "$data_dir/storage" "$data_dir/config"; touch "$data_dir/$marker"
# The config directory is bind-mounted (not the file), so the agent's
# temporary-file-and-rename replacement is seen by the server.
echo "[]" > "$data_dir/config/stream-aggregation.yaml"

docker rm -f "$name" >/dev/null 2>&1 || true
docker run -d --name "$name" --network host \
  --cpuset-cpus "$cores" --cpuset-mems "$numa" --user "$(id -u):$(id -g)" \
  -v "$data_dir/storage:/storage" -v "$data_dir/config:/config:ro" \
  "$image" \
  -httpListenAddr="$LISTEN_HOST:$http_port" -storageDataPath=/storage \
  -streamAggr.config=/config/stream-aggregation.yaml -streamAggr.keepInput=true \
  -search.latencyOffset=0 -search.disableCache=true -retentionPeriod=100y \
  -loggerLevel=WARN >/dev/null
for _ in $(seq 1 60); do
  curl -sf "http://$LISTEN_HOST:$http_port/health" >/dev/null && break; sleep 1
done
curl -sf "http://$LISTEN_HOST:$http_port/health" >/dev/null || { echo "the server did not start" >&2; docker logs "$name" >&2; exit 1; }

id=$(docker inspect -f '{{.Id}}' "$name")
group="/sys/fs/cgroup/system.slice/docker-$id.scope"
[ -d "$group" ] || { echo "no cgroup at $group (is docker on the systemd cgroup driver?)" >&2; exit 1; }
nohup "$APPROXBENCH" cgroup-agent --cgroup-dir "$group" --listen "$LISTEN_HOST:$agent_port" \
  --server-config "$data_dir/config/stream-aggregation.yaml" \
  > "$data_dir/agent.log" 2>&1 & echo $! > "$data_dir/agent.pid"

siblings() {
  for core in ${cores//,/ }; do
    printf '%s:%s ' "$core" "$(cat "/sys/devices/system/cpu/cpu$core/topology/thread_siblings_list")"
  done
}
{ echo "image $image"
  echo "image_id $(docker inspect -f '{{.Image}}' "$name")"
  echo "version $(curl -sf "http://$LISTEN_HOST:$http_port/metrics" | sed -n 's/^vm_app_version{version="\([^"]*\)".*/\1/p')"
  echo "cpuset $cores mems $numa"
  echo "thread_siblings $(siblings)"
  echo "cpu_model $(lscpu | sed -n 's/^Model name: *//p')"
  echo "cgroup $group"
  echo "kernel $(uname -r)"; } > "$data_dir/versions.txt"
cat "$data_dir/versions.txt"
