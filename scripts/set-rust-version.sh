#!/usr/bin/env bash
# Update only the workspace and local Cargo.lock package versions.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "${SCRIPT_DIR}/version.py" set "$@"
