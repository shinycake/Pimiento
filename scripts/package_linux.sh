#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT_DIR="$ROOT/dist/pimiento-linux"
STAGE_DIR="$OUT_DIR/pimiento"
BINARY="$ROOT/target/release/pimiento-app"

case "$(uname -m)" in
  x86_64) ARCH="x86_64" ;;
  aarch64 | arm64) ARCH="aarch64" ;;
  *) ARCH="$(uname -m)" ;;
esac

ARCHIVE="$OUT_DIR/pimiento-linux-$ARCH.tar.gz"

cargo build --manifest-path "$ROOT/Cargo.toml" -p pimiento-app --release

rm -rf "$OUT_DIR"
mkdir -p "$STAGE_DIR"
install -m 755 "$BINARY" "$STAGE_DIR/pimiento-app"

cat >"$STAGE_DIR/README.txt" <<'README'
Pimiento for Linux — personal dogfood archive

Requirements:
- A compatible Linux desktop with the runtime libraries used by this build.
- omp installed and available on your login-shell PATH.

Run:
  ./pimiento-app

Pimiento uses your existing omp installation, configuration, credentials, and
session store. It does not bundle or install omp.

This is a local dogfood archive, not an AppImage or .deb system package.
README

tar -C "$OUT_DIR" -czf "$ARCHIVE" pimiento

printf 'Packaged directory: %s\n' "$STAGE_DIR"
printf 'Packaged archive:   %s\n' "$ARCHIVE"
