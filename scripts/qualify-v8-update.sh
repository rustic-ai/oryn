#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
expected_version="${1:-}"
expected_checksum="${2:-}"
output_dir="${3:-$repo_root/artifacts/g5-v1/v8-qualification}"
policy="$repo_root/security/v8-update-policy.json"

if [[ -z "$expected_version" || -z "$expected_checksum" ]]; then
  echo "usage: $0 <v8-crate-version> <crate-checksum> [artifact-directory]" >&2
  exit 2
fi
if [[ "$expected_version" != "$(jq -r '.expected_crate_version' "$policy")" ]]; then
  echo "requested V8 version does not match the reviewed update policy" >&2
  exit 2
fi
if [[ "$expected_checksum" != "$(jq -r '.expected_crate_checksum' "$policy")" ]]; then
  echo "requested V8 checksum does not match the reviewed update policy" >&2
  exit 2
fi
if [[ -n "$(git -C "$repo_root" status --porcelain --untracked-files=all)" ]]; then
  echo "V8 qualification requires a clean source tree" >&2
  exit 2
fi

"$repo_root/scripts/validate-g5-security.py"
exec "$repo_root/scripts/run-g5-production-gate.sh" "$output_dir"
