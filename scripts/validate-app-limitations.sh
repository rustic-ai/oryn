#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
oryn_bin="${ORYN_BIN:-$repo_root/target/debug/oryn}"
artifact_dir="${ORYN_APP_COMPATIBILITY_ARTIFACT_DIR:-$repo_root/artifacts/app-compatibility/limitations}"
mkdir -p "$artifact_dir"

printf '%s\n' observe exit |
  "$oryn_bin" native >"$artifact_dir/capabilities.jsonl"

jq -s -e '
  [
    .[]
    | select(.kind == "observation")
    | .observation.capabilities[]
    | select(.support == "unsupported")
    | .capability
  ] as $capabilities
  | ($capabilities | index("rendered_layout_and_paint")) != null
    and ($capabilities | index("cross_origin_dom")) != null
    and ($capabilities | index("service_worker_and_cache_api")) != null
    and ($capabilities | index("webrtc_and_media")) != null
' "$artifact_dir/capabilities.jsonl" >/dev/null

echo "application compatibility deferred capability diagnostics passed: 5/5 declared cases"
