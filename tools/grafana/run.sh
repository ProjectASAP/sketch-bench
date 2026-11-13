#!/usr/bin/env bash
set -euo pipefail

repo=$(cd "$(dirname "$0")/../.." && pwd)
data=${GRAFANA_DATA_DIR:-$repo/target/grafana-data}
state=${GRAFANA_STATE_DIR:-$repo/target/grafana-home}
plugin=yesoreyeram-infinity-datasource

if [ -n "${GRAFANA_HOME:-}" ]; then
  home=$GRAFANA_HOME
  grafana=$home/bin/grafana
elif command -v brew >/dev/null && [ -d "$(brew --prefix)/opt/grafana/share/grafana" ]; then
  home=$(brew --prefix)/opt/grafana/share/grafana
  grafana=$(brew --prefix)/opt/grafana/bin/grafana
else
  echo "grafana not found; on macOS run: brew install grafana; elsewhere unpack a Grafana release and set GRAFANA_HOME to its directory" >&2
  exit 1
fi
if [ ! -x "$grafana" ]; then
  echo "no grafana binary at $grafana" >&2
  exit 1
fi

mkdir -p "$state/data" "$state/plugins" "$state/logs" "$data"
if [ ! -d "$state/plugins/$plugin" ]; then
  "$grafana" cli --homepath "$home" --pluginsDir "$state/plugins" plugins install "$plugin"
fi
if [ "${1:-}" = "--check" ]; then
  exit 0
fi

python3 -m http.server 8000 --bind 127.0.0.1 --directory "$data" >/dev/null 2>&1 &
server=$!
trap 'kill $server 2>/dev/null' EXIT

echo "dashboards at http://localhost:3000 reading $data" >&2
GRAFANA_DASHBOARDS_DIR=$repo/tools/grafana/dashboards "$grafana" server \
  --homepath "$home" \
  cfg:default.paths.data="$state/data" \
  cfg:default.paths.logs="$state/logs" \
  cfg:default.paths.plugins="$state/plugins" \
  cfg:default.paths.provisioning="$repo/tools/grafana/provisioning" \
  cfg:server.http_addr=127.0.0.1 \
  cfg:auth.anonymous.enabled=true \
  cfg:auth.anonymous.org_role=Viewer
