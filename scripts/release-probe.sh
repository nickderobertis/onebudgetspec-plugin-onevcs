#!/usr/bin/env bash
# onevcs's declared three-answer probe; Python supplies real TOML/JSON validation.
set -euo pipefail
exec python3 "$(dirname "${BASH_SOURCE[0]}")/release_probe.py" "$@"
