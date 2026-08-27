#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
worker="$repo_root/target/release/oryn-page-worker"
app="$repo_root/target/release/OrynPageWorker.app"
app_worker="$app/Contents/MacOS/oryn-page-worker"
resources="$app/Contents/Resources"
manifest="$resources/worker-manifest.json"
entitlements="$repo_root/sandbox/oryn-page-worker.entitlements"
info_plist="$repo_root/sandbox/oryn-page-worker-Info.plist"
identity="${ORYN_CODESIGN_IDENTITY:-}"
expected_bundle_id="${ORYN_EXPECTED_BUNDLE_ID:-ai.rustic.oryn.page-worker}"
expected_team_id="${ORYN_APPLE_TEAM_ID:-5HVA9VFF8K}"

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

cargo build --locked --release -p oryn
cargo build --locked --release -p oryn-native --features v8-host --bin oryn-page-worker
mkdir -p "$app/Contents/MacOS" "$resources"
cp "$worker" "$app_worker"
cp "$info_plist" "$app/Contents/Info.plist"
runtime="$($app_worker --runtime-info)"
worker_build_sha256="$(shasum -a 256 "$app_worker" | awk '{print $1}')"
worker_build_bytes="$(wc -c < "$app_worker" | tr -d '[:space:]')"
source_tree_sha256="$($repo_root/scripts/source-tree-hash.sh)"
entitlement_sha256="$(
  printf '%s' '{"com.apple.security.app-sandbox":true,"com.apple.security.cs.allow-jit":true}' |
    shasum -a 256 | awk '{print $1}'
)"
policy_sha256="$(
  {
    printf 'security.rs\0'
    sed -n '1,$p' "$repo_root/crates/oryn-native/src/security.rs"
    printf '\0network.rs\0'
    sed -n '1,$p' "$repo_root/crates/oryn-native/src/network.rs"
  } | shasum -a 256 | awk '{print $1}'
)"

jq -n \
  --arg bundle_id "$expected_bundle_id" \
  --arg team_id "$expected_team_id" \
  --arg signing_cert_sha1 "$identity" \
  --arg v8_version "$(jq -r '.v8_version' <<<"$runtime")" \
  --arg worker_build_sha256 "$worker_build_sha256" \
  --argjson worker_build_bytes "$worker_build_bytes" \
  --arg entitlement_sha256 "$entitlement_sha256" \
  --arg policy_sha256 "$policy_sha256" \
  --arg source_tree_sha256 "$source_tree_sha256" \
  '{
    schema_version: 1,
    bundle_id: $bundle_id,
    team_id: $team_id,
    signing_cert_sha1: $signing_cert_sha1,
    protocol_version: 2,
    v8_version: $v8_version,
    worker_build_sha256: $worker_build_sha256,
    worker_build_bytes: $worker_build_bytes,
    entitlement_sha256: $entitlement_sha256,
    policy_sha256: $policy_sha256,
    source_tree_sha256: $source_tree_sha256,
    limits: {
      v8_heap_bytes: 268435456,
      worker_rss_bytes: 536870912,
      ipc_frame_bytes: 25165824,
      command_wall_ms: 10000,
      navigation_wall_ms: 30000,
      queue_capacity: 32
    }
  }' > "$manifest"

codesign --force --options runtime --timestamp=none --sign "$identity" --entitlements "$entitlements" "$app"
codesign --verify --strict --verbose=2 "$app"

cert_dir="$(mktemp -d -t oryn-worker-cert.XXXXXX)"
cert_prefix="$cert_dir/cert"
codesign -d "--extract-certificates=$cert_prefix" "$app_worker" 2>/dev/null
leaf_cert_sha1="$(shasum -a 1 "${cert_prefix}0" | awk '{print toupper($1)}')"
rm -rf "$cert_dir"
if [[ "$leaf_cert_sha1" != "$identity" ]]; then
  echo "Signed worker leaf certificate does not match $identity" >&2
  exit 1
fi

signature_details="$(codesign -dvvv "$app_worker" 2>&1)"
if [[ "$signature_details" != *"Identifier=$expected_bundle_id"* ]]; then
  echo "Signed worker identifier does not match $expected_bundle_id" >&2
  exit 1
fi
if [[ "$signature_details" != *"Authority=Apple Development:"* ||
      "$signature_details" != *"Authority=Apple Worldwide Developer Relations Certification Authority"* ||
      "$signature_details" != *"TeamIdentifier=$expected_team_id"* ||
      "$signature_details" != *"flags=0x10000(runtime)"* ]]; then
  echo "Signed worker does not have the expected trusted Apple Development chain" >&2
  exit 1
fi

entitlements_dump="$(mktemp -t oryn-worker-entitlements.XXXXXX)"
trap 'rm -f "$entitlements_dump"' EXIT
codesign -d --entitlements :- "$app_worker" >"$entitlements_dump" 2>/dev/null
if ! plutil -convert json -o - "$entitlements_dump" |
  jq -e '
    keys == [
      "com.apple.security.app-sandbox",
      "com.apple.security.cs.allow-jit"
    ] and
    .["com.apple.security.app-sandbox"] == true and
    .["com.apple.security.cs.allow-jit"] == true
  ' >/dev/null; then
  echo "Signed worker entitlements are not exactly App Sandbox plus allow-jit" >&2
  exit 1
