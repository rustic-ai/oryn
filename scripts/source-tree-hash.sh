#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

git ls-files \
  | LC_ALL=C sort \
  | while IFS= read -r path; do
      case "$path" in
        benchmarks/evidence/*|docs/oryn2/IMPLEMENTATION_STATUS.md) continue ;;
      esac
      printf '%s\0' "$path"
      shasum -a 256 "$path" | awk '{print $1}'
    done \
  | shasum -a 256 \
  | awk '{print $1}'
