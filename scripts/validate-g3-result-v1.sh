#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
poetry="$repo_root/artifacts/tools/poetry/bin/poetry"
args=("$@")
if [[ -n "${1:-}" && "${1:-}" != --* ]]; then
  args=(--result "$1" "${@:2}")
fi

cd "$repo_root/intentgym"
if [[ -x "$poetry" ]]; then
  "$poetry" run python "$repo_root/scripts/validate_g3_result.py" "${args[@]}"
else
  poetry run python "$repo_root/scripts/validate_g3_result.py" "${args[@]}"
fi