fi
if [[ "$(/usr/libexec/PlistBuddy -c 'Print :com.apple.security.app-sandbox' "$entitlements_dump")" != "true" ]]; then
  echo "Signed worker is missing com.apple.security.app-sandbox=true" >&2
  exit 1
fi
if [[ "$(/usr/libexec/PlistBuddy -c 'Print :com.apple.security.cs.allow-jit' "$entitlements_dump")" != "true" ]]; then
  echo "Signed worker is missing com.apple.security.cs.allow-jit=true" >&2
  exit 1
fi
for prohibited in \
  com.apple.security.network.client \
  com.apple.security.network.server \
  com.apple.security.cs.allow-unsigned-executable-memory \
  com.apple.security.cs.allow-dyld-environment-variables \
  com.apple.security.cs.disable-library-validation \
  com.apple.security.cs.disable-executable-page-protection; do
  if /usr/libexec/PlistBuddy -c "Print :$prohibited" "$entitlements_dump" >/dev/null 2>&1; then
    echo "Signed worker has prohibited entitlement $prohibited" >&2
    exit 1
  fi
done

response="$(printf '%s\n' '{"command":"ping"}' '{"command":"exit"}' | "$app_worker")"
if [[ "$response" != *'"status":"pong"'* || "$response" != *'"status":"exiting"'* ]]; then
  echo "Sandboxed worker did not complete its protocol smoke test" >&2
  exit 1
fi

probe_file="$(mktemp -t oryn-worker-denied.XXXXXX)"
printf '%s\n' "sandbox-denied" >"$probe_file"
trap 'rm -f "$entitlements_dump" "$probe_file"' EXIT
security_probe="$(env -i "$app_worker" --security-probe "$probe_file")"
if ! jq -e '
  .filesystem_read_succeeded == false and
  .outgoing_connection_succeeded == false and
  .listener_succeeded == false and
  .secret_environment_present == false
' <<<"$security_probe" >/dev/null; then
  echo "Signed worker escaped one or more sandbox boundaries: $security_probe" >&2
  exit 1
fi

runtime_report="$(
  ORYN_PAGE_WORKER="$app_worker" "$repo_root/target/release/oryn" native --runtime-info
)"
if ! jq -e '
  .native_v8 == true and
  .build_profile == "release" and
  .execution_mode == "sandboxed_worker" and
  .sandboxed == true and
  .sandbox_state == "app_sandbox" and
  .worker_protocol_version == 2 and
  .worker_bundle_id == "ai.rustic.oryn.page-worker" and
  .worker_team_id == "5HVA9VFF8K"
' <<<"$runtime_report" >/dev/null; then
  echo "Parent rejected or misreported the signed worker handshake" >&2
  exit 1
fi

mutation_output="$(
  printf '%s\n' 'observe' 'click "Mutate"' 'observe' 'exit' |
    ORYN_PAGE_WORKER="$app_worker" "$repo_root/target/release/oryn" native \
      --html "$repo_root/benchmarks/conformance/g2r-v8-mutation.html"
)"
if ! jq -Rse '
  split("\n")
  | map(fromjson? | select(. != null))
  | any(.[]; .kind == "action" and any(.result.effects[]?; .kind == "dom_mutation"))
  and any(.[]; .kind == "observation" and any(.observation.nodes[]?; .name == "changed"))
' <<<"$mutation_output" >/dev/null; then
  echo "Signed worker failed the V8 DOM-mutation smoke test" >&2
  exit 1
fi

server_root="$(mktemp -d -t oryn-broker-server.XXXXXX)"
server_port_file="$(mktemp -t oryn-broker-port.XXXXXX)"
printf '%s\n' '<!doctype html><title>Brokered</title><button>Network</button>' \
  >"$server_root/index.html"
python3 -c '
import functools
import http.server
import pathlib
import sys

root = sys.argv[1]
port_file = pathlib.Path(sys.argv[2])
handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=root)
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
port_file.write_text(str(server.server_port), encoding="utf-8")
server.serve_forever()
' "$server_root" "$server_port_file" >/dev/null 2>&1 &
server_pid="$!"
trap 'kill "$server_pid" 2>/dev/null || true; rm -rf "$server_root"; rm -f "$server_port_file" "$entitlements_dump" "$probe_file"' EXIT
for _ in $(seq 1 50); do
  [[ -s "$server_port_file" ]] && break
  sleep 0.05
done
server_port="$(cat "$server_port_file")"
broker_output="$(
  printf '%s\n' exit |
    ORYN_PAGE_WORKER="$app_worker" "$repo_root/target/release/oryn" native \
      --allow-loopback --url "http://127.0.0.1:$server_port/" \
      --evaluate 'document.title'
)"
if [[ "$broker_output" != "Brokered" ]]; then
  echo "Authorized parent-broker request did not reach the sandboxed worker" >&2
  exit 1
fi
kill "$server_pid" 2>/dev/null || true
wait "$server_pid" 2>/dev/null || true
rm -rf "$server_root"
rm -f "$server_port_file"
trap 'rm -f "$entitlements_dump" "$probe_file"' EXIT
printf '%s\n' "$response"
printf '%s\n' "$security_probe"
printf '%s\n' "$runtime_report"
printf '%s\n' "$signature_details"
