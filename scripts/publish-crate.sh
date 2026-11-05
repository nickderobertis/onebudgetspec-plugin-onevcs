#!/usr/bin/env bash
# Publish the crate a release tag names, once. A re-run after a partial failure, or of a
# release a newer version has already passed, publishes nothing and succeeds.
# Usage: scripts/publish-crate.sh vX.Y.Z
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
crate=onebudgetspec-plugin-onevcs
tag="${1:-}"
case "$tag" in
  v[0-9]*) version="${tag#v}" ;;
  *) echo "publish-crate: expected a release tag like v1.2.3, got '$tag'; run it from the release workflow." >&2; exit 2 ;;
esac
pkgid="$(cargo pkgid -p "$crate")"
declared="${pkgid##*#}"
if [ "$declared" != "$version" ]; then
  echo "publish-crate: tag $tag does not match the manifest version $declared; check out the tagged commit." >&2
  exit 1
fi
if ! live="$(bash scripts/release-probe.sh "crate:$crate")"; then
  echo "publish-crate: could not read the registry (above), so nothing was published; re-run the release workflow." >&2
  exit 1
fi
if [ -n "$live" ] && [ "$(printf '%s\n%s\n' "$live" "$version" | sort -V | tail -n 1)" = "$live" ]; then
  exit 0
fi
cargo publish --locked -p "$crate"
