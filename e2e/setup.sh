#!/usr/bin/env bash
# Fetch and build everything the E2E comparison runs, at the versions in
# pins.env: the Prometheus release tarball, the ASAPQuery-backend checkout
# (control_plane + data_plane), and the harness binary. Idempotent: a step
# whose output already exists is skipped.
set -euo pipefail

E2E_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$E2E_DIR/.." && pwd)"
# shellcheck source=pins.env
source "$E2E_DIR/pins.env"

CACHE_DIR="${E2E_CACHE_DIR:-$E2E_DIR/.cache}"
# One target dir for the harness and the backend, debuginfo and incremental
# off: a debug build of ASAPPlanner alone has filled this disk before.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_DIR/target}"
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_RELEASE_DEBUG=0
export CARGO_PROFILE_DEV_DEBUG=0
mkdir -p "$CACHE_DIR"

# --- Prometheus -------------------------------------------------------------
PROM_NAME="prometheus-${PROMETHEUS_VERSION}.linux-amd64"
PROM_BIN="$CACHE_DIR/$PROM_NAME/prometheus"
if [[ ! -x "$PROM_BIN" ]]; then
  tarball="$CACHE_DIR/$PROM_NAME.tar.gz"
  echo "setup: downloading Prometheus $PROMETHEUS_VERSION"
  curl -fsSL -o "$tarball" \
    "https://github.com/prometheus/prometheus/releases/download/v${PROMETHEUS_VERSION}/${PROM_NAME}.tar.gz"
  echo "${PROMETHEUS_SHA256}  $tarball" | sha256sum -c -
  tar -xzf "$tarball" -C "$CACHE_DIR"
  rm -f "$tarball"
fi
"$PROM_BIN" --version | head -1

# --- ASAPQuery-backend at the pinned commit ---------------------------------
BACKEND_DIR="$CACHE_DIR/ASAPQuery-backend"
if [[ ! -d "$BACKEND_DIR/.git" ]]; then
  echo "setup: fetching ASAPQuery-backend $BACKEND_COMMIT"
  git init -q "$BACKEND_DIR"
  git -C "$BACKEND_DIR" remote add origin "$BACKEND_REPO"
fi
if [[ "$(git -C "$BACKEND_DIR" rev-parse -q --verify HEAD || true)" != "$BACKEND_COMMIT" ]]; then
  git -C "$BACKEND_DIR" fetch -q --depth 1 origin "$BACKEND_COMMIT"
  git -C "$BACKEND_DIR" checkout -q --detach "$BACKEND_COMMIT"
fi
if ! grep -q "rev = \"$ASAPPLANNER_REV\"" "$BACKEND_DIR/Cargo.toml"; then
  echo "setup: $BACKEND_DIR/Cargo.toml does not pin ASAPPlanner $ASAPPLANNER_REV" >&2
  exit 1
fi
if grep -n 'ASAPPlanner' "$BACKEND_DIR/Cargo.toml" | grep -v "$ASAPPLANNER_REV" | grep -q 'rev ='; then
  echo "setup: $BACKEND_DIR/Cargo.toml pins a second ASAPPlanner rev" >&2
  exit 1
fi

echo "setup: building control_plane and data_plane (release) into $CARGO_TARGET_DIR"
cargo build --release --locked \
  --manifest-path "$BACKEND_DIR/Cargo.toml" -p control_plane -p data_plane

# --- Harness ------------------------------------------------------------------
echo "setup: building the harness (aqpbm-e2e)"
cargo build --release --locked --manifest-path "$REPO_DIR/Cargo.toml" -p aqpbm-e2e

df -h / | tail -1 | awk '{print "setup: free disk on /: " $4}'
