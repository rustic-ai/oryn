#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
output_dir="${1:-$repo_root/artifacts/g5-v1/production-gate}"
if [[ "$output_dir" != /* ]]; then output_dir="$repo_root/$output_dir"; fi
parent="$repo_root/target/release/oryn"
worker="$repo_root/target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker"

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
  echo "G5 production evidence requires macOS ARM64" >&2
  exit 2
fi
if [[ -n "$(git -C "$repo_root" status --porcelain --untracked-files=all)" ]]; then
  echo "G5 production evidence requires a clean source tree" >&2
  exit 2
fi
if [[ -z "${ORYN_CODESIGN_IDENTITY:-}" ]]; then
  echo "Set ORYN_CODESIGN_IDENTITY to the trusted local Apple Development certificate SHA-1" >&2
  exit 2
fi
mkdir -p "$output_dir"

"$repo_root/scripts/run-g4-production-gate.sh" "$output_dir/g4" \
  >"$output_dir/g4-production.log" 2>&1
test -x "$parent"
test -x "$worker"
export ORYN_PAGE_WORKER="$worker"

runtime="$($parent native --runtime-info --allow-loopback)"
jq -e '
  .contract_version == 4 and .native_v8 == true and .build_profile == "release" and
  .execution_mode == "sandboxed_worker" and .sandboxed == true and
  .sandbox_state == "app_sandbox" and .worker_protocol_version == 4 and
  .worker_team_id == "5HVA9VFF8K" and
  .limits.v8_heap_bytes == 268435456 and
  .limits.worker_rss_bytes == 536870912 and
  .limits.ipc_frame_bytes == 25165824 and
  .limits.queue_capacity == 32
' <<<"$runtime" >/dev/null
jq . <<<"$runtime" >"$output_dir/runtime.json"

"$repo_root/scripts/test-worker-rejections.sh" \
  >"$output_dir/worker-rejections.log" 2>&1
cargo run --release --locked -p oryn-native --example g5_security_probe \
  >"$output_dir/security-probe.json"
jq -e '.outcome.status == "passed"' "$output_dir/security-probe.json" >/dev/null

cargo run --release --locked -p oryn-native --example g2r_churn \
  >"$output_dir/churn.json"
jq -e '
  .schema_version == 4 and .status == "passed" and
  .false_preservation.numerator == 0 and
  .worker_replacement.status == "passed" and
  .worker_replacement.infinite_loop_terminated == true and
  .worker_replacement.parent_survived == true and
  .worker_replacement.old_reference_invalidated == true
' "$output_dir/churn.json" >/dev/null

ORYN_G5_SKIP_G4=1 ORYN_G5_ARTIFACT_DIR="$output_dir/software" \
  "$repo_root/scripts/validate-g5-software.sh" \
  >"$output_dir/g5-software.log" 2>&1
"$repo_root/scripts/run-g5-fuzz-smoke.sh" "$output_dir/fuzz" \
  >"$output_dir/fuzz.log" 2>&1
ORYN_CHURN_RESULT="$output_dir/churn.json" \
  "$repo_root/scripts/benchmark-g5a-production.sh" "$output_dir/benchmarks" \
  >"$output_dir/benchmarks.stdout.json"

jq -n \
  --arg recorded_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg source_commit "$(git -C "$repo_root" rev-parse HEAD)" \
  --arg source_tree_sha256 "$("$repo_root/scripts/source-tree-hash.sh")" \
  --argjson runtime "$runtime" \
  --slurpfile security "$output_dir/security-probe.json" \
  --slurpfile churn "$output_dir/churn.json" \
  --slurpfile fuzz "$output_dir/fuzz/summary.json" \
  --slurpfile benchmark "$output_dir/benchmarks/result.json" \
  --slurpfile g4 "$output_dir/g4/g4-result-v1.json" \
  '{schema_version:1,suite:"oryn2-g5-production-gate",recorded_at:$recorded_at,
    source_commit:$source_commit,source_tree_sha256:$source_tree_sha256,runtime:$runtime,
    security:$security[0],churn:$churn[0],fuzz:$fuzz[0],benchmark:$benchmark[0],
    regressions:{g4:$g4[0].outcome.status},
    outcome:{status:(if $security[0].outcome.status == "passed"
      and $churn[0].status == "passed" and $churn[0].false_preservation.numerator == 0
      and $fuzz[0].outcome.status == "passed" and $fuzz[0].crashes == 0
      and $benchmark[0].outcome.status == "passed" and $g4[0].outcome.status == "passed"
      then "passed" else "failed" end)}}' >"$output_dir/production-gate.json"
jq -e '.outcome.status == "passed"' "$output_dir/production-gate.json" >/dev/null

result="$output_dir/g5-result-v1.json"
"$repo_root/scripts/build_g5_result.py" \
  --repo-root "$repo_root" --artifact-root "$output_dir" --output "$result"
"$repo_root/scripts/validate-g5-result-v1.sh" "$result" \
  --artifact-root "$output_dir" --require-raw
jq . "$output_dir/production-gate.json"
