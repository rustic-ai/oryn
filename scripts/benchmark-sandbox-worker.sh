#!/usr/bin/env bash
set -euo pipefail

# Full-plan generation admission, including direct invocation.
python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
expected_worker_protocol="$(python3 "$repo_root/scripts/native_protocol_versions.py" --field worker)"
expected_context_protocol="$(python3 "$repo_root/scripts/native_protocol_versions.py" --field context)"
expected_agent_protocol="$(python3 "$repo_root/scripts/native_protocol_versions.py" --field agent)"
output_dir="${1:-$repo_root/artifacts/semantic-sandbox-v4/production}"
samples="${SAMPLES:-30}"
parent="$repo_root/target/release/oryn"
app="$repo_root/target/release/OrynPageWorker.app"
worker="$app/Contents/MacOS/oryn-page-worker"
fixture="$repo_root/benchmarks/conformance/javascript-mutation-smoke.html"
mkdir -p "$output_dir"
source_clean=true
if [[ -n "$(git -C "$repo_root" status --porcelain)" ]]; then
  source_clean=false
  if [[ "${ORYN_ALLOW_DIRTY_BENCHMARK:-0}" != "1" ]]; then
    echo "Refusing to publish performance evidence from a dirty source tree" >&2
    exit 2
  fi
fi

parent_pid=""
fifo=""
compressed_bundle=""
cleanup() {
  if [[ -n "$parent_pid" ]]; then
    kill "$parent_pid" 2>/dev/null || true
  fi
  if [[ -n "$fifo" ]]; then
    rm -f "$fifo"
  fi
  if [[ -n "$compressed_bundle" ]]; then
    rm -f "$compressed_bundle"
  fi
}
trap cleanup EXIT

test -x "$parent"
test -x "$worker"
export ORYN_PAGE_WORKER="$worker"

python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"
runtime="$("$parent" native --runtime-info --allow-loopback)"
signature="$(codesign -dvvv "$worker" 2>&1)"
entitlements_file="$output_dir/worker-entitlements.plist"
codesign -d --entitlements :- "$worker" >"$entitlements_file" 2>/dev/null

: >"$output_dir/startup-handshake-ms.txt"
: >"$output_dir/startup-mutation-ms.txt"
protocol_input="$output_dir/mutation-input.txt"
printf '%s\n' 'click "Mutate"' exit >"$protocol_input"
for _ in $(seq 1 "$samples"); do
  python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"
  /usr/bin/time -p "$parent" native --runtime-info --allow-loopback \
    >"$output_dir/runtime.json" 2>"$output_dir/time.txt"
  awk '/^real / {print $2 * 1000}' "$output_dir/time.txt" \
    >>"$output_dir/startup-handshake-ms.txt"
  python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"
  /usr/bin/time -p "$parent" native --html "$fixture" \
    <"$protocol_input" >"$output_dir/mutation.jsonl" 2>"$output_dir/time.txt"
  awk '/^real / {print $2 * 1000}' "$output_dir/time.txt" \
    >>"$output_dir/startup-mutation-ms.txt"
done

python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"
if [[ -n "${ORYN_SECURITY_PREBUILT_CANDIDATE:-}" ]]; then
  page_benchmark="$(python3 "$repo_root/scripts/security_prebuilt.py" path sandbox_page_benchmark)"
else
  cargo build --release --locked -p oryn-native --example sandbox_page_benchmark
  page_benchmark="$repo_root/target/release/examples/sandbox_page_benchmark"
fi
python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"
"$page_benchmark" 5 10000 \
  >"$output_dir/isolated-page-creation.json"
ORYN_BENCH_WORKER_MODE=isolated-prewarmed \
  "$page_benchmark" 5 10000 \
  >"$output_dir/prewarmed-page-creation.json"
ORYN_BENCH_WORKER_MODE=recycled \
  "$page_benchmark" 5 10000 \
  >"$output_dir/recycled-page-creation.json"
python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"
if [[ -n "${ORYN_SECURITY_PREBUILT_CANDIDATE:-}" ]]; then
  group_benchmark="$(python3 "$repo_root/scripts/security_prebuilt.py" path page_group_benchmark)"
