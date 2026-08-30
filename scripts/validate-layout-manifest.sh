#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$repo_root/benchmarks/conformance/layout-manifest-v2.json"
schema="$repo_root/benchmarks/schema/layout-result-v1.schema.json"

jq -e '
  .schema_version == 2 and .profile == "layout-production" and
  .supersedes == "layout-manifest-v1.json" and .baseline_commit == "b402e5c" and
  .locked == true and .execution_domains == ["native", "chromium_differential"] and
  .viewport == {"width":1280,"height":720,"device_pixel_ratio":1} and
  .font_policy.system_fonts == false and
  (.font_policy.fallback_sha256 | test("^[0-9a-f]{64}$")) and
  (.cases | length == 33) and
  ([.cases[].id] | length == (unique | length)) and
  ([.cases[] | select(.disposition == "required")] | length == 29) and
  ([.cases[] | select(.disposition == "declared_limitation")] | length == 4) and
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

jq -e '
  has("$defs") and
  (.properties | has("$defs") | not) and
  (."$defs" | has("sha1") and has("sha256") and has("binary") and has("worker") and has("artifact") and has("case"))
' "$schema" >/dev/null

echo "layout conformance manifest and result schema are locked and structurally valid"
