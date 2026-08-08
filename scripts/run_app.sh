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

if [[ -f "$PIDFILE" ]] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
  echo "already running pid=$(cat "$PIDFILE")"
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