else
  cargo build --release --locked -p oryn-native --example page_group_benchmark
  group_benchmark="$repo_root/target/release/examples/page_group_benchmark"
fi
for page_count in 1 5 20; do
  python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"
  "$group_benchmark" "$page_count" \
    >"$output_dir/concurrent-${page_count}.json"
done

fifo="$output_dir/live-input.fifo"
mkfifo "$fifo"
exec 9<>"$fifo"
"$parent" native --html "$fixture" <"$fifo" \
  >"$output_dir/live.jsonl" 2>"$output_dir/live.stderr" &
parent_pid="$!"
worker_pid=""
for _ in $(seq 1 100); do
  worker_pid="$(pgrep -P "$parent_pid" -f oryn-page-worker | head -1 || true)"
  [[ -n "$worker_pid" ]] && break
  sleep 0.05
done
if [[ -z "$worker_pid" ]]; then
  echo "Unable to observe the live page worker" >&2
  exit 1
fi
parent_rss_bytes="$(( $(ps -o rss= -p "$parent_pid" | tr -d '[:space:]') * 1024 ))"
worker_rss_bytes="$(( $(ps -o rss= -p "$worker_pid" | tr -d '[:space:]') * 1024 ))"
printf '%s\n' exit >&9
exec 9>&-
wait "$parent_pid"
parent_pid=""
rm -f "$fifo"
fifo=""

parent_bytes="$(wc -c <"$parent" | tr -d '[:space:]')"
worker_bytes="$(wc -c <"$worker" | tr -d '[:space:]')"
app_bytes="$(find "$app" -type f -exec stat -f '%z' {} + | awk '{sum += $1} END {print sum}')"
combined_bytes="$((parent_bytes + app_bytes))"
parent_gzip_bytes="$(gzip -c "$parent" | wc -c | tr -d '[:space:]')"
worker_gzip_bytes="$(gzip -c "$worker" | wc -c | tr -d '[:space:]')"
compressed_bundle="$(mktemp -t oryn-sandbox-worker-bundle.XXXXXX.tar.gz)"
tar -C "$repo_root/target/release" -czf "$compressed_bundle" \
  oryn OrynPageWorker.app
combined_gzip_bytes="$(wc -c <"$compressed_bundle" | tr -d '[:space:]')"

percentile() {
  local file="$1"
  local fraction="$2"
  sort -n "$file" | awk -v fraction="$fraction" '
    {value[NR] = $1}
    END {
      position = int((NR - 1) * fraction) + 1
      print value[position]
    }
  '
}

isolated_page_creation="$(cat "$output_dir/isolated-page-creation.json")"
prewarmed_page_creation="$(cat "$output_dir/prewarmed-page-creation.json")"
recycled_page_creation="$(cat "$output_dir/recycled-page-creation.json")"
concurrency="$(jq -s '.' "$output_dir/concurrent-1.json" "$output_dir/concurrent-5.json" "$output_dir/concurrent-20.json")"
crash_restart='null'
churn_result="${ORYN_CHURN_RESULT:-$repo_root/artifacts/semantic-sandbox-v4/reference-churn.json}"
if [[ -f "$churn_result" ]]; then
  crash_restart="$(jq '.worker_replacement' "$churn_result")"
fi

