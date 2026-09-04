#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$repo_root/benchmarks/conformance/runtime-completion-v1.json"
cache_root="${ORYN_CONFORMANCE_CACHE:-$repo_root/artifacts/upstream-conformance}"
revision="$(jq -r '.upstream.revision' "$manifest")"
checkout="$cache_root/wpt-$revision"

mkdir -p "$cache_root"
if [[ ! -d "$checkout/.git" ]]; then
  git clone --filter=blob:none --no-checkout https://github.com/web-platform-tests/wpt.git "$checkout"
fi
git -C "$checkout" fetch --depth=1 origin "$revision"
git -C "$checkout" sparse-checkout init --cone
git -C "$checkout" sparse-checkout set \
  accname wai-aria WebIDL dom nodes interfaces html encoding fetch streams xhr websockets \
  selection-api traversal editing css/css-cascade css/cssom css/selectors referrer-policy \
  IndexedDB intl wasm resources tools common storage docs
git -C "$checkout" checkout --detach "$revision"
test "$(git -C "$checkout" rev-parse HEAD)" = "$revision"
printf '%s\n' "$checkout"
