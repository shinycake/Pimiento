#!/usr/bin/env bash
# scripts/cloud-agent-setup.sh — idempotent Cloud Agent bootstrap for Pimiento.
#
# Prepares a Linux (Ubuntu 24.04, x86_64) machine to build, test, and run the
# GPUI desktop client end-to-end against the real `omp` harness. Safe to re-run:
# every step is a no-op when already satisfied. Intended as the `install` command
# for a Cursor Cloud Agent environment, and usable standalone on a fresh Ubuntu
# image.
#
# It installs/configures, in order:
#   1. System libraries the GPUI/Zed stack links against (X11, Wayland,
#      fontconfig/freetype, Vulkan + the lavapipe software ICD for headless
#      rendering) plus the clang + mold fast-link toolchain.
#   2. The pinned Rust toolchains (stable = build; nightly = Polonius canary)
#      and rustfmt/clippy components.
#   3. cargo-nextest (the workspace test runner used by scripts/gate.sh).
#   4. The real `omp` (Oh My Pi) harness that Pimiento drives, authenticated to
#      Cursor via the CURSOR_API_KEY runtime secret and defaulted to the cheapest
#      Cursor model (composer-2.5). No secret value is baked into the image —
#      only static config plus a login-shell helper that exchanges the key for a
#      short-lived session token at runtime.
#   5. Warm the dependency graph with `cargo fetch` so the first real build is
#      compile-only (no network).
#
# The X server / DISPLAY used for GUI QA is provided by the Cloud Agent host,
# not by this script.

set -euo pipefail

log() { printf '\n==> %s\n' "$1"; }

SUDO=""
if [ "$(id -u)" -ne 0 ] && command -v sudo >/dev/null 2>&1; then
    SUDO="sudo"
fi

# --- 1. System libraries + fast-link toolchain -------------------------------
# GPUI on Linux links X11/XCB, Wayland, xkbcommon, fontconfig/freetype, GLES/EGL
# and renders via Vulkan (blade). mesa-vulkan-drivers ships lavapipe (llvmpipe),
# the software Vulkan ICD that lets the window render on a headless/virtual GPU.
if command -v apt-get >/dev/null 2>&1; then
    log "Installing system libraries (GPUI/Zed build + headless render deps)"
    export DEBIAN_FRONTEND=noninteractive
    $SUDO apt-get update -qq
    $SUDO apt-get install -y --no-install-recommends \
        build-essential cmake pkg-config clang mold curl git ca-certificates \
        libfontconfig-dev libfreetype-dev \
        libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
        libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
        libx11-dev libxext-dev libxi-dev \
        libgles2-mesa-dev libegl1-mesa-dev libgbm-dev \
        libvulkan-dev vulkan-tools mesa-vulkan-drivers \
        libasound2-dev libssl-dev zlib1g-dev
else
    log "apt-get not found — skipping system packages (assuming base image provides them)"
fi

# --- 2. Rust toolchains ------------------------------------------------------
if command -v rustup >/dev/null 2>&1; then
    log "Ensuring Rust toolchains (stable build + nightly Polonius canary)"
    # rust-toolchain.toml pins the stable channel + components; this materializes it.
    rustup show >/dev/null 2>&1 || true
    rustup toolchain install stable --profile minimal >/dev/null 2>&1 || true
    rustup component add --toolchain stable clippy rustfmt >/dev/null 2>&1 || true
    rustup toolchain install nightly --profile minimal --component clippy >/dev/null 2>&1 || true
else
    log "rustup not found — assuming stable + nightly toolchains are preinstalled"
fi

# --- 3. cargo-nextest (workspace test runner used by gate.sh) ----------------
if ! command -v cargo-nextest >/dev/null 2>&1; then
    log "Installing cargo-nextest"
    CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin"
    if command -v cargo >/dev/null 2>&1; then
        CARGO_BIN="$(dirname "$(command -v cargo)")"
    fi
    tmp="$(mktemp -d)"
    if curl -fsSL --retry 3 https://get.nexte.st/latest/linux -o "$tmp/nextest.tar.gz"; then
        if [ -w "$CARGO_BIN" ]; then
            tar xzf "$tmp/nextest.tar.gz" -C "$CARGO_BIN"
        else
            $SUDO tar xzf "$tmp/nextest.tar.gz" -C "$CARGO_BIN"
        fi
    else
        # Network-restricted fallback: build from crates.io.
        cargo install cargo-nextest --locked
    fi
    rm -rf "$tmp"
else
    log "cargo-nextest already present"
fi

# --- 4. omp (Oh My Pi) harness: install, Cursor auth, cheapest model ---------
# Pimiento is a front-end for the user's real `omp`. Every Cloud Agent must have
# omp installed, authenticated to Cursor via the CURSOR_API_KEY runtime secret,
# and defaulted to the cheapest Cursor model (composer-2.5). The API key is a raw
# dashboard key; omp's cursor provider needs a session JWT, so a login-shell
# helper exchanges the key for a short-lived token at runtime (see below). No
# secret value is baked into the image here — only static config + the helper.
if ! command -v omp >/dev/null 2>&1 && [ ! -x "$HOME/.local/bin/omp" ]; then
    log "Installing omp (Oh My Pi coding-agent harness)"
    curl -fsSL https://omp.sh/install | sh
