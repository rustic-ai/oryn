#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
output_dir="${1:-$repo_root/artifacts/rendering-v1/production-gate}"
if [[ "$output_dir" != /* ]]; then output_dir="$repo_root/$output_dir"; fi
parent="$repo_root/target/release/oryn"
worker="$repo_root/target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker"
renderer="$repo_root/target/release/oryn-render"

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
  echo "rendering production evidence requires macOS ARM64" >&2
  exit 2
fi
if [[ -n "$(git -C "$repo_root" status --porcelain --untracked-files=all)" ]]; then
  echo "rendering production evidence requires a clean source tree" >&2
  exit 2
fi
if [[ -z "${ORYN_CODESIGN_IDENTITY:-}" ]]; then
  echo "Set ORYN_CODESIGN_IDENTITY to the trusted local Apple Development certificate SHA-1" >&2
  exit 2
fi
mkdir -p "$output_dir/probe" "$output_dir/cli" "$output_dir/software" "$output_dir/benchmark"

"$repo_root/scripts/run-execution-replay-suite.sh" "$output_dir/execution-replay" \
  >"$output_dir/execution-replay-production.log" 2>&1
cp "$output_dir/execution-replay/runtime.json" "$output_dir/runtime.json"
export ORYN_PAGE_WORKER="$worker"

cargo build --release --locked -p oryn-render >"$output_dir/renderer-build.log" 2>&1
test -x "$renderer"
cargo run --quiet --release --locked -p oryn --example rendering_probe --features render -- \
  "$output_dir/probe" >"$output_dir/rendering-probe.json"
jq -e '.outcome.status == "passed"' "$output_dir/rendering-probe.json" >/dev/null

cp "$repo_root/benchmarks/rendering/fixtures/paint-basic.html" "$output_dir/cli/fixture.html"
"$renderer" --html "$output_dir/cli/fixture.html" --png "$output_dir/cli/cli.png" \
  --pdf "$output_dir/cli/cli.pdf" >"$output_dir/cli/cli-output.json"
jq -e '.status == "rendered"' "$output_dir/cli/cli-output.json" >/dev/null
file "$output_dir/cli/cli.png" "$output_dir/cli/cli.pdf" >"$output_dir/cli/file-types.log"
grep -q 'PNG image data' "$output_dir/cli/file-types.log"
grep -q 'PDF document' "$output_dir/cli/file-types.log"

"$repo_root/scripts/validate-rendering-differential.sh" "$output_dir/probe/rendering-probe.png" \
  "$output_dir/differential" >"$output_dir/differential.log" 2>&1
ORYN_RENDERING_SKIP_EXECUTION_REPLAY=1 ORYN_RENDERING_ARTIFACT_DIR="$output_dir/software" \
  "$repo_root/scripts/validate-rendering-software.sh" >"$output_dir/software.log" 2>&1

python3 "$repo_root/scripts/benchmark_rendering.py" --renderer "$renderer" --parent "$parent" \
  --worker "$worker" --fixture "$output_dir/cli/fixture.html" --output-dir "$output_dir/benchmark" \
  >"$output_dir/benchmark.json"
jq '{parent_bytes:.sizes.parent.bytes,worker_bytes:.sizes.worker.bytes,renderer_bytes:.sizes.renderer.bytes,parent_gzip_bytes:.sizes.parent.gzip_bytes,worker_gzip_bytes:.sizes.worker.gzip_bytes,renderer_gzip_bytes:.sizes.renderer.gzip_bytes}' \
  "$output_dir/benchmark.json" >"$output_dir/sizes.json"

jq -n \
  --arg recorded_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg source_commit "$(git -C "$repo_root" rev-parse HEAD)" \
  --arg source_tree_sha256 "$("$repo_root/scripts/source-tree-hash.sh")" \
  --slurpfile regressions "$output_dir/execution-replay/production-gate.json" \
  --slurpfile probe "$output_dir/rendering-probe.json" \
  --slurpfile differential "$output_dir/differential/differential.json" \
  --slurpfile software "$output_dir/software/summary.json" \
  '{schema_version:1,suite:"oryn2-rendering-production",recorded_at:$recorded_at,source_commit:$source_commit,source_tree_sha256:$source_tree_sha256,regressions:$regressions[0].outcome.status,probe:$probe[0].outcome.status,differential:$differential[0].outcome.status,software:$software[0].outcome.status,outcome:{status:(if $regressions[0].outcome.status == "passed" and $probe[0].outcome.status == "passed" and $differential[0].outcome.status == "passed" and $software[0].outcome.status == "passed" then "passed" else "failed" end)}}' \
  >"$output_dir/production-gate.json"
jq -e '.outcome.status == "passed"' "$output_dir/production-gate.json" >/dev/null

result="$output_dir/rendering-result-v1.json"
python3 "$repo_root/scripts/build_rendering_result.py" --repo-root "$repo_root" \
  --artifact-root "$output_dir" --output "$result"
"$repo_root/scripts/validate-rendering-result.sh" "$result" --artifact-root "$output_dir" --require-raw
jq . "$output_dir/production-gate.json"
