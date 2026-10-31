#!/usr/bin/env bash
set -euo pipefail

repo=$(cd "$(dirname "$0")/../.." && pwd)
data=${GRAFANA_DATA_DIR:-$repo/target/grafana-data}
prefix=$(brew --prefix)
home=$prefix/opt/grafana/share/grafana
plugins=$prefix/var/lib/grafana/plugins

if ! command -v grafana >/dev/null; then
  echo "grafana not found; run: brew install grafana" >&2
  exit 1
fi
if [ ! -d "$plugins/yesoreyeram-infinity-datasource" ]; then
  echo "Infinity plugin missing; run: grafana cli --homepath $home --pluginsDir $plugins plugins install yesoreyeram-infinity-datasource" >&2
  exit 1
fi

mkdir -p "$data"
python3 -m http.server 8000 --bind 127.0.0.1 --directory "$data" >/dev/null 2>&1 &
server=$!
trap 'kill $server 2>/dev/null' EXIT

GRAFANA_DASHBOARDS_DIR=$repo/tools/grafana/dashboards grafana server \
  --config "$prefix/etc/grafana/grafana.ini" \
  --homepath "$home" \
  --packaging=brew \
  cfg:default.paths.logs="$prefix/var/log/grafana" \
  cfg:default.paths.data="$prefix/var/lib/grafana" \
  cfg:default.paths.plugins="$plugins" \
  cfg:default.paths.provisioning="$repo/tools/grafana/provisioning" \
  cfg:server.http_addr=127.0.0.1 \
  cfg:auth.anonymous.enabled=true \
  cfg:auth.anonymous.org_role=Viewer
