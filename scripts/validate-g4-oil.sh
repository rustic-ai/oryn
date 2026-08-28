#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
oryn_bin="${ORYN_BIN:-$repo_root/target/debug/oryn}"
artifact_dir="${ORYN_G4_ARTIFACT_DIR:-$repo_root/artifacts/g4-v1/oil}"
fixture="$repo_root/benchmarks/g4/g4-oil.html"
mkdir -p "$artifact_dir"

printf '%s\n' \
  'observe --full --positions --hidden' \
  'box "Target"' \
  'viewport 320 200' \
  'media color-scheme dark' \
  'media reduced-motion reduce' \
  'scroll "Far"' \
  'observe --full --viewport --positions' \
  exit |
  "$oryn_bin" native --html "$fixture" >"$artifact_dir/oil.jsonl"

jq -s -e '
  ([.[] | select(.kind == "geometry_observation")] | length) == 2 and
  ([.[] | select(.kind == "box" and .box_model.border_box.width == 120)] | length) == 1 and
  ([.[] | select(.kind == "viewport" and .viewport.width == 320 and .viewport.height == 200)] | length) == 1 and
  ([.[] | select(.kind == "media" and .media.color_scheme == "dark")] | length) >= 1 and
  ([.[] | select(.kind == "media" and .media.reduced_motion == "reduce")] | length) >= 1 and
  ([.[] | select(.kind == "scroll" and .scroll.y > 0)] | length) == 1 and
  ([.[] | select(.kind == "geometry_observation")][-1].observation.nodes |
    any(.name == "Far" and .geometry.in_viewport == true))
' "$artifact_dir/oil.jsonl" >/dev/null

echo "G4 OIL geometry, viewport, media, and scroll commands passed"
