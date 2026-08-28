#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$repo_root/benchmarks/conformance/g4-manifest-v1.json"

jq -e '
  .schema_version == 1 and .gate == "G4" and .baseline_commit == "8d444f6" and
  .locked == true and .execution_domains == ["native", "chromium_differential"] and
  .viewport == {"width":1280,"height":720,"device_pixel_ratio":1} and
  .font_policy.system_fonts == false and
  (.font_policy.fallback_sha256 | test("^[0-9a-f]{64}$")) and
  (.cases | length == 33) and
  ([.cases[].id] | length == (unique | length)) and
  ([.cases[] | select(.disposition == "required")] | length == 28) and
  ([.cases[] | select(.disposition == "declared_limitation")] | length == 5) and
  ([.cases[] | select(.disposition == "required") | .baseline] |
    all(. == "passed" or . == "partial" or . == "missing")) and
  ([.cases[] | select(.disposition == "declared_limitation") | .expected_capability] |
    all(type == "string" and length > 0)) and
  ([.cases[].category] | unique | contains([
    "contract", "worker", "style", "cssom", "layout", "text", "font",
    "viewport", "semantic_geometry", "oil", "action", "security",
    "differential", "limitation"
  ]))
' "$manifest" >/dev/null

echo "G4 conformance manifest is locked and structurally valid"
