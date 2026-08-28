#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
output_dir="${1:-$repo_root/artifacts/g6-g7-v1/production-gate}"
if [[ "$output_dir" != /* ]]; then output_dir="$repo_root/$output_dir"; fi
parent="$repo_root/target/release/oryn"
worker="$repo_root/target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker"

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
  echo "G6/G7 production evidence requires macOS ARM64" >&2
  exit 2
fi
if [[ -n "$(git -C "$repo_root" status --porcelain --untracked-files=all)" ]]; then
  echo "G6/G7 production evidence requires a clean source tree" >&2
  exit 2
fi
if [[ -z "${ORYN_CODESIGN_IDENTITY:-}" ]]; then
  echo "Set ORYN_CODESIGN_IDENTITY to the trusted local Apple Development certificate SHA-1" >&2
  exit 2
fi
mkdir -p "$output_dir"

"$repo_root/scripts/run-g5-production-gate.sh" "$output_dir/g5" \
  >"$output_dir/g5-production.log" 2>&1
test -x "$parent"
test -x "$worker"
export ORYN_PAGE_WORKER="$worker"

runtime="$($parent native --runtime-info --allow-loopback)"
jq -e '.native_v8 == true and .execution_mode == "sandboxed_worker" and .sandbox_state == "app_sandbox" and .worker_protocol_version == 4' \
  <<<"$runtime" >/dev/null
jq . <<<"$runtime" >"$output_dir/runtime.json"

cargo run --release --locked -p oryn-domain --example g6_gate_probe \
  >"$output_dir/g6-probe.json"
jq -e '.outcome.status == "passed"' "$output_dir/g6-probe.json" >/dev/null
cargo run --release --locked -p oryn-native --example g7_gate_probe \
  >"$output_dir/g7-probe.json"
jq -e '.outcome.status == "passed"' "$output_dir/g7-probe.json" >/dev/null

ORYN_G6_G7_SKIP_G5=1 ORYN_G6_G7_ARTIFACT_DIR="$output_dir/software" \
  "$repo_root/scripts/validate-g6-g7-software.sh" \
  >"$output_dir/software.log" 2>&1
"$parent" native --help >"$output_dir/cli-help.log"
for flag in trace-out checkpoint-out restore-checkpoint replay-trace failure-artifact-out replay-failure; do
  grep -q -- "--$flag" "$output_dir/cli-help.log"
done

jq -n \
  --arg recorded_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg source_commit "$(git -C "$repo_root" rev-parse HEAD)" \
  --arg source_tree_sha256 "$("$repo_root/scripts/source-tree-hash.sh")" \
  --slurpfile g5 "$output_dir/g5/g5-result-v1.json" \
  --slurpfile g6 "$output_dir/g6-probe.json" \
  --slurpfile g7 "$output_dir/g7-probe.json" \
  --slurpfile software "$output_dir/software/summary.json" \
  '{schema_version:1,suite:"oryn2-g6-g7-production-gate",recorded_at:$recorded_at,
    source_commit:$source_commit,source_tree_sha256:$source_tree_sha256,
    regressions:{g5:$g5[0].outcome.status},g6:$g6[0].outcome.status,g7:$g7[0].outcome.status,
    software:$software[0].outcome.status,
    outcome:{status:(if $g5[0].outcome.status == "passed" and $g6[0].outcome.status == "passed"
      and $g7[0].outcome.status == "passed" and $software[0].outcome.status == "passed"
      then "passed" else "failed" end)}}' >"$output_dir/production-gate.json"
jq -e '.outcome.status == "passed"' "$output_dir/production-gate.json" >/dev/null

g6_result="$output_dir/g6-result-v1.json"
g7_result="$output_dir/g7-result-v1.json"
"$repo_root/scripts/build_g6_g7_results.py" --repo-root "$repo_root" \
  --artifact-root "$output_dir" --g6-output "$g6_result" --g7-output "$g7_result"
"$repo_root/scripts/validate-g6-result-v1.sh" "$g6_result" --artifact-root "$output_dir" --require-raw
"$repo_root/scripts/validate-g7-result-v1.sh" "$g7_result" --artifact-root "$output_dir" --require-raw
jq . "$output_dir/production-gate.json"
