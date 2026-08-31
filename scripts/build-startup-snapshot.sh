#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
profile="${1:?usage: build-startup-snapshot.sh core-full|core-lite <v8-archive> <output.blob>}"
archive="${2:?missing sandboxed V8 archive}"
output="${3:?missing output snapshot}"
test -f "$archive"
RUSTY_V8_ARCHIVE="$archive" cargo run \
  --manifest-path "$repo_root/Cargo.toml" \
  --release -p oryn-native --features "v8-host v8-sandbox $profile" \
  --bin oryn-startup-snapshot -- "$output"
shasum -a 256 "$output"
