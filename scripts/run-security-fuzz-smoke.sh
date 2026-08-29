#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output_dir="${1:-$repo_root/artifacts/security-v1/fuzz}"
if [[ "$output_dir" != /* ]]; then output_dir="$repo_root/$output_dir"; fi
seconds="${ORYN_SECURITY_FUZZ_SECONDS:-10}"
toolchain="$(jq -r '.fuzz_toolchain' "$repo_root/security/v8-update-policy.json")"
targets=(html_tree_sink dom_mutation url_policy worker_protocol)
work_root="$(mktemp -d -t oryn-security-fuzz.XXXXXX)"
cleanup() {
  rm -rf "$work_root"
}
trap cleanup EXIT

mkdir -p "$output_dir/crashes"
rustup run "$toolchain" rustc --version >"$output_dir/toolchain.txt"
cargo fuzz --version >"$output_dir/cargo-fuzz.txt"

for target in "${targets[@]}"; do
  seed_dir="$repo_root/fuzz/seeds/$target"
  test -d "$seed_dir"
  test -n "$(find "$seed_dir" -type f -print -quit)"
  corpus_dir="$work_root/$target"
  mkdir -p "$corpus_dir"
  cp -R "$seed_dir/." "$corpus_dir/"
  (
    cd "$repo_root/fuzz"
    cargo "+$toolchain" fuzz run "$target" "$corpus_dir" -- \
      -max_total_time="$seconds" \
      -timeout=2 \
      -artifact_prefix="$output_dir/crashes/$target-"
  ) >"$output_dir/$target.log" 2>&1
done

if find "$output_dir/crashes" -type f -print -quit | grep -q .; then
  echo "security fuzzing produced one or more crashing inputs" >&2
  exit 1
fi

jq -n \
  --arg toolchain "$toolchain" \
  --argjson seconds "$seconds" \
  --argjson targets "$(printf '%s\n' "${targets[@]}" | jq -R . | jq -s .)" \
  '{schema_version:1,suite:"oryn2-security-fuzz-smoke",toolchain:$toolchain,
    seconds_per_target:$seconds,targets:[$targets[] | {name:.,status:"passed"}],
    crashes:0,outcome:{status:"passed"}}' >"$output_dir/summary.json"
jq . "$output_dir/summary.json"
