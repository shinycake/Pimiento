#!/usr/bin/env bash
# Idempotent Cloud Agent bootstrap for Pimiento.
# Installs/updates omp, installs a Cursor-auth shim, then pins non-secret omp model roles.
#
# Auth rule: Dashboard secret CURSOR_API_KEY is required for Cursor models.
# omp v17 currently reads CURSOR_ACCESS_TOKEN for the cursor provider, so the
# shim exchanges CURSOR_API_KEY -> CURSOR_ACCESS_TOKEN (never logged/committed).
set -euo pipefail

log() { printf '==> %s\n' "$*"; }

ensure_path() {
  export PATH="${HOME}/.local/bin:${HOME}/.omp/bin:${PATH:-/usr/bin:/bin}"
}

real_omp_path() {
  printf '%s\n' "${HOME}/.omp/bin/omp"
}

install_omp() {
  ensure_path
  mkdir -p "${HOME}/.omp/bin" "${HOME}/.local/bin"

  local real
  real="$(real_omp_path)"

  # Official installer drops a binary at ~/.local/bin/omp. Keep the real
  # binary under ~/.omp/bin/omp and put our auth shim on PATH instead.
  is_real_binary() {
    local p="$1"
    [[ -x "${p}" ]] && ! head -n1 "${p}" 2>/dev/null | grep -q '^#!'
  }

  if is_real_binary "${real}"; then
    log "omp already present: $("${real}" --version 2>/dev/null || true)"
  elif is_real_binary "${HOME}/.local/bin/omp"; then
    log "relocating existing omp binary to ${real}"
    mv -f "${HOME}/.local/bin/omp" "${real}"
  else
    log "installing omp via official installer"
    # Temporarily move any existing shim aside so the installer can write.
    if [[ -e "${HOME}/.local/bin/omp" ]]; then
      mv -f "${HOME}/.local/bin/omp" "${HOME}/.local/bin/omp.pre-install.bak" || true
    fi
    curl -fsSL https://omp.sh/install | sh
    ensure_path
    if [[ -x "${HOME}/.local/bin/omp" ]]; then
      mv -f "${HOME}/.local/bin/omp" "${real}"
    fi
    rm -f "${HOME}/.local/bin/omp.pre-install.bak" || true
  fi

  if [[ ! -x "${real}" ]]; then
    # Fallback: maybe omp landed only on PATH elsewhere.
    local found
    found="$(command -v omp || true)"
    if [[ -n "${found}" && -x "${found}" && "${found}" != "${HOME}/.local/bin/omp" ]]; then
      cp -f "${found}" "${real}"
      chmod 755 "${real}"
    fi
  fi

  if [[ ! -x "${real}" ]]; then
    printf 'error: real omp binary missing at %s after install\n' "${real}" >&2
    exit 1
  fi
  log "omp ok: ${real} ($("${real}" --version 2>/dev/null || true))"
}

