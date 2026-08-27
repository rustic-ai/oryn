#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$repo_root/benchmarks/corpus/g2.json"

jq -e '
  .schema_version == 1 and
  (.tasks | length == 30) and
  ([.tasks[].id] | length == (unique | length)) and
  ([.tasks[] | select(.class == "harness")] | length == 12) and
  ([.tasks[] | select(.class == "miniwob")] | length == 8) and
  ([.tasks[] | select(.class == "framework")] | length == 6) and
  ([.tasks[] | select(.class == "public_canary")] | length == 4) and
  ([.tasks[] | select(.class == "public_canary" and .policy != "read_only")] | length == 0)
' "$manifest" >/dev/null

while IFS= read -r source; do
  if [[ ! -f "$repo_root/$source" ]]; then
    echo "Missing required corpus source: $source" >&2
    exit 1
  fi
done < <(jq -r '.tasks[] | select(.required and (.source | startswith("http") | not)) | .source' "$manifest")

echo "Benchmark corpus is valid: 30 tasks (12 harness, 8 MiniWoB, 6 framework, 4 canary)"
