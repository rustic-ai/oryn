#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
output_dir="${1:-$repo_root/artifacts/execution-replay-v1/production-gate}"
if [[ "$output_dir" != /* ]]; then output_dir="$repo_root/$output_dir"; fi
parent="$repo_root/target/release/oryn"
worker="$repo_root/target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker"

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
  echo "execution and replay production evidence requires macOS ARM64" >&2
  exit 2
fi
if [[ -n "$(git -C "$repo_root" status --porcelain --untracked-files=all)" ]]; then
  echo "execution and replay production evidence requires a clean source tree" >&2
  exit 2
fi
if [[ -z "${ORYN_CODESIGN_IDENTITY:-}" ]]; then
  echo "Set ORYN_CODESIGN_IDENTITY to the trusted local Apple Development certificate SHA-1" >&2
  exit 2
fi
mkdir -p "$output_dir"

"$repo_root/scripts/run-security-suite.sh" "$output_dir/security" \
  >"$output_dir/security-production.log" 2>&1
test -x "$parent"
test -x "$worker"
export ORYN_PAGE_WORKER="$worker"

runtime="$($parent native --runtime-info --allow-loopback)"
jq -e '.native_v8 == true and .execution_mode == "sandboxed_worker" and .sandbox_state == "app_sandbox" and .worker_protocol_version == 7' \
  <<<"$runtime" >/dev/null
jq . <<<"$runtime" >"$output_dir/runtime.json"

cargo run --release --locked -p oryn-domain --example execution_domain_probe \
  >"$output_dir/execution-domain-probe.json"
jq -e '.outcome.status == "passed"' "$output_dir/execution-domain-probe.json" >/dev/null
cargo run --release --locked -p oryn-native --example replay_checkpoint_probe \
  >"$output_dir/replay-probe.json"
jq -e '.outcome.status == "passed"' "$output_dir/replay-probe.json" >/dev/null

ORYN_EXECUTION_REPLAY_SKIP_SECURITY=1 ORYN_EXECUTION_REPLAY_ARTIFACT_DIR="$output_dir/software" \
  "$repo_root/scripts/validate-execution-replay-software.sh" \
  >"$output_dir/software.log" 2>&1
"$parent" native --help >"$output_dir/cli-help.log"
for flag in trace-out checkpoint-out restore-checkpoint replay-trace failure-artifact-out replay-failure; do
  grep -q -- "--$flag" "$output_dir/cli-help.log"
done

jq -n \
  --arg recorded_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg source_commit "$(git -C "$repo_root" rev-parse HEAD)" \
  --arg source_tree_sha256 "$("$repo_root/scripts/source-tree-hash.sh")" \
  --slurpfile security "$output_dir/security/security-result-v2.json" \
  --slurpfile execution "$output_dir/execution-domain-probe.json" \
  --slurpfile replay "$output_dir/replay-probe.json" \
  --slurpfile software "$output_dir/software/summary.json" \
  '{schema_version:1,suite:"oryn2-execution-replay-production",recorded_at:$recorded_at,
    source_commit:$source_commit,source_tree_sha256:$source_tree_sha256,
    regressions:{security:$security[0].outcome.status},execution:$execution[0].outcome.status,replay:$replay[0].outcome.status,
    software:$software[0].outcome.status,
    outcome:{status:(if $security[0].outcome.status == "passed" and $execution[0].outcome.status == "passed"
      and $replay[0].outcome.status == "passed" and $software[0].outcome.status == "passed"
      then "passed" else "failed" end)}}' >"$output_dir/production-gate.json"
jq -e '.outcome.status == "passed"' "$output_dir/production-gate.json" >/dev/null

execution_result="$output_dir/execution-domains-result-v1.json"
replay_result="$output_dir/replay-result-v1.json"
"$repo_root/scripts/build_execution_replay_results.py" --repo-root "$repo_root" \
  --artifact-root "$output_dir" --execution-domains-output "$execution_result" --replay-output "$replay_result"
"$repo_root/scripts/validate-execution-domains-result.sh" "$execution_result" --artifact-root "$output_dir" --require-raw
"$repo_root/scripts/validate-replay-result.sh" "$replay_result" --artifact-root "$output_dir" --require-raw
jq . "$output_dir/production-gate.json"
