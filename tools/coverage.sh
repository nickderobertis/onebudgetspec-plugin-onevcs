#!/usr/bin/env bash
# One profile directory per clone; measure per project and enforce the union once.
set -euo pipefail
case "${1:-}" in
  --clear)
    cargo llvm-cov clean --workspace
    rm -f target/coverage-ran
    exit 0 ;;
  --report)
    [ -f target/coverage-ran ] || exit 0
    exec cargo llvm-cov report --summary-only --fail-under-lines 95 ;;
  ""|-*|*[!A-Za-z0-9_-]*) echo "coverage: pass a Cargo package, --clear or --report" >&2; exit 2 ;;
esac
export CARGO_TARGET_DIR="$PWD/target/llvm-cov-target"
if ! coverage_env="$(cargo llvm-cov show-env --sh)"; then
  echo "coverage: could not read instrumentation environment; fix cargo llvm-cov's error above and retry" >&2
  exit 1
fi
eval "$coverage_env"
if [ -n "${2:-}" ]; then
  cargo build --locked -p "$2"
  export SCAFFOLD_BIN="$CARGO_LLVM_COV_TARGET_DIR/debug/$2"
fi
cargo test --locked -p "$1"
touch target/coverage-ran
