#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
profile="${1:?usage: build-v8-runtime-profile.sh core-full|core-lite <output-directory>}"
output="${2:?usage: build-v8-runtime-profile.sh core-full|core-lite <output-directory>}"
case "$profile" in core-full|core-lite) ;; *) echo "invalid profile: $profile" >&2; exit 2;; esac

mkdir -p "$output"
target_dir="$repo_root/target/v8-$profile"
export CARGO_TARGET_DIR="$target_dir"
export V8_FROM_SOURCE=1
export EXTRA_GN_ARGS="v8_enable_sandbox=true v8_enable_pointer_compression=true v8_enable_external_code_space=true"

icu_source=""
if [[ "$profile" == core-lite ]]; then
  icu_source="${ORYN_LITE_ICU_DATA:?core-lite requires ORYN_LITE_ICU_DATA produced for root,en,en-US}"
  test -f "$icu_source"
  export EXTRA_GN_ARGS="$EXTRA_GN_ARGS icu_use_data_file=true"
fi

cargo build --manifest-path "$repo_root/Cargo.toml" -p oryn-native --release --features "v8-host v8-sandbox $profile" --bin oryn-page-worker
archive="$(find "$target_dir/release/gn_out/obj" -name librusty_v8.a -print -quit)"
test -n "$archive"
archive_sha="$(shasum -a 256 "$archive" | awk '{print $1}')"
archive_out="$output/librusty_v8-$profile-$archive_sha.a"
cp "$archive" "$archive_out"

icu_sha=null
icu_out=null
if [[ -n "$icu_source" ]]; then
  icu_sha="$(shasum -a 256 "$icu_source" | awk '{print $1}')"
  icu_out="$output/icudtl-$profile-$icu_sha.dat"
  cp "$icu_source" "$icu_out"
fi

jq -n \
  --arg profile "$profile" \
  --arg target "$(rustc -vV | awk '/host:/ {print $2}')" \
  --arg v8 "150.4.0" \
  --arg archive "$archive_out" \
  --arg archive_sha256 "$archive_sha" \
  --argjson icu_sha256 "$(if [[ "$icu_sha" == null ]]; then echo null; else jq -Rn --arg value "$icu_sha" '$value'; fi)" \
  --argjson icu_path "$(if [[ "$icu_out" == null ]]; then echo null; else jq -Rn --arg value "$icu_out" '$value'; fi)" \
  --arg gn_args "$EXTRA_GN_ARGS" \
  '{schema_version:1,profile:$profile,target:$target,v8_version:$v8,v8_heap_sandbox:true,pointer_compression:true,archive_path:$archive,archive_sha256:$archive_sha256,icu_data_path:$icu_path,icu_data_sha256:$icu_sha256,gn_args:$gn_args}' \
  > "$output/$profile-manifest.json"
printf '%s\n' "$output/$profile-manifest.json"
