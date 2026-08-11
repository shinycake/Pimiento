#!/bin/sh
# scripts/gate.sh — Pimiento local quality gate.
#
# This is the sole quality gate before merging to `main`.
# Run this on every branch tip before merging.
#
# Blocking steps run in cost-ordered sequence (cheapest first):
#   1. cargo fmt --all --check
#   2. cargo clippy --workspace --all-targets -- -D warnings
#   3. cargo nextest run --workspace
#   4. cargo audit
#
# Then a non-blocking nightly Polonius canary:
#   5. cargo +nightly check --workspace         (result reported, never fails the gate)
#
# Requirements: Rust stable toolchain, cargo-nextest, cargo-audit.
#
# Environment:
#   RUSTC_WRAPPER   Honored automatically if already set (e.g. sccache). Never overridden.
#
# Exit status: the first failing blocking step. If all blocking steps pass, exit 0
# regardless of the canary outcome (the canary only prints PASS/WARN).

set -eu

say() {
    printf '\n==> %s\n' "$1"
}

if [ -n "${RUSTC_WRAPPER:-}" ]; then
    printf '(using RUSTC_WRAPPER=%s)\n' "$RUSTC_WRAPPER"
fi

# --- Blocking steps, in exact order ------------------------------------------

say "1/5  cargo fmt --all --check"
cargo fmt --all --check

say "2/5  cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

say "3/5  cargo nextest run --workspace"
cargo nextest run --workspace

say "4/5  cargo audit"
if ! command -v cargo-audit >/dev/null 2>&1; then
    printf 'cargo-audit not found. Install with:\n' >&2
    printf '  cargo install cargo-audit --locked\n' >&2
    exit 1
fi
cargo audit

# --- Non-blocking nightly Polonius canary ------------------------------------
#
# Guard against `set -e` so a canary failure never propagates. We capture the
# exit status explicitly and report PASS/WARN — the blocking gate is already
# green by the time we reach this line.

say "5/5  cargo +nightly check --workspace   (Polonius canary, non-blocking)"
canary_status=0
cargo +nightly check --workspace || canary_status=$?

if [ "$canary_status" -eq 0 ]; then
    printf '\nPolonius canary: PASS\n'
else
    printf '\nPolonius canary: WARN (exit %d) — non-blocking, gate still green\n' "$canary_status"
fi

printf '\nGate: OK\n'
