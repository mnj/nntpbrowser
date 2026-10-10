#!/usr/bin/env bash
# Print the next stable version; auto uses the Conventional Commit title.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "${SCRIPT_DIR}/version.py" plan "$@"
