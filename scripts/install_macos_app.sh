#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOURCE_APP="$ROOT/dist/Pimiento.app"
INSTALL_DIR="${PIMIENTO_INSTALL_DIR:-$HOME/Applications}"
DESTINATION="$INSTALL_DIR/Pimiento.app"

"$ROOT/scripts/package_macos_app.sh"

mkdir -p "$INSTALL_DIR"
rm -rf "$DESTINATION"
cp -R "$SOURCE_APP" "$DESTINATION"

printf 'Installed %s\n' "$DESTINATION"
printf 'Open with: open "%s"\n' "$DESTINATION"
