#!/usr/bin/env bash
# Install the package that crates.io would serve, without uploading it.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."
crate=onebudgetspec-plugin-onevcs
pkgid="$(cargo pkgid -p "$crate")"
version="${pkgid##*#}"
cargo package --locked -p "$crate"
install_root="$(mktemp -d)"
trap 'rm -rf "$install_root"' EXIT
cargo install --locked --root "$install_root" --path "target/package/$crate-$version"
export SCAFFOLD_BIN="$install_root/bin/$crate"
cargo test --locked -p "$crate-e2e" --test cli
