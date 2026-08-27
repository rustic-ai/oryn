#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
output_dir="${1:-$repo_root/artifacts/g3-v1/production-benchmarks}"
parent="$repo_root/target/release/oryn"
worker="$repo_root/target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker"
pages="${G3_BENCHMARK_PAGES:-8}"
mkdir -p "$output_dir"

test -x "$parent"
test -x "$worker"
export ORYN_PAGE_WORKER="$worker"

runtime="$($parent native --runtime-info --allow-loopback)"
jq -e '
  .native_v8 == true and
  .build_profile == "release" and
  .execution_mode == "sandboxed_worker" and
  .sandboxed == true and
  .sandbox_state == "app_sandbox" and
  .worker_protocol_version == 3
' <<<"$runtime" >/dev/null

"$repo_root/scripts/benchmark-g5a-production.sh" "$output_dir/g5a" \
  >"$output_dir/g5a.stdout.json"

cargo build --release --locked -p oryn-native --example g3_page_group_benchmark
"$repo_root/target/release/examples/g3_page_group_benchmark" "$pages" \
  >"$output_dir/page-group.json"
jq -e '.passed == true' "$output_dir/page-group.json" >/dev/null

jq -n \
  --arg recorded_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg source_commit "$(git -C "$repo_root" rev-parse HEAD)" \
  --arg source_tree_sha256 "$("$repo_root/scripts/source-tree-hash.sh")" \
  --arg parent_sha256 "$(shasum -a 256 "$parent" | awk '{print $1}')" \
  --arg worker_sha256 "$(shasum -a 256 "$worker" | awk '{print $1}')" \
  --argjson runtime "$runtime" \
  --slurpfile g5a "$output_dir/g5a/result.json" \
  --slurpfile page_group "$output_dir/page-group.json" \
  '{
    schema_version: 1,
    suite: "oryn2-g3-production-benchmarks",
    recorded_at: $recorded_at,
    source_commit: $source_commit,
    source_tree_sha256: $source_tree_sha256,
    parent_sha256: $parent_sha256,
    worker_sha256: $worker_sha256,
    runtime: $runtime,
    size_startup_and_replacement: $g5a[0],
    multi_page_stream_websocket: $page_group[0],
    outcome: {
      status: (
        if $g5a[0].outcome.status == "passed"
          and $page_group[0].passed == true
        then "passed"
        else "failed"
        end
      )
    }
  }' >"$output_dir/g3-benchmarks-v1.json"

jq -e '.outcome.status == "passed"' "$output_dir/g3-benchmarks-v1.json" >/dev/null
jq . "$output_dir/g3-benchmarks-v1.json"
