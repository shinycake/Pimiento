#!/usr/bin/env bash
# scripts/dogfood.sh — record the live OMP session pointer for terminal fallback.
#
# Writes ~/.pimiento/dogfood.json with {sessionFile, cwd, updatedAt}.
# If Pimiento cannot launch, resume from the terminal with:
#   omp --resume "$(jq -r .sessionFile ~/.pimiento/dogfood.json)"
set -euo pipefail

PIMIENTO_HOME="${PIMIENTO_HOME:-$HOME/.pimiento}"
DOGFOOD_JSON="$PIMIENTO_HOME/dogfood.json"
LAST_SESSION="$PIMIENTO_HOME/last-session"
CWD="${PIMIENTO_CWD:-$(pwd)}"
mkdir -p "$PIMIENTO_HOME"

SESSION_FILE=""
if [[ -f "$LAST_SESSION" ]]; then
  SESSION_FILE="$(tr -d '[:space:]' < "$LAST_SESSION")"
fi

if [[ "${1:-}" != "" ]]; then
  SESSION_FILE="$1"
fi

if [[ -z "$SESSION_FILE" ]]; then
  echo "No sessionFile yet. Launch Pimiento once (it writes $LAST_SESSION), then re-run." >&2
  echo "Or: $0 /path/to/session.jsonl" >&2
  exit 1
fi

if [[ ! -f "$SESSION_FILE" ]]; then
  echo "sessionFile not found: $SESSION_FILE" >&2
  exit 1
fi

UPDATED_AT="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
python3 - "$DOGFOOD_JSON" "$SESSION_FILE" "$CWD" "$UPDATED_AT" <<'PY'
import json, sys
path, session, cwd, updated = sys.argv[1:5]
payload = {"sessionFile": session, "cwd": cwd, "updatedAt": updated}
with open(path, "w", encoding="utf-8") as f:
    json.dump(payload, f, indent=2)
    f.write("\n")
print(path)
print(json.dumps(payload))
PY

echo "Terminal fallback:"
echo "  omp --resume \"$SESSION_FILE\""
