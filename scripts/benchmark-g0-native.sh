#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output_dir="${1:-$repo_root/artifacts/g0-native}"
samples="${SAMPLES:-30}"
pages="${PAGES:-20}"
mkdir -p "$output_dir"

cd "$repo_root"
cargo build --release --locked -p oryn-native --bin oryn-native-probe
binary="$repo_root/target/release/oryn-native-probe"

for _ in $(seq 1 5); do "$binary" 1 "$pages" >/dev/null; done
"$binary" "$samples" "$pages" > "$output_dir/native.json"

/usr/bin/time -l "$binary" 1 "$pages" > "$output_dir/rss-probe.json" \
  2> "$output_dir/rss.txt"
wc -c "$binary" > "$output_dir/installed-size.txt"
gzip -c "$binary" | wc -c > "$output_dir/compressed-size.txt"
git rev-parse HEAD > "$output_dir/commit.txt"
sw_vers > "$output_dir/os.txt"
uname -m >> "$output_dir/os.txt"

commit="$(git rev-parse HEAD)"
cpu="$(sysctl -n machdep.cpu.brand_string)"
memory_bytes="$(sysctl -n hw.memsize)"
toolchain="$(rustc --version)"
page_creation_ms="$(jq '[.[].page_creation_ms] | sort | .[length / 2 | floor]' "$output_dir/native.json")"
load_observe_ms="$(jq '[.[].load_observe_ms] | sort | .[length / 2 | floor]' "$output_dir/native.json")"
observation_bytes="$(jq '.[0].observation_bytes' "$output_dir/native.json")"
rss_bytes="$(awk '/maximum resident set size/ {print $1}' "$output_dir/rss.txt")"
installed_bytes="$(awk '{print $1}' "$output_dir/installed-size.txt")"
compressed_bytes="$(tr -d '[:space:]' < "$output_dir/compressed-size.txt")"

jq -n \
  --arg run_id "g0-native-$(date -u +%Y%m%dT%H%M%SZ)" \
  --arg commit "$commit" \
  --arg started_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg os "$(sw_vers -productVersion)" \
  --arg arch "$(uname -m)" \
  --arg cpu "$cpu" \
  --argjson memory_bytes "$memory_bytes" \
  --arg toolchain "$toolchain" \
  --argjson page_creation_ms "$page_creation_ms" \
  --argjson load_observe_ms "$load_observe_ms" \
  --argjson observation_bytes "$observation_bytes" \
  --argjson rss_bytes "$rss_bytes" \
  --argjson installed_bytes "$installed_bytes" \
  --argjson compressed_bytes "$compressed_bytes" \
  '{
    schema_version: 1,
    run_id: $run_id,
    commit: $commit,
    started_at: $started_at,
    environment: {
      os: $os, arch: $arch, cpu: $cpu,
      memory_bytes: $memory_bytes, toolchain: $toolchain
    },
    runtime: {domain: "native", profile: "g0-native", host: null, sandboxed: false},
    corpus: {
      name: "g0-native-microbench", revision: "1",
      task_id: "20-page-load-observe", seed: 0,
      model_provider: null, model: null, model_revision: null
    },
    measurements: {
      page_creation_ms: $page_creation_ms,
      load_observe_ms: $load_observe_ms,
      observation_bytes: $observation_bytes,
      rss_bytes: $rss_bytes,
      installed_bytes: $installed_bytes,
      compressed_bytes: $compressed_bytes
    },
    outcome: {
      status: "passed", execution_domain: "native", effects: [], diagnostics: []
    }
  }' > "$output_dir/result.json"

echo "G0 native measurements written to $output_dir"
