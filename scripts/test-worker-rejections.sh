#!/usr/bin/env bash
set -euo pipefail

# Full-plan generation admission, including direct invocation.
python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
parent="$repo_root/target/release/oryn"
signed_app="$repo_root/target/release/OrynPageWorker.app"
if [[ -n "${ORYN_SECURITY_PREBUILT_CANDIDATE:-}" ]]; then
  parent="$(python3 "$repo_root/scripts/security_prebuilt.py" path runtime)"
  selected_worker="$(python3 "$repo_root/scripts/security_prebuilt.py" path worker)"
  signed_app="$(dirname "$(dirname "$(dirname "$selected_worker")")")"
fi
identity="${ORYN_CODESIGN_IDENTITY:-}"
entitlements="$repo_root/sandbox/oryn-page-worker.entitlements"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Worker signature rejection tests require macOS" >&2
  exit 2
fi
if [[ -z "$identity" ]]; then
  echo "Set ORYN_CODESIGN_IDENTITY for the trusted re-signing cases" >&2
  exit 2
fi
test -x "$parent"
test -d "$signed_app"

variant_root="$(mktemp -d -t oryn-worker-rejections.XXXXXX)"
cleanup() {
  rm -rf "$variant_root"
}
trap cleanup EXIT

expect_rejected() {
  local label="$1"
  local worker="$2"
  if ORYN_PAGE_WORKER="$worker" "$parent" native --runtime-info \
    >"$variant_root/$label.stdout" 2>"$variant_root/$label.stderr"; then
    echo "Production accepted the $label worker" >&2
    exit 1
  fi
  printf 'rejected: %s\n' "$label"
}

copy_variant() {
  local name="$1"
  local destination="$variant_root/$name/OrynPageWorker.app"
  mkdir -p "$variant_root/$name"
  ditto "$signed_app" "$destination"
  printf '%s\n' "$destination"
}

altered_app="$(copy_variant altered-manifest)"
jq '.source_tree_sha256 = ("0" * 64)' \
  "$altered_app/Contents/Resources/worker-manifest.json" \
  >"$variant_root/altered-manifest.json"
mv "$variant_root/altered-manifest.json" \
  "$altered_app/Contents/Resources/worker-manifest.json"
expect_rejected altered-manifest \
  "$altered_app/Contents/MacOS/oryn-page-worker"

altered_executable_app="$(copy_variant altered-executable)"
printf '\0' >>"$altered_executable_app/Contents/MacOS/oryn-page-worker"
expect_rejected altered-executable \
  "$altered_executable_app/Contents/MacOS/oryn-page-worker"

adhoc_app="$(copy_variant adhoc)"
codesign --force --options runtime --timestamp=none --sign - \
  --entitlements "$entitlements" "$adhoc_app" >/dev/null
expect_rejected adhoc "$adhoc_app/Contents/MacOS/oryn-page-worker"

overentitled_app="$(copy_variant overentitled)"
overentitlements="$variant_root/overentitled.plist"
cp "$entitlements" "$overentitlements"
/usr/libexec/PlistBuddy -c \
  'Add :com.apple.security.network.client bool true' "$overentitlements"
codesign --force --options runtime --timestamp=none --sign "$identity" \
  --entitlements "$overentitlements" "$overentitled_app" >/dev/null
expect_rejected overentitled \
  "$overentitled_app/Contents/MacOS/oryn-page-worker"

wrong_id_app="$(copy_variant wrong-identifier)"
codesign --force --options runtime --timestamp=none --sign "$identity" \
  --identifier ai.rustic.oryn.wrong-worker \
  --entitlements "$entitlements" "$wrong_id_app" >/dev/null
expect_rejected wrong-identifier \
  "$wrong_id_app/Contents/MacOS/oryn-page-worker"

python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"
if [[ -n "${ORYN_SECURITY_PREBUILT_CANDIDATE:-}" ]]; then
  debug_worker="$(python3 "$repo_root/scripts/security_prebuilt.py" path debug_worker)"
else
  cargo build --locked -p oryn-native --features v8-host,v8-sandbox --bin oryn-page-worker
  debug_worker="$repo_root/target/debug/oryn-page-worker"
fi
debug_app="$(copy_variant debug)"
cp "$debug_worker" \
  "$debug_app/Contents/MacOS/oryn-page-worker"
codesign --force --options runtime --timestamp=none --sign "$identity" \
  --entitlements "$entitlements" "$debug_app" >/dev/null
expect_rejected debug "$debug_app/Contents/MacOS/oryn-page-worker"

printf '%s\n' "All worker substitution variants were rejected."
