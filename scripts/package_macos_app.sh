#!/usr/bin/env bash
# Build an unsigned macOS .app for local dogfood.
#
# Icon pipeline (Zed-style masters + Apple HIG):
#   assets/app-icon.png      — 512×512 full-bleed square master
#   assets/app-icon@2x.png   — 1024×1024 full-bleed square master
# Provide unmasked square artwork; the system applies the macOS squircle mask.
# Do not pre-bake rounded corners (HIG: App Icons → Icon shape).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_DIR="$ROOT/dist/Pimiento.app"
CONTENTS_DIR="$APP_DIR/Contents"
BINARY="$ROOT/target/release/pimiento-app"
ICON_1X="$ROOT/assets/app-icon.png"
ICON_2X="$ROOT/assets/app-icon@2x.png"
ICONSET_DIR="$ROOT/dist/Pimiento.iconset"
ICON_STEM="Pimiento"

if [[ ! -f "$ICON_1X" || ! -f "$ICON_2X" ]]; then
  echo "Missing icon masters. Expected:" >&2
  echo "  $ICON_1X" >&2
  echo "  $ICON_2X" >&2
  exit 1
fi

cargo build --manifest-path "$ROOT/Cargo.toml" -p pimiento-app --release

rm -rf "$APP_DIR" "$ICONSET_DIR"
mkdir -p "$CONTENTS_DIR/MacOS" "$CONTENTS_DIR/Resources" "$ICONSET_DIR"
install -m 755 "$BINARY" "$CONTENTS_DIR/MacOS/pimiento-app"

# Prefer the 1024 master for all rasters so downscales stay sharp (Lanczos via sips).
master="$ICON_2X"
sips -z 16 16 "$master" --out "$ICONSET_DIR/icon_16x16.png" >/dev/null
sips -z 32 32 "$master" --out "$ICONSET_DIR/icon_32x32.png" >/dev/null
cp "$ICONSET_DIR/icon_32x32.png" "$ICONSET_DIR/icon_16x16@2x.png"
sips -z 64 64 "$master" --out "$ICONSET_DIR/icon_32x32@2x.png" >/dev/null
sips -z 128 128 "$master" --out "$ICONSET_DIR/icon_128x128.png" >/dev/null
sips -z 256 256 "$master" --out "$ICONSET_DIR/icon_256x256.png" >/dev/null
cp "$ICONSET_DIR/icon_256x256.png" "$ICONSET_DIR/icon_128x128@2x.png"
sips -z 512 512 "$master" --out "$ICONSET_DIR/icon_512x512.png" >/dev/null
cp "$ICONSET_DIR/icon_512x512.png" "$ICONSET_DIR/icon_256x256@2x.png"
# 1024 — use the checked-in @2x master verbatim (no re-encode).
cp "$master" "$ICONSET_DIR/icon_512x512@2x.png"

iconutil --convert icns "$ICONSET_DIR" --output "$CONTENTS_DIR/Resources/${ICON_STEM}.icns"
rm -rf "$ICONSET_DIR"

# Also keep PNG masters beside the icns for tooling / future asset catalogs.
cp "$ICON_1X" "$CONTENTS_DIR/Resources/app-icon.png"
cp "$ICON_2X" "$CONTENTS_DIR/Resources/app-icon@2x.png"

VERSION="$(
  sed -n 's/^version *= *"\([^"]*\)".*/\1/p' "$ROOT/Cargo.toml" | head -1
)"
if [[ -z "$VERSION" ]]; then
  VERSION="0.0.0"
fi

cat >"$CONTENTS_DIR/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleDisplayName</key>
    <string>Pimiento</string>
    <key>CFBundleExecutable</key>
    <string>pimiento-app</string>
    <key>CFBundleIconFile</key>
    <string>${ICON_STEM}</string>
    <key>CFBundleIdentifier</key>
    <string>dev.pimiento.app</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>Pimiento</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.developer-tools</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
</dict>
</plist>
PLIST

# Flush Finder/Dock icon caches for this bundle path (best-effort).
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister \
  -f "$APP_DIR" >/dev/null 2>&1 || true

printf 'Packaged %s\n' "$APP_DIR"
printf 'Icon: %s/Contents/Resources/%s.icns\n' "$APP_DIR" "$ICON_STEM"
printf 'Open with: open "%s"\n' "$APP_DIR"
