#!/usr/bin/env bash
# Build debug pimiento-app and replace any running dogfood instance.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
cargo build -p pimiento-app
PIMIENTO_RESTART=1 "$ROOT/scripts/run_app.sh"
