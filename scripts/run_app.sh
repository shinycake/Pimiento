#!/usr/bin/env bash
# scripts/run_app.sh — daemonize pimiento-app so Cursor shell exit cannot SIGHUP it.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PIDFILE="${PIMIENTO_PIDFILE:-/tmp/pimiento-app.pid}"
LOG="${PIMIENTO_LOG:-/tmp/pimiento-app.log}"
BIN="${PIMIENTO_BIN:-$ROOT/target/debug/pimiento-app}"

if [[ ! -x "$BIN" ]]; then
  echo "missing binary: $BIN (run: cargo build -p pimiento-app)" >&2
  exit 1
fi

bring_to_front() {
  local pid="$1"
  [[ -n "$pid" ]] || return 0
  case "$(uname -s)" in
    Darwin)
      # Double-fork + setsid leaves the process as a background agent on macOS,
      # so the window often opens behind Cursor. Explicitly raise it.
      osascript -e \
        "tell application \"System Events\" to set frontmost of (first process whose unix id is ${pid}) to true" \
        >/dev/null 2>&1 || true
      ;;
  esac
}

if [[ "${PIMIENTO_RESTART:-}" == "1" ]]; then
  if [[ -f "$PIDFILE" ]] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
    kill "$(cat "$PIDFILE")" 2>/dev/null || true
    sleep 0.3
  fi
  pkill -f '/target/debug/pimiento-app' 2>/dev/null || true
  sleep 0.2
  rm -f "$PIDFILE"
fi

if [[ -f "$PIDFILE" ]] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
  pid="$(cat "$PIDFILE")"
  echo "already running pid=${pid}"
  bring_to_front "$pid"
  exit 0
fi

python3 - "$BIN" "$PIDFILE" "$LOG" "${PIMIENTO_CWD:-$ROOT}" <<'PY'
import os, sys, time
from pathlib import Path

bin_path, pidfile, log_path, cwd = sys.argv[1:5]
Path(log_path).write_text("")
if os.fork() > 0:
    time.sleep(0.6)
    print(Path(pidfile).read_text().strip())
    raise SystemExit(0)
os.setsid()
if os.fork() > 0:
    os._exit(0)
os.chdir(cwd)
os.environ["PIMIENTO_CWD"] = cwd
os.environ.setdefault("RUST_BACKTRACE", "1")
# Auto-connect so GUI driving does not depend on AX/Start-here clicks
# (GPUI often exposes an empty accessibility tree).
os.environ.setdefault("PIMIENTO_AUTO_CONNECT", "1")
sys.stdout.flush(); sys.stderr.flush()
si = open("/dev/null", "rb")
so = open(log_path, "ab", buffering=0)
se = open(log_path, "ab", buffering=0)
os.dup2(si.fileno(), 0)
os.dup2(so.fileno(), 1)
os.dup2(se.fileno(), 2)
Path(pidfile).write_text(str(os.getpid()))
os.execv(bin_path, [bin_path])
PY

pid="$(cat "$PIDFILE" 2>/dev/null || true)"
# Give GPUI a beat to create the window before raising.
sleep 0.5
bring_to_front "$pid"
