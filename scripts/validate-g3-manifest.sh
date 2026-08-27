#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$repo_root/benchmarks/conformance/g3-manifest-v1.json"

jq -e '
  .schema_version == 1 and
  .gate == "G3" and
  .baseline_commit == "c0472b6" and
  .locked == true and
  .execution_domains == ["native", "chromium_differential"] and
  (.cases | length >= 45) and
  ([.cases[].id] | length == (unique | length)) and
  ([.cases[].category] | unique | contains([
    "contract", "page_group", "storage", "indexeddb", "xhr",
    "fetch_streams", "websocket", "file_fetch", "crypto", "modules",
    "webidl", "dom", "framework", "oauth", "geometry", "cross_origin",
    "later_platform"
  ])) and
  ([.cases[] | select(.disposition == "required") | .baseline] |
    all(. == "passed" or . == "partial" or . == "missing")) and
  ([.cases[] | select(.disposition == "g4_deferred") | .expected_capability] |
    length >= 5 and all(type == "string" and length > 0)) and
  ([.cases[] | select(.category == "framework")] | length == 8) and
  ([.cases[] | select(.category == "oauth")] | length == 5)
' "$manifest" >/dev/null

echo "G3 conformance manifest is locked and structurally valid"
