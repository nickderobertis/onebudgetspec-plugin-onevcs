#!/usr/bin/env bash
# Best-effort startup: provision just and the judged tier without blocking a session.
# llmlint: ignore-file[tool_output_is_signal, boundary_inputs_validated] Session hooks report install progress and failures but return zero so startup continues; package executables come through the PyPI installer rather than unchecked downloaded shell.
set -uo pipefail

readonly JUST_INSTALL_FLOOR="1.51.0"
readonly BIN_DIR="$HOME/.local/bin"
readonly ORIG_PATH="${PATH}"

log() { printf 'session-setup: %s\n' "$*" >&2; }

if [ -n "${CI:-}" ]; then
  log "CI detected; skipping (toolchain provisioned by the CI workflow)"
  exit 0
fi

export PATH="${BIN_DIR}:${PATH}"

ensure_just() {
  if command -v just >/dev/null 2>&1; then
    log "just present ($(just --version 2>/dev/null || echo unknown))"
    return 0
  fi
  if ! command -v uv >/dev/null 2>&1; then
    log "uv not found; cannot install just (install uv: https://docs.astral.sh/uv/)"
    return 0
  fi
  log "installing rust-just >= $JUST_INSTALL_FLOOR via uv tool"
  uv tool install --upgrade "rust-just>=$JUST_INSTALL_FLOOR" >&2 \
    || log "rust-just install failed (continuing)"
}

verify_uv() {
  local tool
  for tool in uv; do
    command -v "$tool" >/dev/null 2>&1 \
      || log "$tool not on PATH (install uv before running the development recipes)"
  done
}

persist_session_env() {
  [ -n "${CLAUDE_ENV_FILE:-}" ] || { log "no CLAUDE_ENV_FILE (not a session); skipping env"; return 0; }
  case ":${ORIG_PATH}:" in
    *":${BIN_DIR}:"*) ;;
    *) printf 'export PATH=%q\n' "${BIN_DIR}:${PATH}" >> "$CLAUDE_ENV_FILE"
       log "prepended ${BIN_DIR} to PATH for the session";;
  esac
}

ensure_just
verify_uv
persist_session_env

setup_llmlint="$(dirname "$0")/setup-llmlint.sh"
if [ -x "$setup_llmlint" ]; then
  log "running setup-llmlint.sh"
  "$setup_llmlint" || log "setup-llmlint.sh reported an issue (continuing)"
fi

exit 0
