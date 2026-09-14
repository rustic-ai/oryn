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
  accname wai-aria html-aam webidl dom domparsing domxpath nodes interfaces html url urlpattern \
  encoding encoding-detection fetch xhr cors webstorage custom-elements shadow-dom mimesniff \
  streams FileAPI workers wasm WebCryptoAPI console referrer-policy content-security-policy \
  subresource-integrity secure-contexts cookies storage IndexedDB websockets eventsource \
  import-maps navigation-api page-visibility requestidlecallback scheduler permissions-policy \
  svg uievents pointerevents input-events intersection-observer resize-observer clipboard-apis \
  selection selection-api traversal editing css/css-cascade css/cssom css/selectors \
  intl js ecmascript resources tools common docs
git -C "$checkout" checkout --detach "$revision"
test "$(git -C "$checkout" rev-parse HEAD)" = "$revision"
printf '%s\n' "$checkout"