jq -n --argjson expected_worker_protocol "$expected_worker_protocol" --argjson expected_context_protocol "$expected_context_protocol" --argjson expected_agent_protocol "$expected_agent_protocol" \
  --arg captured_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg source_commit "$(git -C "$repo_root" rev-parse HEAD)" \
  --arg source_tree_sha256 "$("$repo_root/scripts/source-tree-hash.sh")" \
  --arg parent_sha256 "$(shasum -a 256 "$parent" | awk '{print $1}')" \
  --arg worker_sha256 "$(shasum -a 256 "$worker" | awk '{print $1}')" \
  --arg signature_sha256 "$(printf '%s' "$signature" | shasum -a 256 | awk '{print $1}')" \
  --arg entitlements_sha256 "$(shasum -a 256 "$entitlements_file" | awk '{print $1}')" \
  --argjson runtime "$runtime" \
  --argjson parent_bytes "$parent_bytes" \
  --argjson worker_bytes "$worker_bytes" \
  --argjson app_bytes "$app_bytes" \
  --argjson combined_bytes "$combined_bytes" \
  --argjson parent_gzip_bytes "$parent_gzip_bytes" \
  --argjson worker_gzip_bytes "$worker_gzip_bytes" \
  --argjson combined_gzip_bytes "$combined_gzip_bytes" \
  --argjson samples "$samples" \
  --argjson source_clean "$source_clean" \
  --argjson handshake_p50_ms "$(percentile "$output_dir/startup-handshake-ms.txt" 0.50)" \
  --argjson handshake_p95_ms "$(percentile "$output_dir/startup-handshake-ms.txt" 0.95)" \
  --argjson mutation_p50_ms "$(percentile "$output_dir/startup-mutation-ms.txt" 0.50)" \
  --argjson mutation_p95_ms "$(percentile "$output_dir/startup-mutation-ms.txt" 0.95)" \
  --argjson isolated_page_creation "$isolated_page_creation" \
  --argjson prewarmed_page_creation "$prewarmed_page_creation" \
  --argjson recycled_page_creation "$recycled_page_creation" \
  --argjson concurrency "$concurrency" \
  --argjson parent_rss_bytes "$parent_rss_bytes" \
  --argjson worker_rss_bytes "$worker_rss_bytes" \
  --argjson crash_restart "$crash_restart" \
  '{
    schema_version: 5,
    run_id: "sandbox-production-macos-arm64",
    captured_at: $captured_at,
    source_commit: $source_commit,
    source_tree_sha256: $source_tree_sha256,
    publishable: $source_clean,
    runtime: $runtime,
    artifacts: {
      parent: {
        sha256: $parent_sha256,
        installed_bytes: $parent_bytes,
        gzip_bytes: $parent_gzip_bytes
      },
      worker: {
        sha256: $worker_sha256,
        installed_bytes: $worker_bytes,
        gzip_bytes: $worker_gzip_bytes,
        signature_sha256: $signature_sha256,
        entitlements_sha256: $entitlements_sha256
      },
      worker_app_bytes: $app_bytes,
      combined_installed_bytes: $combined_bytes,
      combined_compressed_bytes: $combined_gzip_bytes
    },
    measurements: {
      samples: $samples,
      startup_to_handshake_p50_ms: $handshake_p50_ms,
      startup_to_handshake_p95_ms: $handshake_p95_ms,
      startup_to_mutation_p50_ms: $mutation_p50_ms,
      startup_to_mutation_p95_ms: $mutation_p95_ms,
      isolated: $isolated_page_creation,
      prewarmed: $prewarmed_page_creation,
      recycled: $recycled_page_creation,
      concurrency: $concurrency,
      parent_rss_bytes: $parent_rss_bytes,
      worker_rss_bytes: $worker_rss_bytes,
      crash_restart: $crash_restart
    },
    outcome: {
      status: (
        ($concurrency | map(.worker_rss_bytes[]) | sort) as $rss_samples
        | if $runtime.execution_mode == "sandboxed_worker"
          and $runtime.sandboxed
          and $runtime.native_v8
          and $runtime.worker_protocol_version == $expected_worker_protocol and $runtime.context_service_protocol_version == $expected_context_protocol and $runtime.agent_ipc_protocol_version == $expected_agent_protocol
          and $worker_rss_bytes <= $runtime.limits.worker_rss_bytes
          and $isolated_page_creation.first_cold_verification_launch_handshake_ms <= 250
          and $isolated_page_creation.warm_actions_per_second >= 300
          and ($prewarmed_page_creation.cached_isolated_launch_ms | all(. <= 50))
          and ($recycled_page_creation.cached_isolated_launch_ms | all(. <= 50))
          and ($recycled_page_creation.reset_or_close_ms | all(. <= 50))
          and $recycled_page_creation.recycled_javascript_state_erased
          and ($concurrency | all(.passed))
          and ($rss_samples | length > 0)
          and $rss_samples[(length / 2 | floor)] <= 67108864
          and ($rss_samples | all(. <= 167772160))
        then "passed"
        else "failed"
        end
      )
    }
  }' >"$output_dir/result.json"

jq -e '.outcome.status == "passed"' "$output_dir/result.json" >/dev/null
jq . "$output_dir/result.json"
