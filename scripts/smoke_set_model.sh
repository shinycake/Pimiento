#!/usr/bin/env bash
# Cheap headless smoke: set_model to cursor/composer-2.5 and verify get_state.
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PY'
import json, os, subprocess, sys, time
omp = os.environ.get('PIMIENTO_OMP_BIN') or '/Users/idan/.bun/bin/omp'
provider, mid = 'cursor', 'composer-2.5'
proc = subprocess.Popen(
    [omp, '--mode', 'rpc-ui', '--no-session'],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
    text=True, bufsize=1,
)

def read_obj(timeout=60):
    end = time.time() + timeout
    while time.time() < end:
        line = proc.stdout.readline()
        if not line:
            raise SystemExit('eof from omp')
        try:
            return json.loads(line)
        except Exception:
            continue
    raise SystemExit('timeout reading frame')

def wait_id(want, timeout=90):
    chunks = {}
    end = time.time() + timeout
    while time.time() < end:
        obj = read_obj(timeout=max(1, end - time.time()))
        t = obj.get('type')
        if t == 'extension_ui_request' and obj.get('id'):
            cancel = {
                'type': 'extension_ui_response',
                'id': obj['id'],
                'response': {'cancelled': True},
            }
            proc.stdin.write(json.dumps(cancel) + '\n')
            proc.stdin.flush()
            continue
        if t in ('agent_start', 'message_update', 'notice', 'tool_execution', 'model_changed'):
            continue
        if t == 'rpc_chunk':
            rid = obj.get('id')
            part = obj.get('data') or obj.get('chunk') or ''
            if isinstance(part, dict):
                part = part.get('data') or json.dumps(part)
            chunks.setdefault(rid, []).append(part if isinstance(part, str) else json.dumps(part))
            done = obj.get('done') or obj.get('final')
            if obj.get('index') is not None and obj.get('total') is not None:
                done = obj['index'] + 1 >= obj['total']
            if done:
                outer = json.loads(''.join(chunks[rid]))
                if outer.get('id') == want or rid == want:
                    return outer
            continue
        if obj.get('id') == want:
            return obj
    raise SystemExit(f'timeout waiting id={want}')

ready = read_obj()
assert ready.get('type') == 'ready', ready
# Flat wire shape (no params wrapper) — matches omp-rpc-client RpcCommand serialization.
proc.stdin.write(json.dumps({'id': 1, 'type': 'negotiate_protocol', 'protocolVersion': 2}) + '\n')
proc.stdin.flush()
neg = wait_id(1)
print('negotiated', neg.get('success'), (neg.get('data') or {}).get('protocolVersion') or neg.get('protocolVersion'))
proc.stdin.write(json.dumps({'id': 2, 'type': 'get_state'}) + '\n')
proc.stdin.flush()
st = wait_id(2)
model = (st.get('data') or {}).get('model')
before = model if isinstance(model, str) else f"{(model or {}).get('provider')}/{(model or {}).get('id')}"
print('before', before)
proc.stdin.write(json.dumps({
    'id': 4,
    'type': 'set_model',
    'provider': provider,
    'modelId': mid,
}) + '\n')
proc.stdin.flush()
sm = wait_id(4)
print('set_model_ok', sm.get('success'), sm.get('error'))
proc.stdin.write(json.dumps({'id': 5, 'type': 'get_state'}) + '\n')
proc.stdin.flush()
st2 = wait_id(5)
model2 = (st2.get('data') or {}).get('model')
after = model2 if isinstance(model2, str) else f"{(model2 or {}).get('provider')}/{(model2 or {}).get('id')}"
print('after', after)
proc.kill()
ok = bool(sm.get('success')) and mid in str(after)
print('PASS' if ok else 'FAIL')
sys.exit(0 if ok else 1)
PY
