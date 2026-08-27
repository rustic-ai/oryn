#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output_dir="${1:-$repo_root/artifacts/g2r/production-macos-arm64}"
samples="${SAMPLES:-30}"
fixture="$repo_root/benchmarks/conformance/g2r-v8-mutation.html"
mkdir -p "$output_dir"

cd "$repo_root"
cargo build --release --locked -p oryn
binary="$repo_root/target/release/oryn"
protocol_input="$output_dir/protocol-input.txt"
printf '%s\n' 'observe' 'click "Mutate"' 'observe' 'exit' > "$protocol_input"

for _ in $(seq 1 5); do
  "$binary" native --html "$fixture" < "$protocol_input" >/dev/null 2>/dev/null
done

: > "$output_dir/startup-ms.txt"
for _ in $(seq 1 "$samples"); do
  /usr/bin/time -lp "$binary" native --html "$fixture" \
    < "$protocol_input" > "$output_dir/protocol.jsonl" \
    2> "$output_dir/time.txt"
  awk '/^real / {print $2 * 1000}' "$output_dir/time.txt" >> "$output_dir/startup-ms.txt"
done

/usr/bin/time -l "$binary" native --html "$fixture" \
  < "$protocol_input" > "$output_dir/protocol.jsonl" \
  2> "$output_dir/rss.txt"

runtime="$($binary native --runtime-info)"
commit="$(git rev-parse HEAD)"
sha256="$(shasum -a 256 "$binary" | awk '{print $1}')"
installed_bytes="$(wc -c < "$binary" | tr -d '[:space:]')"
compressed_bytes="$(gzip -c "$binary" | wc -c | tr -d '[:space:]')"
rss_bytes="$(awk '/maximum resident set size/ {print $1}' "$output_dir/rss.txt")"
startup_p50_ms="$(sort -n "$output_dir/startup-ms.txt" | awk '{v[NR]=$1} END {print v[int((NR+1)/2)]}')"
startup_p95_ms="$(sort -n "$output_dir/startup-ms.txt" | awk '{v[NR]=$1} END {i=int((NR-1)*.95)+1; print v[i]}')"
mutation_effects="$(jq -Rsc '
  split("\n")
  | map(sub("^[^{]*"; "") | fromjson? | select(. != null))
  | [.[] | select(.kind == "action") | .result.effects[].kind]
  | unique
' "$output_dir/protocol.jsonl")"
mutation_observed="$(jq -Rsc '
  split("\n")
  | map(sub("^[^{]*"; "") | fromjson? | select(. != null))
  | any(.[] | select(.kind == "observation") | .observation.nodes[]?; .name == "changed")
' "$output_dir/protocol.jsonl")"

jq -n \
  --arg run_id "g2r-production-$(date -u +%Y%m%dT%H%M%SZ)" \
  --arg captured_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg source_commit "$commit" \
  --arg binary_sha256 "$sha256" \
  --arg os "$(sw_vers -productVersion)" \
  --arg arch "$(uname -m)" \
  --arg rustc "$(rustc --version)" \
  --argjson samples "$samples" \
  --argjson runtime "$runtime" \
  --argjson installed_bytes "$installed_bytes" \
  --argjson compressed_bytes "$compressed_bytes" \
  --argjson startup_p50_ms "$startup_p50_ms" \
  --argjson startup_p95_ms "$startup_p95_ms" \
  --argjson peak_rss_bytes "$rss_bytes" \
  --argjson mutation_effects "$mutation_effects" \
  --argjson mutation_observed "$mutation_observed" \
  '{
    schema_version: 3,
    run_id: $run_id,
    captured_at: $captured_at,
    source_commit: $source_commit,
    environment: {os: $os, arch: $arch, rustc: $rustc},
    binary: {
      path: "target/release/oryn",
      sha256: $binary_sha256,
      installed_bytes: $installed_bytes,
      compressed_bytes: $compressed_bytes,
      runtime: $runtime
    },
    measurements: {
      samples: $samples,
      startup_to_mutation_protocol_p50_ms: $startup_p50_ms,
      startup_to_mutation_protocol_p95_ms: $startup_p95_ms,
      peak_rss_bytes: $peak_rss_bytes
    },
    mutation_smoke: {
      observed_changed_state: $mutation_observed,
      effect_kinds: $mutation_effects
    },
    outcome: {
      status: (if $runtime.native_v8 and $mutation_observed and ($mutation_effects | index("dom_mutation")) then "passed" else "failed" end),
      diagnostics: []
    }
  }' > "$output_dir/result.json"

jq . "$output_dir/result.json"
