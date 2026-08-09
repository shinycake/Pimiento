#!/usr/bin/env bash
# Idempotent Cloud Agent bootstrap for Pimiento.
# Installs/updates omp, then pins non-secret omp model roles.
# Cursor auth MUST come from Dashboard secret CURSOR_API_KEY (never this script).
set -euo pipefail

log() { printf '==> %s\n' "$*"; }

ensure_path() {
  export PATH="${HOME}/.local/bin:${HOME}/.omp/bin:${PATH:-/usr/bin:/bin}"
}

install_omp() {
  ensure_path
  if command -v omp >/dev/null 2>&1; then
    log "omp already present: $(omp --version 2>/dev/null || omp -V 2>/dev/null || true)"
  else
    log "installing omp via official installer"
    curl -fsSL https://omp.sh/install | sh
    ensure_path
  fi

  if ! command -v omp >/dev/null 2>&1; then
    printf 'error: omp not on PATH after install\n' >&2
    exit 1
  fi
  log "omp ok: $(command -v omp) ($(omp --version 2>/dev/null || true))"
}

configure_omp_models() {
  local cfg_dir="${HOME}/.omp/agent"
  local cfg="${cfg_dir}/config.yml"
  # Non-secret defaults only. Auth stays in Dashboard secrets / agent.db.
  # default = Composer 2.5; task stays on Luna for delegated code work (AGENTS.md).
  local roles_json='{"default":"cursor/composer-2.5","smol":"cursor/composer-2.5","slow":"cursor/composer-2.5","plan":"cursor/composer-2.5","task":"cursor/gpt-5.6-luna-max"}'
  mkdir -p "${cfg_dir}"
  chmod 700 "${cfg_dir}" 2>/dev/null || true

  if omp config set modelRoles "${roles_json}"; then
    :
  else
    # Fallback if `omp config` rejects the record write.
    cat > "${cfg}" <<'YAML'
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
  configure_omp_models
  verify_cursor_auth_hint
  log "cloud-agent-install complete"
}

main "$@"
