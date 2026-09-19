#!/usr/bin/env bash
set -euo pipefail

# Full-plan generation admission, including direct invocation.
python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"

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
failures=0
if [[ "$domain" != "native" && "$domain" != "chromium" && "$domain" != "both" ]]; then
  echo "Invalid domain: $domain" >&2
  exit 2
fi

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
  local task="$3"
  local action_count
  action_count="$(awk '$1 ~ /^(click|type|select)$/ { count++ } END { print count+0 }' "$task")"
  jq -s -e --arg expected "$expected" --argjson actions "$action_count" \
    -f "$repo_root/scripts/validate_auth_flow.jq" "$log" >/dev/null
}

if [[ "$domain" == "native" || "$domain" == "both" ]]; then
  declare -a cases=(
    "popup-pkce|rp/popup|Authenticated with PKCE"
    "redirect|rp/redirect|Redirect authenticated"
    "mfa-handoff|rp/mfa|MFA authenticated"
    "popup-close-recovery|rp/recovery|Popup closed; opener recovered"
    "existing-session-sso|idp/session|SSO authenticated"
  )
  for entry in "${cases[@]}"; do
    IFS='|' read -r name route expected <<<"$entry"
    log="$artifact_dir/$name-native.jsonl"
    task="$artifact_dir/$name-native.oil"
    # Bind the locked fixture origin to this run. In particular, SSO must stay
    # in the context that established the cookie, not navigate to port 3000.
    sed "s|http://127.0.0.1:3000|$base_url|g" \
      "$repo_root/benchmarks/app-compatibility/oauth/$name.oil" >"$task"
    if "$oryn_bin" --file "$task" \
      native --url "$base_url/app-compatibility/auth/$route" --allow-loopback >"$log" 2>"$log.stderr" \
      && [[ ! -s "$log.stderr" ]] \
      && assert_native_result "$log" "$expected" "$task"; then
      echo "application compatibility OAuth passed: $name (native)"
    else
      failures=$((failures + 1))
      echo "application compatibility OAuth failed: $name (native); see $log" >&2
    fi
  done
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

if [[ "$failures" != "0" ]]; then
  echo "application compatibility OAuth failed: $failures of 5 native cases" >&2
  exit 1
fi
echo "application compatibility OAuth corpus passed in domain mode: $domain"
