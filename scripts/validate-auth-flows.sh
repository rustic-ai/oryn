#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
oryn_bin="${ORYN_BIN:-$repo_root/target/debug/oryn}"
node_bin="${ORYN_NODE_BIN:-$(command -v node || true)}"
if [[ -z "$node_bin" && -x "/Users/rohit/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node" ]]; then
  node_bin="/Users/rohit/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node"
fi
port="${ORYN_APP_COMPATIBILITY_PORT:-3104}"
domain="${ORYN_APP_COMPATIBILITY_DOMAIN:-both}"
artifact_dir="${ORYN_APP_COMPATIBILITY_ARTIFACT_DIR:-$repo_root/artifacts/app-compatibility/oauth}"
server_pid=""

cleanup() {
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if [[ "${ORYN_APP_COMPATIBILITY_EXTERNAL_SERVER:-0}" != "1" ]]; then
  if [[ -z "$node_bin" ]]; then
    echo "Node.js is required to start the application compatibility fixture server" >&2
    exit 2
  fi
  (
    cd "$repo_root/test-harness"
    PORT="$port" "$node_bin" server.js
  ) >"${TMPDIR:-/tmp}/oryn-app-compatibility-harness-$port.log" 2>&1 &
  server_pid="$!"
  for _ in $(seq 1 50); do
    curl --fail --silent "http://127.0.0.1:$port/ping" >/dev/null 2>&1 && break
    sleep 0.1
  done
fi

base_url="http://127.0.0.1:$port"
curl --fail --silent "$base_url/ping" >/dev/null
mkdir -p "$artifact_dir"

assert_native_result() {
  local log="$1"
  local expected="$2"
  jq -s -e --arg expected "$expected" '
    any(.[]; .kind == "structured" and .value == $expected) and
    (all(.[]; (.kind != "trace") or all(.events[]; .kind.kind != "script_diagnostic" and (.kind.kind != "console" or .kind.level != "error"))))
  ' "$log" >/dev/null
}

if [[ "$domain" == "native" || "$domain" == "both" ]]; then
  declare -a cases=(
    "popup-pkce|rp/popup|Authenticated with PKCE"
    "redirect|rp/redirect|Redirect authenticated"
    "mfa-handoff|rp/mfa|MFA authenticated"
    "popup-close-recovery|rp/recovery|Popup closed; opener recovered"
  )
  for entry in "${cases[@]}"; do
    IFS='|' read -r name route expected <<<"$entry"
    log="$artifact_dir/$name-native.jsonl"
    "$oryn_bin" --file "$repo_root/benchmarks/app-compatibility/oauth/$name.oil" \
      native --url "$base_url/app-compatibility/auth/$route" --allow-loopback >"$log"
    assert_native_result "$log" "$expected"
    echo "application compatibility OAuth passed: $name (native)"
  done

  # Establish the IdP cookie in the same ephemeral BrowserContext before the
  # relying party opens its SSO popup.
  sso_log="$artifact_dir/existing-session-sso-native.jsonl"
  {
    printf 'goto %s\n' "$base_url/app-compatibility/auth/rp/sso"
    cat "$repo_root/benchmarks/app-compatibility/oauth/existing-session-sso.oil"
  } | "$oryn_bin" native --url "$base_url/app-compatibility/auth/idp/session" --allow-loopback >"$sso_log"
  assert_native_result "$sso_log" "SSO authenticated"
  echo "application compatibility OAuth passed: existing-session-sso (native)"
fi

if [[ "$domain" == "chromium" || "$domain" == "both" ]]; then
  if [[ -z "$node_bin" ]]; then
    echo "Node.js is required to run the Chromium differential harness" >&2
    exit 2
  fi
  chromium_log="$artifact_dir/chromium.jsonl"
  "$node_bin" "$repo_root/test-harness/app-compatibility-chromium.js" oauth "$base_url" >"$chromium_log"
  [[ "$(jq -s '[.[] | select(.domain == "chromium" and .status == "passed")] | length' "$chromium_log")" == "5" ]]
  echo "application compatibility OAuth corpus passed: Chromium differential"
fi

echo "application compatibility OAuth corpus passed in domain mode: $domain"
