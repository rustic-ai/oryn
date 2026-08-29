#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
output_dir="${1:-$repo_root/artifacts/app-compatibility-v1/production-gate}"
if [[ "$output_dir" != /* ]]; then
  output_dir="$repo_root/$output_dir"
fi
parent="$repo_root/target/release/oryn"
worker="$repo_root/target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker"
miniwob_checkout="${ORYN_MINIWOB_CHECKOUT:-$repo_root/artifacts/deps/miniwob-plusplus}"
miniwob_port="${ORYN_MINIWOB_PORT:-8765}"
poetry="$repo_root/artifacts/tools/poetry/bin/poetry"
miniwob_pid=""

cleanup() {
  if [[ -n "$miniwob_pid" ]]; then
    kill "$miniwob_pid" 2>/dev/null || true
    wait "$miniwob_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if [[ -n "$(git -C "$repo_root" status --porcelain --untracked-files=all)" ]]; then
  echo "application compatibility production evidence requires a clean tracked source tree" >&2
  exit 2
fi
test -x "$parent"
test -x "$worker"
test -d "$miniwob_checkout/miniwob/html"
test "$(git -C "$miniwob_checkout" rev-parse HEAD)" = \
  "33c3b4ddef8c6eb67c57a29663d844b1eda7e614"
test -z "$(git -C "$miniwob_checkout" status --porcelain)"
mkdir -p "$output_dir"
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
jq . <<<"$runtime" >"$output_dir/runtime.json"

"$repo_root/scripts/validate-app-compatibility-manifest.sh" >"$output_dir/app-manifest.log" 2>&1
"$repo_root/scripts/validate-app-fixtures.sh" >"$output_dir/app-fixtures.log" 2>&1
"$repo_root/scripts/validate-app-platform-subset.sh" >"$output_dir/platform-subset.log" 2>&1
ORYN_APP_COMPATIBILITY_ARTIFACT_DIR="$output_dir/software" \
  "$repo_root/scripts/validate-app-compatibility-software.sh" >"$output_dir/app-software.log" 2>&1

ORYN_BIN="$parent" \
  ORYN_APP_COMPATIBILITY_ARTIFACT_DIR="$output_dir/controlled" \
  "$repo_root/scripts/validate-app-controlled-regressions.sh"
ORYN_BIN="$parent" \
  ORYN_APP_COMPATIBILITY_DOMAIN=both \
  ORYN_APP_COMPATIBILITY_ARTIFACT_DIR="$output_dir/frameworks" \
  "$repo_root/scripts/validate-frameworks.sh"
ORYN_BIN="$parent" \
  ORYN_APP_COMPATIBILITY_DOMAIN=both \
  ORYN_APP_COMPATIBILITY_ARTIFACT_DIR="$output_dir/oauth" \
  "$repo_root/scripts/validate-auth-flows.sh"
ORYN_BIN="$parent" \
  ORYN_APP_COMPATIBILITY_ARTIFACT_DIR="$output_dir/limitations" \
  "$repo_root/scripts/validate-app-limitations.sh"

"$repo_root/scripts/validate-benchmark-corpus.sh" >"$output_dir/corpus.log" 2>&1
"$repo_root/scripts/validate-executable-document-manifest.sh" --gate >"$output_dir/executable-document.log" 2>&1
"$repo_root/scripts/validate-semantic-controlled.sh" >"$output_dir/semantic-controlled.log" 2>&1

cargo run --release --locked -p oryn-native --example sandbox_worker_integration \
  >"$output_dir/worker-integration.txt"
cargo run --release --locked -p oryn-native --example semantic_reference_churn \
  >"$output_dir/churn.json"
jq -e '
  .schema_version == 4 and
  .false_preservation.numerator == 0 and
  .recovery.value == 1 and
  .worker_replacement.status == "passed" and
  .status == "passed"
' "$output_dir/churn.json" >/dev/null

(
  cd "$repo_root/intentgym"
  if [[ -x "$poetry" ]]; then
    "$poetry" run python scripts/serve_deterministic_miniwob.py \
      --checkout "$miniwob_checkout" --port "$miniwob_port"
  else
    poetry run python scripts/serve_deterministic_miniwob.py \
      --checkout "$miniwob_checkout" --port "$miniwob_port"
  fi
) >"$output_dir/miniwob-server.log" 2>&1 &
miniwob_pid="$!"
for _ in $(seq 1 100); do
  curl --fail --silent \
    "http://127.0.0.1:$miniwob_port/.well-known/oryn-deterministic-fixture.json" \
    >"$output_dir/miniwob-fixture.json" 2>/dev/null && break
  sleep 0.1
done
curl --fail --silent \
  "http://127.0.0.1:$miniwob_port/.well-known/oryn-deterministic-fixture.json" >/dev/null
(
  cd "$repo_root/intentgym"
  if [[ -x "$poetry" ]]; then
    "$poetry" run python scripts/run_semantic_deterministic.py \
      --domain native \
      --oryn-binary "$parent" \
      --miniwob-url "http://127.0.0.1:$miniwob_port" \
      --output "$output_dir/miniwob"
  else
    poetry run python scripts/run_semantic_deterministic.py \
      --domain native \
      --oryn-binary "$parent" \
      --miniwob-url "http://127.0.0.1:$miniwob_port" \
      --output "$output_dir/miniwob"
  fi
)
jq -e '
  .cells == 24 and
  .native.completed == 24 and
  .native.passed == 24 and
  .outcome.status == "passed"
' "$output_dir/miniwob/aggregate.json" >/dev/null

"$repo_root/scripts/benchmark-app-compatibility.sh" "$output_dir/benchmarks" \
  >"$output_dir/benchmarks.stdout.json"

jq -n \
  --arg recorded_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg source_commit "$(git -C "$repo_root" rev-parse HEAD)" \
  --arg source_tree_sha256 "$("$repo_root/scripts/source-tree-hash.sh")" \
  --argjson runtime "$runtime" \
  --slurpfile churn "$output_dir/churn.json" \
  --slurpfile miniwob "$output_dir/miniwob/aggregate.json" \
  --slurpfile benchmarks "$output_dir/benchmarks/app-compatibility-benchmarks-v1.json" \
  '{
    schema_version: 1,
    suite: "oryn2-app-compatibility-production",
    recorded_at: $recorded_at,
    source_commit: $source_commit,
    source_tree_sha256: $source_tree_sha256,
    runtime: $runtime,
    controlled_oil: {passed: 12, total: 12},
    frameworks: {native: 8, chromium: 8},
    oauth: {native: 5, chromium: 5},
    churn: $churn[0],
    native_miniwob: $miniwob[0],
    benchmarks: $benchmarks[0],
    outcome: {status: "passed"}
  }' >"$output_dir/production-gate.json"

result="$output_dir/app-compatibility-result-v1.json"
"$repo_root/scripts/build_app_compatibility_result.py" \
  --repo-root "$repo_root" \
  --artifact-root "$output_dir" \
  --output "$result"
"$repo_root/scripts/validate-app-compatibility-result.sh" "$result" \
  --artifact-root "$output_dir" \
  --require-raw

jq . "$output_dir/production-gate.json"
