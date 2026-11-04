#!/usr/bin/env bash
# Best-effort judged-tool installation; oneharness.toml owns harness selection.
# llmlint: ignore-file[tool_output_is_signal, boundary_inputs_validated] Session hooks report install progress and failures but return zero so startup continues; package executables come through the PyPI installer rather than unchecked downloaded shell.
set -uo pipefail

readonly LLMLINT_MIN="0.3.23"
readonly ORIG_PATH="$PATH"
readonly BIN_DIR="$HOME/.local/bin"

log() { printf 'setup-llmlint: %s\n' "$*" >&2; }

ensure_toolchain() {
  if ! command -v uv >/dev/null 2>&1; then
    log "uv not found; cannot install llmlint (install uv: https://docs.astral.sh/uv/)"
    return 0
  fi
  log "installing llmlint-cli >= $LLMLINT_MIN via uv tool"
  uv tool install --upgrade "llmlint-cli>=$LLMLINT_MIN" >&2 \
    || log "llmlint-cli install failed (continuing)"
}

persist_session_env() {
  [ -n "${CLAUDE_ENV_FILE:-}" ] || { log "no CLAUDE_ENV_FILE (not a session); skipping env"; return 0; }
  {
    case ":${ORIG_PATH}:" in *":${BIN_DIR}:"*) ;; *) printf 'export PATH=%q\n' "${BIN_DIR}:${PATH}";; esac
  } >> "$CLAUDE_ENV_FILE"
  log "exported PATH"
}

export PATH="${BIN_DIR}:${PATH}"
ensure_toolchain
persist_session_env
if command -v llmlint >/dev/null 2>&1; then
  log "ready (llmlint: $(llmlint --version 2>/dev/null || echo unknown))"
  llmlint doctor >&2 2>&1 || log "llmlint doctor reported an issue (see above)"
else
  log "llmlint not installed"
fi
exit 0
