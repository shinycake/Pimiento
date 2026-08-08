#!/usr/bin/env bash
# Cheap headless smoke: set_model to cursor/composer-2.5 and verify get_state.
# Wire shape matches omp-rpc-client: flat NDJSON, no params wrapper.
set -euo pipefail
cd "$(dirname "$0")/.."

# Avoid colliding with a leftover rpc-ui child from a prior hung smoke.
pkill -f 'omp --mode rpc-ui --no-session' >/dev/null 2>&1 || true
sleep 0.2

python3 - <<'PY'
import json, os, subprocess, sys, time

omp = os.environ.get("PIMIENTO_OMP_BIN") or "/Users/idan/.bun/bin/omp"
provider, mid = "cursor", "composer-2.5"
proc = subprocess.Popen(
    [omp, "--mode", "rpc-ui", "--no-session"],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.DEVNULL,
    text=True,
    bufsize=1,
)


def read_obj(timeout=60):
    end = time.time() + timeout
    while time.time() < end:
        line = proc.stdout.readline()
        if not line:
            raise SystemExit("eof from omp")
        try:
            return json.loads(line)
        except Exception:
            continue
    raise SystemExit("timeout reading frame")


def same_id(a, b):
    return str(a) == str(b)


def wait_response(want_id, timeout=90):
    """Wait for a typed RPC response for want_id (ignores events / chunks)."""
    chunks = {}
    end = time.time() + timeout
    while time.time() < end:
        obj = read_obj(timeout=max(1, end - time.time()))
        t = obj.get("type")
        if t == "extension_ui_request" and obj.get("id") is not None:
            cancel = {
                "type": "extension_ui_response",
                "id": obj["id"],
                "response": {"cancelled": True},
            }
            proc.stdin.write(json.dumps(cancel) + "\n")
            proc.stdin.flush()
            continue
        if t == "rpc_chunk" and same_id(obj.get("id"), want_id):
            part = obj.get("data") or obj.get("chunk") or ""
            if isinstance(part, dict):
                part = part.get("data") or json.dumps(part)
            chunks.setdefault(str(want_id), []).append(
                part if isinstance(part, str) else json.dumps(part)
            )
            done = obj.get("done") or obj.get("final")
            if obj.get("index") is not None and obj.get("total") is not None:
                done = obj["index"] + 1 >= obj["total"]
            if done:
                outer = json.loads("".join(chunks[str(want_id)]))
                if outer.get("type") == "response":
                    return outer
            continue
        if t == "response" and same_id(obj.get("id"), want_id):
            return obj
        # Ignore unsolicited events (model_changed, available_commands_update, …).
    raise SystemExit(f"timeout waiting response id={want_id}")


def send(cmd):
    proc.stdin.write(json.dumps(cmd) + "\n")
    proc.stdin.flush()


ready = read_obj()
assert ready.get("type") == "ready", ready

send({"id": "1", "type": "negotiate_protocol", "protocolVersion": 2})
neg = wait_response("1")
print(
    "negotiated",
    neg.get("success"),
    ((neg.get("data") or {}) if isinstance(neg.get("data"), dict) else {}).get(
        "protocolVersion"
    ),
)
if not neg.get("success"):
    print("FAIL negotiate", neg)
    proc.kill()
    sys.exit(1)

send({"id": "2", "type": "get_state"})
st = wait_response("2")
model = (st.get("data") or {}).get("model")
before = (
    model
    if isinstance(model, str)
    else f"{(model or {}).get('provider')}/{(model or {}).get('id')}"
)
print("before", before)

send({"id": "4", "type": "set_model", "provider": provider, "modelId": mid})
sm = wait_response("4")
print("set_model_ok", sm.get("success"), sm.get("error"))
if not sm.get("success"):
    print("FAIL set_model", sm)
    proc.kill()
    sys.exit(1)

send({"id": "5", "type": "get_state"})
st2 = wait_response("5")
model2 = (st2.get("data") or {}).get("model")
after = (
    model2
    if isinstance(model2, str)
    else f"{(model2 or {}).get('provider')}/{(model2 or {}).get('id')}"
)
print("after", after)
proc.kill()
ok = mid in str(after) and provider in str(after)
print("PASS" if ok else "FAIL")
sys.exit(0 if ok else 1)
PY
