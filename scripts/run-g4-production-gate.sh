#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
output_dir="${1:-$repo_root/artifacts/g4-v1/production-gate}"
if [[ "$output_dir" != /* ]]; then output_dir="$repo_root/$output_dir"; fi
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
  echo "G4 production evidence requires a clean source tree" >&2
  exit 2
fi
if [[ -z "${ORYN_CODESIGN_IDENTITY:-}" ]]; then
  echo "Set ORYN_CODESIGN_IDENTITY to the trusted local Apple Development certificate SHA-1" >&2
  exit 2
fi
mkdir -p "$output_dir"

"$repo_root/scripts/build-sandboxed-worker.sh" >"$output_dir/signed-worker.log" 2>&1
test -x "$parent"
test -x "$worker"
export ORYN_PAGE_WORKER="$worker"

runtime="$($parent native --runtime-info --allow-loopback)"
jq -e '
  .contract_version == 4 and .native_v8 == true and .build_profile == "release" and
  .execution_mode == "sandboxed_worker" and .sandboxed == true and
  .sandbox_state == "app_sandbox" and .worker_protocol_version == 4 and
  .worker_team_id == "5HVA9VFF8K" and
  .style_engine_version == "stylo-declarations-0.20+oryn-cascade-v1" and
  .box_engine_version == "taffy-0.14" and
  .inline_engine_version == "parley-0.11.1" and
  .embedded_font_sha256 == "b85c38ecea8a7cfb39c24e395a4007474fa5a4fc864f6ee33309eb4948d232d5"
' <<<"$runtime" >/dev/null
jq . <<<"$runtime" >"$output_dir/runtime.json"

"$repo_root/scripts/validate-g4-manifest.sh" >"$output_dir/g4-manifest.log" 2>&1
"$repo_root/scripts/validate-g4-wpt.sh" >"$output_dir/g4-wpt.log" 2>&1
ORYN_G4_ARTIFACT_DIR="$output_dir/software" \
  "$repo_root/scripts/validate-g4-software.sh" >"$output_dir/g4-software.log" 2>&1

ORYN_BIN="$parent" ORYN_G4_ARTIFACT_DIR="$output_dir/oil" \
  "$repo_root/scripts/validate-g4-oil.sh" >"$output_dir/g4-oil.log" 2>&1
ORYN_BIN="$parent" ORYN_G4_ARTIFACT_DIR="$output_dir/limitations" \
  "$repo_root/scripts/validate-g4-limitations.sh" >"$output_dir/g4-limitations.log" 2>&1
"$repo_root/scripts/validate-g4-differential.sh" "$output_dir/differential" \
  >"$output_dir/g4-differential.log" 2>&1
cargo run --release --locked -p oryn-native --example g4_layout_probe \
  >"$output_dir/g4-layout.json"
jq -e '.outcome.status == "passed" and all(.relations[]; . == true or type == "number")' \
  "$output_dir/g4-layout.json" >/dev/null

"$repo_root/scripts/validate-g3-fixtures.sh" >"$output_dir/g3-fixtures.log" 2>&1
ORYN_BIN="$parent" ORYN_G3_ARTIFACT_DIR="$output_dir/g3-controlled" \
  "$repo_root/scripts/validate-g3-controlled-regressions.sh" >"$output_dir/g3-controlled.log" 2>&1
ORYN_BIN="$parent" ORYN_G3_DOMAIN=both ORYN_G3_ARTIFACT_DIR="$output_dir/g3-frameworks" \
  "$repo_root/scripts/validate-g3-frameworks.sh" >"$output_dir/g3-frameworks.log" 2>&1
ORYN_BIN="$parent" ORYN_G3_DOMAIN=both ORYN_G3_ARTIFACT_DIR="$output_dir/g3-oauth" \
  "$repo_root/scripts/validate-g3-oauth.sh" >"$output_dir/g3-oauth.log" 2>&1
"$repo_root/scripts/validate-benchmark-corpus.sh" >"$output_dir/corpus.log" 2>&1
"$repo_root/scripts/validate-g1-manifest.sh" --gate >"$output_dir/g1.log" 2>&1
"$repo_root/scripts/validate-g2-controlled.sh" >"$output_dir/g2-controlled.log" 2>&1

cargo run --release --locked -p oryn-native --example g5a_worker_integration \
  >"$output_dir/worker-integration.txt"
cargo run --release --locked -p oryn-native --example g2r_churn >"$output_dir/churn.json"
jq -e '.schema_version == 4 and .false_preservation.numerator == 0 and .worker_replacement.status == "passed" and .status == "passed"' \
  "$output_dir/churn.json" >/dev/null

test -d "$miniwob_checkout/miniwob/html"
test "$(git -C "$miniwob_checkout" rev-parse HEAD)" = "33c3b4ddef8c6eb67c57a29663d844b1eda7e614"
test -z "$(git -C "$miniwob_checkout" status --porcelain)"
(
  cd "$repo_root/intentgym"
  if [[ -x "$poetry" ]]; then
    "$poetry" run python scripts/serve_g2_miniwob.py --checkout "$miniwob_checkout" --port "$miniwob_port"
  else
    poetry run python scripts/serve_g2_miniwob.py --checkout "$miniwob_checkout" --port "$miniwob_port"
  fi
) >"$output_dir/miniwob-server.log" 2>&1 &
miniwob_pid="$!"
for _ in $(seq 1 100); do
  curl --fail --silent "http://127.0.0.1:$miniwob_port/.well-known/oryn-g2-fixture.json" \
    >"$output_dir/miniwob-fixture.json" 2>/dev/null && break
  sleep 0.1
done
curl --fail --silent "http://127.0.0.1:$miniwob_port/.well-known/oryn-g2-fixture.json" >/dev/null
(
  cd "$repo_root/intentgym"
  if [[ -x "$poetry" ]]; then runner=("$poetry" run python); else runner=(poetry run python); fi
  "${runner[@]}" scripts/run_g2_deterministic.py --domain native --oryn-binary "$parent" \
    --miniwob-url "http://127.0.0.1:$miniwob_port" --output "$output_dir/miniwob"
)
jq -e '.cells == 24 and .native == {"completed":24,"passed":24} and .outcome.status == "passed"' \
  "$output_dir/miniwob/aggregate.json" >/dev/null

"$repo_root/scripts/benchmark-g4-production.sh" "$output_dir/benchmarks" \
  >"$output_dir/benchmarks.stdout.json"

jq -n \
  --arg recorded_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg source_commit "$(git -C "$repo_root" rev-parse HEAD)" \
  --arg source_tree_sha256 "$("$repo_root/scripts/source-tree-hash.sh")" \
  --argjson runtime "$runtime" \
  --slurpfile churn "$output_dir/churn.json" \
  --slurpfile miniwob "$output_dir/miniwob/aggregate.json" \
  --slurpfile benchmarks "$output_dir/benchmarks/g4-benchmarks-v1.json" \
  '{schema_version:1,suite:"oryn2-g4-production-gate",recorded_at:$recorded_at,
    source_commit:$source_commit,source_tree_sha256:$source_tree_sha256,runtime:$runtime,
    g4:{required_passed:28,required_total:28,limitations_reported:5,limitations_total:5},
    regressions:{g1:"passed",g2_controlled:"passed",g3_frameworks:"passed",g3_oauth:"passed",native_miniwob:"24/24"},
    churn:$churn[0],native_miniwob:$miniwob[0],benchmarks:$benchmarks[0],outcome:{status:"passed"}}' \
  >"$output_dir/production-gate.json"

result="$output_dir/g4-result-v1.json"
"$repo_root/scripts/build_g4_result.py" --repo-root "$repo_root" --artifact-root "$output_dir" --output "$result"
"$repo_root/scripts/validate-g4-result-v1.sh" "$result" --artifact-root "$output_dir" --require-raw
jq . "$output_dir/production-gate.json"
