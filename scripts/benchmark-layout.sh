#!/usr/bin/env bash
set -euo pipefail

# Full-plan generation admission, including direct invocation.
python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
expected_worker_protocol="$(python3 "$repo_root/scripts/native_protocol_versions.py" --field worker)"
expected_context_protocol="$(python3 "$repo_root/scripts/native_protocol_versions.py" --field context)"
expected_agent_protocol="$(python3 "$repo_root/scripts/native_protocol_versions.py" --field agent)"
output_dir="${1:-$repo_root/artifacts/layout-v1/production-benchmarks}"
parent="$repo_root/target/release/oryn"
worker="$repo_root/target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker"
nodes="${ORYN_LAYOUT_BENCHMARK_NODES:-400}"
samples="${ORYN_LAYOUT_BENCHMARK_SAMPLES:-20}"
mkdir -p "$output_dir"

test -x "$parent"
test -x "$worker"
export ORYN_PAGE_WORKER="$worker"

python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"
runtime="$($parent native --runtime-info --allow-loopback)"
jq -e --argjson expected_worker_protocol "$expected_worker_protocol" --argjson expected_context_protocol "$expected_context_protocol" --argjson expected_agent_protocol "$expected_agent_protocol" '
  .contract_version == 4 and
  .native_v8 == true and
  .build_profile == "release" and
  .execution_mode == "sandboxed_worker" and
  .sandboxed == true and
  .sandbox_state == "app_sandbox" and
  .worker_protocol_version == $expected_worker_protocol and .context_service_protocol_version == $expected_context_protocol and .agent_ipc_protocol_version == $expected_agent_protocol and
  (.style_engine_version | length > 0) and
  (.box_engine_version | length > 0) and
  (.inline_engine_version | length > 0) and
  .embedded_font_sha256 == "b85c38ecea8a7cfb39c24e395a4007474fa5a4fc864f6ee33309eb4948d232d5"
' <<<"$runtime" >/dev/null

SAMPLES="$samples" "$repo_root/scripts/benchmark-sandbox-worker.sh" "$output_dir/process" \
  >"$output_dir/process.stdout.json"
python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"
if [[ -n "${ORYN_SECURITY_PREBUILT_CANDIDATE:-}" ]]; then
  layout_benchmark="$(python3 "$repo_root/scripts/security_prebuilt.py" path layout_benchmark)"
else
  cargo build --release --locked -p oryn-native --example layout_benchmark
  layout_benchmark="$repo_root/target/release/examples/layout_benchmark"
fi
python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"
"$layout_benchmark" "$nodes" "$samples" \
  >"$output_dir/layout.json"
jq -e '.passed == true and .nodes > 0 and .samples > 0' "$output_dir/layout.json" >/dev/null

jq -n \
  --arg recorded_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg source_commit "$(git -C "$repo_root" rev-parse HEAD)" \
  --arg source_tree_sha256 "$("$repo_root/scripts/source-tree-hash.sh")" \
  --argjson runtime "$runtime" \
  --slurpfile process "$output_dir/process/result.json" \
  --slurpfile layout "$output_dir/layout.json" \
  '{
    schema_version: 1,
    suite: "oryn2-layout-production-benchmarks",
    recorded_at: $recorded_at,
    source_commit: $source_commit,
    source_tree_sha256: $source_tree_sha256,
    runtime: $runtime,
    process: $process[0],
    layout: $layout[0],
    outcome: {status: (if $process[0].outcome.status == "passed" and $layout[0].passed then "passed" else "failed" end)}
  }' >"$output_dir/layout-benchmarks-v1.json"

jq -e '.outcome.status == "passed"' "$output_dir/layout-benchmarks-v1.json" >/dev/null
jq . "$output_dir/layout-benchmarks-v1.json"
