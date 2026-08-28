#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
poetry="$repo_root/artifacts/tools/poetry/bin/poetry"
cd "$repo_root/intentgym"
if [[ -x "$poetry" ]]; then
  exec "$poetry" run python "$repo_root/scripts/validate_g5_result.py" "$@"
fi
exec poetry run python "$repo_root/scripts/validate_g5_result.py" "$@"