install_cursor_auth_shim() {
  local real shim cache_dir
  real="$(real_omp_path)"
  shim="${HOME}/.local/bin/omp"
  cache_dir="${HOME}/.omp/agent"
  mkdir -p "${HOME}/.local/bin" "${cache_dir}"
  chmod 700 "${cache_dir}" 2>/dev/null || true

  # Wrapper exchanges CURSOR_API_KEY -> CURSOR_ACCESS_TOKEN when needed, then
  # execs the real binary. Never prints credential values.
  cat >"${shim}" <<EOF
#!/usr/bin/env bash
set -euo pipefail
REAL_OMP="${real}"
CACHE_DIR="\${HOME}/.omp/agent"
CACHE_FILE="\${CACHE_DIR}/.cursor_access_token"
META_FILE="\${CACHE_DIR}/.cursor_access_token.exp"

ensure_cursor_access_token() {
  if [[ -n "\${CURSOR_ACCESS_TOKEN:-}" ]]; then
    return 0
  fi
  if [[ -z "\${CURSOR_API_KEY:-}" ]]; then
    return 0
  fi

  mkdir -p "\${CACHE_DIR}"
  chmod 700 "\${CACHE_DIR}" 2>/dev/null || true

  local now exp=0
  now="\$(date +%s)"
  if [[ -f "\${CACHE_FILE}" && -f "\${META_FILE}" ]]; then
    exp="\$(cat "\${META_FILE}" 2>/dev/null || echo 0)"
    if [[ "\${exp}" =~ ^[0-9]+$ ]] && (( now + 300 < exp )); then
      export CURSOR_ACCESS_TOKEN="\$(cat "\${CACHE_FILE}")"
      return 0
    fi
  fi

  # Exchange dashboard user API key for a short-lived session access token.
  # Request shape matches Cursor's exchange_user_api_key (Bearer + empty JSON).
  local tmp
  tmp="\$(mktemp)"
  if ! curl -fsS -X POST "https://api2.cursor.sh/auth/exchange_user_api_key" \\
    -H "Content-Type: application/json" \\
    -H "Authorization: Bearer \${CURSOR_API_KEY}" \\
    --data '{}' \\
    -o "\${tmp}"; then
    rm -f "\${tmp}"
    printf 'error: failed to exchange CURSOR_API_KEY for CURSOR_ACCESS_TOKEN\n' >&2
    exit 1
  fi

  # Parse JSON without printing secrets. Writes sibling files for the shell.
  python3 - "\${tmp}" <<'PY'
import json, sys, base64, time
path = sys.argv[1]
obj = json.load(open(path, encoding="utf-8"))
access = obj.get("accessToken") or obj.get("access_token")
if not access:
    raise SystemExit("missing accessToken in exchange response")
# Derive exp from JWT payload when present; else cache ~50 minutes.
exp = int(time.time()) + 3000
try:
    payload = access.split(".")[1]
    payload += "=" * (-len(payload) % 4)
    data = json.loads(base64.urlsafe_b64decode(payload.encode("ascii")))
    if isinstance(data.get("exp"), (int, float)):
        exp = int(data["exp"])
except Exception:
    pass
open(path + ".access", "w", encoding="utf-8").write(access)
open(path + ".exp", "w", encoding="utf-8").write(str(exp))
PY
  local access exp
  access="\$(cat "\${tmp}.access")"
  exp="\$(cat "\${tmp}.exp")"
  umask 077
  printf '%s' "\${access}" >"\${CACHE_FILE}"
  printf '%s' "\${exp}" >"\${META_FILE}"
  chmod 600 "\${CACHE_FILE}" "\${META_FILE}" 2>/dev/null || true
  rm -f "\${tmp}" "\${tmp}.access" "\${tmp}.exp"
  export CURSOR_ACCESS_TOKEN="\${access}"
}

ensure_cursor_access_token
exec "\${REAL_OMP}" "\$@"
EOF
  chmod 755 "${shim}"
  log "installed Cursor auth shim at ${shim} -> ${real}"
}

configure_omp_models() {
  local cfg_dir="${HOME}/.omp/agent"
  local cfg="${cfg_dir}/config.yml"
  # Non-secret defaults only. Auth stays in Dashboard secrets / agent.db.
  # default = Composer 2.5; task stays on Luna for delegated code work (AGENTS.md).
  local roles_json='{"default":"cursor/composer-2.5","smol":"cursor/composer-2.5","slow":"cursor/composer-2.5","plan":"cursor/composer-2.5","task":"cursor/gpt-5.6-luna-max"}'
  mkdir -p "${cfg_dir}"
  chmod 700 "${cfg_dir}" 2>/dev/null || true

  ensure_path
  if omp config set modelRoles "${roles_json}"; then
    :
  else
    # Fallback if `omp config` rejects the record write.
    cat >"${cfg}" <<'YAML'
modelRoles:
  default: cursor/composer-2.5
  smol: cursor/composer-2.5
  slow: cursor/composer-2.5
  plan: cursor/composer-2.5
  task: cursor/gpt-5.6-luna-max
defaultThinkingLevel: minimal
YAML
    chmod 600 "${cfg}"
  fi

  log "omp modelRoles:"
  omp config get modelRoles 2>/dev/null || cat "${cfg}"
}

verify_cursor_auth_hint() {
  if [[ -n "${CURSOR_API_KEY:-}" ]]; then
    log "Cursor env credential present (CURSOR_API_KEY)"
    # Warm the shim cache once so the first agent call is fast.
    ensure_path
    if omp models cursor >/dev/null 2>&1; then
      log "Cursor auth exchange OK (models discoverable)"
    else
      log "WARNING: CURSOR_API_KEY present but Cursor model discovery failed"
    fi
    return 0
  fi
  if [[ -n "${CURSOR_ACCESS_TOKEN:-}" ]]; then
    log "Cursor env credential present (CURSOR_ACCESS_TOKEN fallback)"
    return 0
  fi
  if omp token cursor --list >/dev/null 2>&1; then
    log "Cursor OAuth credential present in local agent.db (dev snapshot only)"
    return 0
  fi
  log "WARNING: no Cursor credential in env or agent.db — set Dashboard secret CURSOR_API_KEY"
  return 0
}

main() {
  install_omp
  install_cursor_auth_shim
  configure_omp_models
  verify_cursor_auth_hint
  log "cloud-agent-install complete"
}

main "$@"
