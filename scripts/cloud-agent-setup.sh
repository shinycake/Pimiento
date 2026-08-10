#!/usr/bin/env bash
# scripts/cloud-agent-setup.sh — idempotent Cloud Agent bootstrap for Pimiento.
#
# Prepares a Linux (Ubuntu 24.04, x86_64) machine to build, test, and run the
# GPUI desktop client end-to-end. Safe to re-run: every step is a no-op when
# already satisfied. Intended as the `install` command for a Cursor Cloud
# Agent environment, and usable standalone on a fresh Ubuntu image.
#
# It installs, in order:
#   1. System libraries the GPUI/Zed stack links against (X11, Wayland,
#      fontconfig/freetype, Vulkan + the lavapipe software ICD for headless
#      rendering) plus the clang + mold fast-link toolchain.
#   2. The pinned Rust toolchains (stable = build; nightly = Polonius canary)
#      and rustfmt/clippy components.
#   3. cargo-nextest (the workspace test runner used by scripts/gate.sh).
#   4. Warm the dependency graph with `cargo fetch` so the first real build is
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

# --- 4. Warm the dependency graph -------------------------------------------
log "Fetching workspace dependencies (cargo fetch)"
cargo fetch --locked || cargo fetch

log "Cloud Agent setup complete: $(rustc --version), $(cargo-nextest --version 2>/dev/null | head -1)"