else
    log "omp already installed"
fi

log "Configuring omp model roles -> cursor/composer-2.5 (cheapest Cursor model)"
mkdir -p "$HOME/.omp/agent"
cat > "$HOME/.omp/agent/config.yml" <<'YAML'
# Managed by scripts/cloud-agent-setup.sh.
# Cheapest Cursor model (Composer 2.5) for every role, per project requirement.
modelRoles:
  default: cursor/composer-2.5
  smol: cursor/composer-2.5
  tiny: cursor/composer-2.5
  task: cursor/composer-2.5
YAML

log "Installing Cursor session-token helper (CURSOR_API_KEY -> CURSOR_ACCESS_TOKEN)"
mkdir -p "$HOME/.local/share"
cat > "$HOME/.local/share/omp-cursor-auth.sh" <<'SH'
#!/bin/sh
# omp-cursor-auth.sh — resolve a fresh Cursor session token for omp at shell start.
#
# omp's "cursor" provider authenticates with a short-lived session JWT
# (CURSOR_ACCESS_TOKEN), not the raw dashboard API key (CURSOR_API_KEY,
# "crsr_..."). This helper exchanges the injected CURSOR_API_KEY runtime secret
# for a session token via Cursor's official exchange endpoint and exports it as
# CURSOR_ACCESS_TOKEN, caching it for ~45 min (JWTs live ~60 min). Only the
# short-lived token is written to disk (mode 600); the API key is never stored.
if [ -n "${CURSOR_API_KEY:-}" ] && [ -z "${CURSOR_ACCESS_TOKEN:-}" ]; then
  __omp_c="${XDG_CACHE_HOME:-$HOME/.cache}/omp-cursor-token"
  if [ -s "$__omp_c" ] && [ -z "$(find "$__omp_c" -mmin +45 2>/dev/null)" ]; then
    CURSOR_ACCESS_TOKEN="$(cat "$__omp_c" 2>/dev/null)"
  elif command -v curl >/dev/null 2>&1; then
    __omp_t="$(curl -fsS --max-time 20 -X POST \
      https://api2.cursor.sh/auth/exchange_user_api_key \
      -H "Authorization: Bearer $CURSOR_API_KEY" \
      -H 'Content-Type: application/json' -d '{}' 2>/dev/null \
      | sed -n 's/.*"accessToken":"\([^"]*\)".*/\1/p')"
    if [ -n "$__omp_t" ]; then
      mkdir -p "$(dirname "$__omp_c")" 2>/dev/null
      printf '%s' "$__omp_t" > "$__omp_c" 2>/dev/null
      chmod 600 "$__omp_c" 2>/dev/null
      CURSOR_ACCESS_TOKEN="$__omp_t"
    fi
    unset __omp_t
  fi
  [ -n "${CURSOR_ACCESS_TOKEN:-}" ] && export CURSOR_ACCESS_TOKEN
  unset __omp_c
fi
SH
chmod +x "$HOME/.local/share/omp-cursor-auth.sh"

# Wire ~/.local/bin onto PATH and source the auth helper from login + interactive
# shells (idempotent). Pimiento discovers omp through `$SHELL -lc`, so the login
# shell must both find omp and export a fresh CURSOR_ACCESS_TOKEN.
__omp_marker='# >>> pimiento omp cursor auth >>>'
for __rc in "$HOME/.profile" "$HOME/.bashrc"; do
    touch "$__rc"
    if ! grep -qF "$__omp_marker" "$__rc"; then
        cat >> "$__rc" <<'RC'

# >>> pimiento omp cursor auth >>>
case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) export PATH="$HOME/.local/bin:$PATH" ;; esac
[ -f "$HOME/.local/share/omp-cursor-auth.sh" ] && . "$HOME/.local/share/omp-cursor-auth.sh"
# <<< pimiento omp cursor auth <<<
RC
    fi
done
# If a ~/.bash_profile shadows ~/.profile for login shells, chain it once.
if [ -f "$HOME/.bash_profile" ] \
    && ! grep -qF "$__omp_marker" "$HOME/.bash_profile" \
    && ! grep -qE '\.profile' "$HOME/.bash_profile"; then
    printf '\n[ -f "$HOME/.profile" ] && . "$HOME/.profile"\n' >> "$HOME/.bash_profile"
fi

# --- 5. Warm the dependency graph -------------------------------------------
log "Fetching workspace dependencies (cargo fetch)"
cargo fetch --locked || cargo fetch

log "Cloud Agent setup complete: $(rustc --version), omp $("$HOME/.local/bin/omp" --version 2>/dev/null || omp --version 2>/dev/null || echo '(not found)')"
