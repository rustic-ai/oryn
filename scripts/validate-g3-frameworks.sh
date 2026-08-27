#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
oryn_bin="${ORYN_BIN:-$repo_root/target/debug/oryn}"
node_bin="${ORYN_NODE_BIN:-$(command -v node || true)}"
if [[ -z "$node_bin" && -x "/Users/rohit/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node" ]]; then
  node_bin="/Users/rohit/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node"
fi
port="${ORYN_G3_PORT:-3103}"
domain="${ORYN_G3_DOMAIN:-both}"
artifact_dir="${ORYN_G3_ARTIFACT_DIR:-$repo_root/artifacts/g3/frameworks}"
server_pid=""

cleanup() {
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if [[ "${ORYN_G3_EXTERNAL_SERVER:-0}" != "1" ]]; then
  if [[ -z "$node_bin" ]]; then
    echo "Node.js is required to start the G3 fixture server" >&2
    exit 2
  fi
  (
    cd "$repo_root/test-harness"
    PORT="$port" "$node_bin" server.js
  ) >"${TMPDIR:-/tmp}/oryn-g3-harness-$port.log" 2>&1 &
  server_pid="$!"
  for _ in $(seq 1 50); do
    curl --fail --silent "http://127.0.0.1:$port/ping" >/dev/null 2>&1 && break
    sleep 0.1
  done
fi

curl --fail --silent "http://127.0.0.1:$port/ping" >/dev/null
mkdir -p "$artifact_dir"

frameworks=(react next vue nuxt angular svelte sveltekit preact)
for framework in "${frameworks[@]}"; do
  url="http://127.0.0.1:$port/g3/frameworks/$framework/"
  if [[ "$domain" == "native" || "$domain" == "both" ]]; then
    log="$artifact_dir/$framework-native.jsonl"
    oil="$repo_root/benchmarks/g3/frameworks/framework-action-smoke.oil"
    if [[ "$framework" == "react" ]]; then
      oil="$repo_root/benchmarks/g3/frameworks/framework-react-surface.oil"
    fi
    "$oryn_bin" --file "$oil" \
      native --url "$url" --allow-loopback >"$log"
    jq -s -e --arg framework "$framework" '
      any(.[]; .kind == "structured" and .value == "Items: 1") and
      ($framework != "react" or any(.[];
        .kind == "structured" and
        .value == "Surface: oryn|tools|2026-08-27|2|1|fixture.txt:7|loaded|closed"
      )) and
      (all(.[]; (.kind != "trace") or all(.events[]; .kind.kind != "script_diagnostic" and (.kind.kind != "console" or .kind.level != "error"))))
    ' "$log" >/dev/null
    echo "G3 framework passed: $framework (native)"
  fi
done

if [[ "$domain" == "chromium" || "$domain" == "both" ]]; then
  if [[ -z "$node_bin" ]]; then
    echo "Node.js is required to run the Chromium differential harness" >&2
    exit 2
  fi
  chromium_log="$artifact_dir/chromium.jsonl"
  "$node_bin" "$repo_root/test-harness/g3-chromium.js" frameworks \
    "http://127.0.0.1:$port" >"$chromium_log"
  [[ "$(jq -s '[.[] | select(.domain == "chromium" and .status == "passed")] | length' "$chromium_log")" == "8" ]]
  echo "G3 framework corpus passed: Chromium differential"
fi

echo "G3 framework corpus passed in domain mode: $domain"
