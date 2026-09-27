#!/usr/bin/env bash
set -euo pipefail

# Full-plan generation admission, including direct invocation.
python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
cargo_target="${CARGO_TARGET_DIR:-$repo_root/target}"
case "$cargo_target" in /*) ;; *) cargo_target="$repo_root/$cargo_target" ;; esac
export CARGO_TARGET_DIR="$cargo_target"
build_output="$cargo_target/release"
artifact_root="${ORYN_ARTIFACT_ROOT:-$build_output}"
case "$artifact_root" in /*) ;; *) artifact_root="$repo_root/$artifact_root" ;; esac
export ORYN_ARTIFACT_ROOT="$artifact_root"
worker_protocol="$(python3 "$repo_root/scripts/native_protocol_versions.py" --field worker)"
expected_context_protocol="$(python3 "$repo_root/scripts/native_protocol_versions.py" --field context)"
expected_agent_protocol="$(python3 "$repo_root/scripts/native_protocol_versions.py" --field agent)"
worker="$build_output/oryn-page-worker"
app="$artifact_root/OrynPageWorker.app"
app_worker="$app/Contents/MacOS/oryn-page-worker"
resources="$app/Contents/Resources"
manifest="$resources/worker-manifest.json"
entitlements="$repo_root/sandbox/oryn-page-worker.entitlements"
info_plist="$repo_root/sandbox/oryn-page-worker-Info.plist"
identity="${ORYN_CODESIGN_IDENTITY:-}"
expected_bundle_id="${ORYN_EXPECTED_BUNDLE_ID:-ai.rustic.oryn.page-worker}"
expected_team_id="${ORYN_APPLE_TEAM_ID:-5HVA9VFF8K}"
v8_profile_manifest="${ORYN_V8_PROFILE_MANIFEST:-}"
typed_engine_profile="${ORYN_TYPED_ENGINE_PROFILE:-}"
typed_source_identity="${ORYN_TYPED_SOURCE_IDENTITY:-}"
typed_runtime_output="${ORYN_TYPED_RUNTIME_OUTPUT:-}"
prebuilt_candidate="${ORYN_SECURITY_PREBUILT_CANDIDATE:-}"
if [[ -n "$typed_engine_profile" || -n "$typed_source_identity" || -n "$typed_runtime_output" ]]; then
  if [[ -z "$typed_source_identity" || -n "$prebuilt_candidate" || ( -z "$typed_engine_profile" && -z "$typed_runtime_output" ) || ( -n "$typed_engine_profile" && -n "$typed_runtime_output" ) ]]; then
    echo "Typed signing requires source identity and exactly one typed profile or new typed output; legacy prebuilt substitution is forbidden." >&2
    exit 2
  fi
  python3 "$repo_root/scripts/oryn-validate.py" source-info \
    --source-root "$repo_root" --source-identity "$typed_source_identity" >/dev/null
fi
if [[ -n "$prebuilt_candidate" ]]; then
  python3 "$repo_root/scripts/security_prebuilt.py" check
  # Only validated candidate paths may enter the no-build/no-sign branch.
  parent="$(python3 "$repo_root/scripts/security_prebuilt.py" path runtime)"
  worker="$(python3 "$repo_root/scripts/security_prebuilt.py" path engine_worker)"
  app_worker="$(python3 "$repo_root/scripts/security_prebuilt.py" path worker)"
  app="$(dirname "$(dirname "$(dirname "$app_worker")")")"
  resources="$app/Contents/Resources"
  manifest="$(python3 "$repo_root/scripts/security_prebuilt.py" path worker_manifest)"
  v8_profile_manifest="$(python3 "$repo_root/scripts/security_prebuilt.py" path engine_profile)"
else
  parent="$build_output/oryn"
fi

if [[ -z "$v8_profile_manifest" ]]; then
  echo "Set ORYN_V8_PROFILE_MANIFEST to the current core-full manifest from build-v8-runtime-profile.sh." >&2
  exit 2
fi
python3 "$repo_root/scripts/validate-v8-runtime-profile.py" "$v8_profile_manifest" \
  --profile core-full --target "$(rustc -vV | awk '/host:/ {print $2}')"
export RUSTY_V8_ARCHIVE="$(jq -r '.archive_path' "$v8_profile_manifest")"
export RUSTY_V8_SRC_BINDING_PATH="$(jq -r '.binding_path' "$v8_profile_manifest")"
export ORYN_STARTUP_SNAPSHOT="$(jq -r '.startup_snapshot_path' "$v8_profile_manifest")"
export ORYN_ICU_DATA_FINGERPRINT_PATH="$(jq -r '.icu_data_path' "$v8_profile_manifest")"
export ORYN_V8_BRIDGE_ABI="$(jq -r '.bridge_abi_path' "$v8_profile_manifest")"

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

if [[ -z "$prebuilt_candidate" ]]; then
if [[ -n "$typed_runtime_output" ]]; then
  # A committed-source build produces its own snapshot under the same owned
  # process/cache lease. The original profile remains an immutable prelaunch
  # input; this new intermediate profile is an output of that process.
  snapshot_output="$artifact_root/oryn-startup-core-full.blob"
  snapshot_profile="$artifact_root/snapshot-core-full-manifest.json"
  if [[ -e "$snapshot_output" || -L "$snapshot_output" || -e "$snapshot_profile" || -L "$snapshot_profile" ]]; then
    echo "Typed builds require new snapshot and intermediate profile outputs." >&2
    exit 2
  fi
  mkdir -p "$artifact_root"
  (
    unset ORYN_STARTUP_SNAPSHOT
    /bin/bash "$repo_root/scripts/build-startup-snapshot.sh" core-full \
      "$RUSTY_V8_ARCHIVE" "$snapshot_output"
  )
  test -s "$snapshot_output"
  python3 "$repo_root/scripts/oryn-validate.py" engine-profile-refresh \
    --source-root "$repo_root" --legacy-profile "$v8_profile_manifest" \
    --output "$snapshot_profile" --startup-snapshot "$snapshot_output"
  v8_profile_manifest="$snapshot_profile"
  export ORYN_STARTUP_SNAPSHOT="$snapshot_output"
fi
python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"
cargo build --locked --release -p oryn \
  --bin oryn --bin oryn-webdriver --no-default-features --features "native core-full webdriver-server"
python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"
cargo build --locked --release -p oryn-native --features v8-host,v8-sandbox,core-full --bin oryn-page-worker
mkdir -p "$app/Contents/MacOS" "$resources"
cp "$worker" "$app_worker"
cp "$info_plist" "$app/Contents/Info.plist"
fi
# The exec-only driver launches this CLI in its WebDriver role. Reject a
# feature-incomplete build or frozen prebuilt before signing/admission. Help
# exits during argument parsing without starting a listener or page worker.
"$parent" webdriver --help >/dev/null
if [[ -z "$prebuilt_candidate" ]]; then
  "$build_output/oryn-webdriver" --help >/dev/null
fi
# The bundle is temporarily invalid after replacing a previously signed
# executable. Read build metadata from the linker-signed build output; never
# launch the replaced bundle until codesign has sealed it again below.
python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"
runtime="$("$worker" --runtime-info)"
runtime_file="$build_output/rusty-v8-worker-runtime.json"
if [[ -n "$prebuilt_candidate" ]]; then
  runtime_file="$(mktemp -t oryn-prebuilt-runtime.XXXXXX)"
  trap 'rm -f "$runtime_file"' EXIT
fi
printf '%s\n' "$runtime" > "$runtime_file"
python3 "$repo_root/scripts/validate-v8-rust-runtime.py" "$v8_profile_manifest" "$runtime_file" --binding "$RUSTY_V8_SRC_BINDING_PATH" --archive "$RUSTY_V8_ARCHIVE"
if [[ -n "$prebuilt_candidate" ]]; then rm -f "$runtime_file"; fi
# Verify compiled metadata before creating a manifest or invoking codesign.
jq -e --argjson expected_context_protocol "$expected_context_protocol" --argjson expected_agent_protocol "$expected_agent_protocol" '
  .context_service_protocol_version == $expected_context_protocol and
  .agent_ipc_protocol_version == $expected_agent_protocol
' <<<"$runtime" >/dev/null
if ! jq -e --arg archive "$(jq -r '.archive_sha256' "$v8_profile_manifest")" '
  .v8_archive_sha256 == $archive and
  (.v8_patch_chain_sha256 | type == "string" and test("^[0-9a-f]{64}$"))
' <<<"$runtime" >/dev/null; then
  echo "Built worker does not attest the selected V8 archive and current patch chain" >&2
  exit 2
fi
if [[ -z "$prebuilt_candidate" ]]; then
if [[ -n "$typed_runtime_output" ]]; then
  compiled_profile="$artifact_root/compiled-core-full-manifest.json"
  python3 "$repo_root/scripts/oryn-validate.py" engine-profile-refresh \
    --source-root "$repo_root" --legacy-profile "$v8_profile_manifest" \
    --output "$compiled_profile" --worker "$worker" --runtime-info "$runtime_file"
  v8_profile_manifest="$compiled_profile"
  python3 "$repo_root/scripts/oryn-validate.py" typed-runtime-prepare \
    --source-root "$repo_root" --store-root "${ORYN_DATA_ROOT:?typed runtime requires data root}" \
    --legacy-profile "$v8_profile_manifest" --output-directory "$typed_runtime_output" \
    --source-identity "$typed_source_identity"
  typed_engine_profile="$ORYN_DATA_ROOT/$typed_runtime_output/engine-profile.json"
fi
if [[ -n "$typed_engine_profile" ]]; then
  typed_source_info="$(python3 "$repo_root/scripts/oryn-validate.py" typed-profile-check \
    --source-root "$repo_root" --legacy-profile "$v8_profile_manifest" \
    --typed-profile "$typed_engine_profile" --source-identity "$typed_source_identity")"
  cp "$typed_engine_profile" "$resources/v8-profile-manifest.json"
else
  cp "$v8_profile_manifest" "$resources/v8-profile-manifest.json"
fi

worker_build_sha256="$(shasum -a 256 "$app_worker" | awk '{print $1}')"
worker_build_bytes="$(wc -c < "$app_worker" | tr -d '[:space:]')"
if [[ -n "$typed_engine_profile" ]]; then
  source_tree_sha256="$(jq -er .tree_sha256 <<<"$typed_source_info")"
else
  source_tree_sha256="$("$repo_root/scripts/source-tree-hash.sh")"
fi
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
  --argjson worker_protocol "$worker_protocol" --argjson expected_context_protocol "$expected_context_protocol" --argjson expected_agent_protocol "$expected_agent_protocol" \
  --argjson render_contract "$(jq -er .render_contract_version <<<"$runtime")" \
  --argjson local_effects_contract "$(jq -er .local_effects_contract_version <<<"$runtime")" \
  --arg bundle_id "$expected_bundle_id" \
  --arg team_id "$expected_team_id" \
  --arg signing_cert_sha1 "$identity" \
  --arg v8_archive_sha256 "$(jq -r '.v8_archive_sha256' <<<"$runtime")" \
  --arg rusty_v8_source_manifest_sha256 "$(jq -er '.rusty_v8_source_manifest_sha256' <<<"$runtime")" \
  --arg rusty_v8_source_tree_sha256 "$(jq -er '.rusty_v8_source_tree_sha256' <<<"$runtime")" \
  --arg rusty_v8_weak_teardown_patch_sha256 "$(jq -er '.rusty_v8_weak_teardown_patch_sha256' <<<"$runtime")" \
  --arg rusty_v8_patch_metadata_sha256 "$(jq -er '.rusty_v8_patch_metadata_sha256' <<<"$runtime")" \
  --arg v8_binding_sha256 "$(jq -er '.v8_binding_sha256' <<<"$runtime")" \
  --arg v8_patch_chain_sha256 "$(jq -r '.v8_patch_chain_sha256' <<<"$runtime")" \
  --arg v8_profile_sha256 "$(shasum -a 256 "$resources/v8-profile-manifest.json" | awk '{print $1}')" \
  --arg v8_version "$(jq -r '.v8_version' <<<"$runtime")" \
  --arg worker_build_sha256 "$worker_build_sha256" \
  --argjson worker_build_bytes "$worker_build_bytes" \
  --arg entitlement_sha256 "$entitlement_sha256" \
  --arg policy_sha256 "$policy_sha256" \
  --arg source_tree_sha256 "$source_tree_sha256" \
  --arg style_engine_version "$(jq -r '.style_engine_version' <<<"$runtime")" \
  --arg box_engine_version "taffy-0.14" \
  --arg inline_engine_version "parley-0.11.1" \
  --arg embedded_font_sha256 "b85c38ecea8a7cfb39c24e395a4007474fa5a4fc864f6ee33309eb4948d232d5" \
  '{
    schema_version: 1,
    bundle_id: $bundle_id,
    team_id: $team_id,
    signing_cert_sha1: $signing_cert_sha1,
    protocol_version: $worker_protocol,
    context_service_protocol_version: $expected_context_protocol,
    agent_ipc_protocol_version: $expected_agent_protocol,
    render_contract_version: $render_contract,
    local_effects_contract_version: $local_effects_contract,
    v8_version: $v8_version,
    v8_archive_sha256: $v8_archive_sha256,
    v8_patch_chain_sha256: $v8_patch_chain_sha256,
    rusty_v8_source_manifest_sha256: $rusty_v8_source_manifest_sha256,
    rusty_v8_source_tree_sha256: $rusty_v8_source_tree_sha256,
    rusty_v8_weak_teardown_patch_sha256: $rusty_v8_weak_teardown_patch_sha256,
    rusty_v8_patch_metadata_sha256: $rusty_v8_patch_metadata_sha256,
    v8_binding_sha256: $v8_binding_sha256,

    v8_profile_sha256: $v8_profile_sha256,
    worker_build_sha256: $worker_build_sha256,
    worker_build_bytes: $worker_build_bytes,
    entitlement_sha256: $entitlement_sha256,
    policy_sha256: $policy_sha256,
    source_tree_sha256: $source_tree_sha256,
    style_engine_version: $style_engine_version,
    box_engine_version: $box_engine_version,
    inline_engine_version: $inline_engine_version,
    embedded_font_sha256: $embedded_font_sha256,
    limits: {
      runtime_profile: "full",
      v8_heap_bytes: 268435456,
      worker_rss_bytes: 536870912,
      ipc_frame_bytes: 25165824,
      command_wall_ms: 10000,
      navigation_wall_ms: 30000,
      queue_capacity: 32,
      max_layout_nodes: 100000,
      max_font_bytes: 8388608,
      max_page_font_bytes: 33554432,
      max_viewport_css_px: 16384,
      max_resource_cache_entries: 256,
      max_resource_cache_bytes: 33554432,
      max_resource_cache_entry_bytes: 8388608
    }
  }' > "$manifest"

if [[ -n "$typed_engine_profile" ]]; then
  # New metadata is written before codesign creates the immutable resource seal.
  # Legacy manifests and signed bundles are never relabeled.
  typed_manifest="$(mktemp "$resources/worker-manifest.XXXXXX")"
  jq --arg source_commit "$(jq -er .commit <<<"$typed_source_info")" \
    '. + {source_commit: $source_commit}' "$manifest" > "$typed_manifest"
  mv "$typed_manifest" "$manifest"
fi

codesign --force --options runtime --timestamp=none --sign "$identity" --entitlements "$entitlements" "$app"
fi
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
  ORYN_PAGE_WORKER="$app_worker" "$parent" native --runtime-info
)"
if ! jq -e --argjson worker_protocol "$worker_protocol" --argjson expected_context_protocol "$expected_context_protocol" --argjson expected_agent_protocol "$expected_agent_protocol" '
  .native_v8 == true and
  .build_profile == "release" and
  .execution_mode == "sandboxed_worker" and
  .sandboxed == true and
  .sandbox_state == "app_sandbox" and
  .worker_protocol_version == $worker_protocol and .context_service_protocol_version == $expected_context_protocol and .agent_ipc_protocol_version == $expected_agent_protocol and
  .worker_bundle_id == "ai.rustic.oryn.page-worker" and
  .worker_team_id == "5HVA9VFF8K"
' <<<"$runtime_report" >/dev/null; then
  echo "Parent rejected or misreported the signed worker handshake" >&2
  exit 1
fi

mutation_output="$(
  printf '%s\n' 'observe' 'click "Mutate"' 'observe' 'exit' |
    ORYN_PAGE_WORKER="$app_worker" "$parent" native \
      --html "$repo_root/benchmarks/conformance/javascript-mutation-smoke.html"
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
    ORYN_PAGE_WORKER="$app_worker" "$parent" native \
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
if [[ -n "$prebuilt_candidate" ]]; then
  python3 "$repo_root/scripts/security_prebuilt.py" check
fi
