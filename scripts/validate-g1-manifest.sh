#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$repo_root/benchmarks/conformance/g1-manifest-v1.json"

jq -e '
  .schema_version == 1 and
  .gate == "G1" and
  .execution_domain == "native" and
  (.cases | length > 0) and
  ([.cases[].id] | length == (unique | length)) and
  ([.cases[].category] | unique | contains([
    "html", "scripts", "modules", "events", "forms", "fetch", "url",
    "encoding", "compression", "cookies", "lifecycle", "shadow_dom",
    "frames", "capabilities"
  ])) and
  ([.cases[].status] | all(. == "passed" or . == "partial" or . == "missing"))
' "$manifest" >/dev/null

if [[ "${1:-}" == "--gate" ]]; then
  jq -e '[.cases[] | select(.required and .status != "passed")] | length == 0' \
    "$manifest" >/dev/null
fi

echo "G1 conformance manifest is structurally valid"
