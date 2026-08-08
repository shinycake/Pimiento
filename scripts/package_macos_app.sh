#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_DIR="$ROOT/dist/Pimiento.app"
CONTENTS_DIR="$APP_DIR/Contents"
BINARY="$ROOT/target/release/pimiento-app"
ICON_SOURCE="$ROOT/assets/pimiento-icon.svg"
ICONSET_DIR="$ROOT/dist/Pimiento.iconset"
ICON_FILE="Pimiento.icns"

cargo build --manifest-path "$ROOT/Cargo.toml" -p pimiento-app --release

rm -rf "$APP_DIR"
rm -rf "$ICONSET_DIR"
mkdir -p "$CONTENTS_DIR/MacOS" "$CONTENTS_DIR/Resources" "$ICONSET_DIR"
install -m 755 "$BINARY" "$CONTENTS_DIR/MacOS/pimiento-app"

sips -s format png -z 16 16 "$ICON_SOURCE" --out "$ICONSET_DIR/icon_16x16.png" >/dev/null
sips -s format png -z 32 32 "$ICON_SOURCE" --out "$ICONSET_DIR/icon_16x16@2x.png" >/dev/null
sips -s format png -z 32 32 "$ICON_SOURCE" --out "$ICONSET_DIR/icon_32x32.png" >/dev/null
sips -s format png -z 64 64 "$ICON_SOURCE" --out "$ICONSET_DIR/icon_32x32@2x.png" >/dev/null
sips -s format png -z 128 128 "$ICON_SOURCE" --out "$ICONSET_DIR/icon_128x128.png" >/dev/null
sips -s format png -z 256 256 "$ICON_SOURCE" --out "$ICONSET_DIR/icon_128x128@2x.png" >/dev/null
sips -s format png -z 256 256 "$ICON_SOURCE" --out "$ICONSET_DIR/icon_256x256.png" >/dev/null
sips -s format png -z 512 512 "$ICON_SOURCE" --out "$ICONSET_DIR/icon_256x256@2x.png" >/dev/null
sips -s format png -z 512 512 "$ICON_SOURCE" --out "$ICONSET_DIR/icon_512x512.png" >/dev/null
sips -s format png -z 1024 1024 "$ICON_SOURCE" --out "$ICONSET_DIR/icon_512x512@2x.png" >/dev/null
iconutil --convert icns "$ICONSET_DIR" --output "$CONTENTS_DIR/Resources/$ICON_FILE"
rm -rf "$ICONSET_DIR"

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
    <key>CFBundleIconFile</key>
    <string>Pimiento.icns</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
</dict>
</plist>
PLIST

printf 'Packaged %s\n' "$APP_DIR"
printf 'Open with: open "%s"\n' "$APP_DIR"
