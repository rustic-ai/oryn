#!/usr/bin/env bash
set -euo pipefail

# Full-plan generation admission, including direct invocation.
python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$repo_root/benchmarks/layout/wpt-subset-v1.json"

jq -e '
  .schema_version == 1 and .locked == true and
  .upstream.project == "web-platform-tests/wpt" and
  (.upstream.revision | test("^[0-9a-f]{40}$")) and
  ([.subsets[].category] | sort == ["css_cascade","css_flexbox","css_grid","css_position","css_transforms","cssom","cssom_view","fonts"]) and
  all(.subsets[]; (.sources | length) > 0 and (((.native_tests // []) + (.layout_tests // [])) | length) > 0)
' "$manifest" >/dev/null

python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"
cargo test --manifest-path "$repo_root/Cargo.toml" -p oryn-layout --lib
python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"
cargo test --manifest-path "$repo_root/Cargo.toml" -p oryn-native \
  --features v8-host,in-process-probe --lib

native_tests="$(cargo test --manifest-path "$repo_root/Cargo.toml" -p oryn-native \
  --features v8-host,in-process-probe --lib -- --list)"
layout_tests="$(cargo test --manifest-path "$repo_root/Cargo.toml" -p oryn-layout --lib -- --list)"
while IFS= read -r test_name; do
  grep -Fx "$test_name: test" >/dev/null <<<"$native_tests"
done < <(jq -r '.subsets[].native_tests[]?' "$manifest" | sort -u)
while IFS= read -r test_name; do
  grep -Fx "$test_name: test" >/dev/null <<<"$layout_tests"
done < <(jq -r '.subsets[].layout_tests[]?' "$manifest" | sort -u)

echo "layout pinned WPT-derived subset passed"
