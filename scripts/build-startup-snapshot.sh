#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
profile="${1:?usage: build-startup-snapshot.sh core-full|core-lite <v8-archive> <output.blob>}"
archive="${2:?missing sandboxed V8 archive}"
output="${3:?missing output snapshot}"
test -f "$archive"
icu_args=()
if [[ "$profile" == core-lite ]]; then
  icu_data="${ORYN_LITE_ICU_DATA:?core-lite snapshot requires ORYN_LITE_ICU_DATA}"
  test -f "$icu_data"
  icu_args=(ORYN_ICU_DATA_PATH="$icu_data")
fi
env "${icu_args[@]}" RUSTY_V8_ARCHIVE="$archive" cargo run \
  --manifest-path "$repo_root/Cargo.toml" \
  --release -p oryn-native --features "v8-host v8-sandbox $profile" \
  --bin oryn-startup-snapshot -- "$output"
shasum -a 256 "$output"
