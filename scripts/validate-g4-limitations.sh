#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
oryn_bin="${ORYN_BIN:-$repo_root/target/debug/oryn}"
artifact_dir="${ORYN_G4_ARTIFACT_DIR:-$repo_root/artifacts/g4-v1/limitations}"
mkdir -p "$artifact_dir"

printf '%s\n' 'observe --positions --hidden' exit |
  "$oryn_bin" native >"$artifact_dir/capabilities.jsonl"

jq -s -e '
  [
    .[]
    | select(.kind == "observation" or .kind == "geometry_observation")
    | .observation.capabilities[]
    | select(.support == "unsupported")
    | .capability
  ] as $capabilities
  | [
      "layout.table",
      "paint",
      "layout.hit_testing.paint_order",
      "layout.transform.3d",
      "layout.float.complex"
    ] as $expected
  | all($expected[]; ($capabilities | index(.)) != null)
' "$artifact_dir/capabilities.jsonl" >/dev/null

echo "G4 deferred capability diagnostics passed: 5/5 declared cases"
