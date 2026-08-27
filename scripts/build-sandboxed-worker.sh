#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
worker="$repo_root/target/release/oryn-page-worker"
app="$repo_root/target/release/OrynPageWorker.app"
app_worker="$app/Contents/MacOS/oryn-page-worker"
entitlements="$repo_root/sandbox/oryn-page-worker.entitlements"
info_plist="$repo_root/sandbox/oryn-page-worker-Info.plist"
identity="${ORYN_CODESIGN_IDENTITY:-}"
expected_bundle_id="${ORYN_EXPECTED_BUNDLE_ID:-ai.rustic.oryn.page-worker}"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "App Sandbox worker signing is only available on macOS" >&2
  exit 2
fi

if [[ -z "$identity" ]]; then
  echo "Set ORYN_CODESIGN_IDENTITY to a valid Apple Development signing identity." >&2
  echo "Ad-hoc signatures carry entitlements but macOS aborts App Sandbox workers at launch." >&2
  exit 2
fi

if ! security find-identity -v -p codesigning | grep -Fq "$identity"; then
  echo "The requested signing identity is not trusted or has no accessible private key: $identity" >&2
  exit 2
fi

plutil -lint "$info_plist" >/dev/null

cargo build --locked --release -p oryn-native --bin oryn-page-worker
mkdir -p "$app/Contents/MacOS"
cp "$worker" "$app_worker"
cp "$info_plist" "$app/Contents/Info.plist"
codesign --force --sign "$identity" --entitlements "$entitlements" "$app"
codesign --verify --strict --verbose=2 "$app"

signature_details="$(codesign -dvvv "$app_worker" 2>&1)"
if [[ "$signature_details" != *"Identifier=$expected_bundle_id"* ]]; then
  echo "Signed worker identifier does not match $expected_bundle_id" >&2
  exit 1
fi
if [[ "$signature_details" != *"Authority=Apple Development:"* ||
      "$signature_details" != *"Authority=Apple Worldwide Developer Relations Certification Authority"* ||
      "$signature_details" != *"TeamIdentifier="* ]]; then
  echo "Signed worker does not have the expected trusted Apple Development chain" >&2
  exit 1
fi

entitlements_dump="$(mktemp -t oryn-worker-entitlements.XXXXXX)"
trap 'rm -f "$entitlements_dump"' EXIT
codesign -d --entitlements :- "$app_worker" >"$entitlements_dump" 2>/dev/null
if [[ "$(/usr/libexec/PlistBuddy -c 'Print :com.apple.security.app-sandbox' "$entitlements_dump")" != "true" ]]; then
  echo "Signed worker is missing com.apple.security.app-sandbox=true" >&2
  exit 1
fi

response="$(printf '%s\n' '{"command":"ping"}' '{"command":"exit"}' | "$app_worker")"
if [[ "$response" != *'"status":"pong"'* || "$response" != *'"status":"exiting"'* ]]; then
  echo "Sandboxed worker did not complete its protocol smoke test" >&2
  exit 1
fi
printf '%s\n' "$response"
printf '%s\n' "$signature_details"
