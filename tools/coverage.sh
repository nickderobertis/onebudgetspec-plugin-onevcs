#!/usr/bin/env bash
# One profile directory per clone; measure per project and enforce the union once.
set -euo pipefail
case "${1:-}" in
  --clear)
    cargo llvm-cov clean --workspace
    rm -f target/coverage-ran
    exit 0 ;;
  --report)
    # A documentation-only affected set runs no measured Rust suite.
    [ -f target/coverage-ran ] || exit 0
    exec cargo llvm-cov report --summary-only --fail-under-lines 95 ;;
  onebudgetspec-plugin-onevcs|onebudgetspec-plugin-onevcs-e2e) ;;
  *) echo "coverage: pass a measured package, --clear or --report" >&2; exit 2 ;;
esac
export CARGO_TARGET_DIR="$PWD/target/llvm-cov-target"
eval "$(cargo llvm-cov show-env --sh)"
cargo build --locked -p onebudgetspec-plugin-onevcs
export SCAFFOLD_BIN="$CARGO_LLVM_COV_TARGET_DIR/debug/onebudgetspec-plugin-onevcs"
cargo test --locked -p "$1"
touch target/coverage-ran
