#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
product_app="$repo_root/target/release/Oryn.app"
product_parent="$product_app/Contents/MacOS/oryn"
product_worker="$product_app/Contents/Helpers/OrynPageWorker.app"
standalone_parent="$repo_root/target/release/oryn"
standalone_worker="$repo_root/target/release/OrynPageWorker.app"
info_plist="$repo_root/sandbox/oryn-Info.plist"
identity="${ORYN_CODESIGN_IDENTITY:-}"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Oryn.app packaging is only available on macOS" >&2
  exit 2
fi
if [[ -z "$identity" ]]; then
  echo "Set ORYN_CODESIGN_IDENTITY to the trusted Apple signing identity." >&2
  exit 2
fi

"$repo_root/scripts/build-sandboxed-worker.sh"

rm -rf "$product_app"
mkdir -p "$product_app/Contents/MacOS" "$product_app/Contents/Helpers"
cp "$standalone_parent" "$product_parent"
cp "$info_plist" "$product_app/Contents/Info.plist"
ditto "$standalone_worker" "$product_worker"

# Nested code is already signed with the worker's minimal App Sandbox/JIT
# entitlements. Sign the outer product last so its resource seal covers the
# exact parent and nested worker without granting worker entitlements upstream.
codesign --force --options runtime --timestamp=none --sign "$identity" "$product_app"
codesign --verify --deep --strict --verbose=2 "$product_app"

runtime="$($product_parent native --runtime-info)"
jq -e '
  .native_v8 == true and
  .execution_mode == "sandboxed_worker" and
  .sandbox_state == "app_sandbox" and
  .worker_bundle_id == "ai.rustic.oryn.page-worker"
' <<<"$runtime" >/dev/null

jq -n \
  --arg product "$product_app" \
  --arg parent_sha256 "$(shasum -a 256 "$product_parent" | awk '{print $1}')" \
  --arg worker_sha256 "$(shasum -a 256 "$product_worker/Contents/MacOS/oryn-page-worker" | awk '{print $1}')" \
  --argjson parent_bytes "$(wc -c < "$product_parent" | tr -d '[:space:]')" \
  --argjson worker_bytes "$(wc -c < "$product_worker/Contents/MacOS/oryn-page-worker" | tr -d '[:space:]')" \
  '{
    product: $product,
    packaging: "single_macos_app_with_nested_sandbox_worker",
    parent: {sha256: $parent_sha256, bytes: $parent_bytes},
    worker: {sha256: $worker_sha256, bytes: $worker_bytes}
  }'
