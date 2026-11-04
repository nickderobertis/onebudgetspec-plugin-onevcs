#!/usr/bin/env bash
# Accumulate profiles across the crate and its binary journeys; report once in Nx.
set -euo pipefail
case "${1:-}" in
  onebudgetspec-plugin-onevcs|onebudgetspec-plugin-onevcs-e2e) ;;
  *) echo "coverage: pass a measured workspace package" >&2; exit 2 ;;
esac
eval "$(cargo llvm-cov show-env --sh)"
cargo build --locked -p onebudgetspec-plugin-onevcs
export SCAFFOLD_BIN="$CARGO_LLVM_COV_TARGET_DIR/debug/onebudgetspec-plugin-onevcs"
cargo test --locked -p "$1"
