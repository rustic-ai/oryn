#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$repo_root/benchmarks/g3/wpt-subset-v1.json"

jq -e '
  .schema_version == 1 and .locked == true and
  (.upstream.project == "web-platform-tests/wpt") and
  (.upstream.revision | test("^[0-9a-f]{40}$")) and
  ([.subsets[].category] | sort == ["dom_events","fetch_streams","file_fetch","indexeddb","shadow_custom_elements","storage","url_encoding","websocket","xhr"]) and
  all(.subsets[]; (.sources | length) > 0 and (.native_tests | length) > 0)
' "$manifest" >/dev/null

# The Rust test names are the immutable executable form of the curated vectors.
# Running the complete V8 library suite also catches interactions between them.
cargo test --manifest-path "$repo_root/Cargo.toml" -p oryn-native \
  --features v8-host,in-process-probe --lib
cargo test --manifest-path "$repo_root/Cargo.toml" -p oryn-webidl

test_list="$(cargo test --manifest-path "$repo_root/Cargo.toml" -p oryn-native \
  --features v8-host,in-process-probe --lib -- --list)"
while IFS= read -r test_name; do
  grep -Fx "$test_name: test" >/dev/null <<<"$test_list"
done < <(jq -r '.subsets[].native_tests[]' "$manifest" | sort -u)

echo "G3 pinned WPT-derived subset passed"
