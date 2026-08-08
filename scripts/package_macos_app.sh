#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_DIR="$ROOT/dist/Pimiento.app"
CONTENTS_DIR="$APP_DIR/Contents"
BINARY="$ROOT/target/release/pimiento-app"

cargo build --manifest-path "$ROOT/Cargo.toml" -p pimiento-app --release

rm -rf "$APP_DIR"
mkdir -p "$CONTENTS_DIR/MacOS"
install -m 755 "$BINARY" "$CONTENTS_DIR/MacOS/pimiento-app"

cat >"$CONTENTS_DIR/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Pimiento</string>
    <key>CFBundleIdentifier</key>
    <string>local.dev.pimiento</string>
    <key>CFBundleExecutable</key>
    <string>pimiento-app</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
</dict>
</plist>
PLIST

printf 'Packaged %s\n' "$APP_DIR"
printf 'Open with: open "%s"\n' "$APP_DIR"
