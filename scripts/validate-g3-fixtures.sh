#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
lock="$repo_root/benchmarks/g3/fixtures.lock.json"

hash_file() {
  shasum -a 256 "$1" | awk '{print $1}'
}

hash_app() {
  local app="$1"
  (
    cd "$repo_root"
    find "benchmarks/g3/frameworks/apps/$app" \
      \( -type d \( -name node_modules -o -name dist -o -name out -o -name build \
        -o -name .next -o -name .nuxt -o -name .output -o -name .angular \
        -o -name .svelte-kit -o -name ssr-dist \) -prune \) -o \
      -type f -print | sort | xargs shasum -a 256 | shasum -a 256 | awk '{print $1}'
  )
}

jq -e '.schema_version == 1 and .locked == true and .hash_algorithm == "sha256"' "$lock" >/dev/null

for app in angular next nuxt preact react svelte sveltekit vue; do
  expected="$(jq -r --arg app "$app" '.frameworks[$app].source_hash' "$lock")"
  actual="$(hash_app "$app")"
  test "$actual" = "$expected" || {
    echo "G3 fixture hash mismatch for $app: expected $expected, got $actual" >&2
    exit 1
  }
done

while IFS=$'\t' read -r key path; do
  expected="$(jq -r --arg key "$key" '.workspace[$key]' "$lock")"
  actual="$(hash_file "$repo_root/$path")"
  test "$actual" = "$expected" || {
    echo "G3 workspace fixture hash mismatch for $path" >&2
    exit 1
  }
done <<'EOF'
package_json	benchmarks/g3/frameworks/package.json
pnpm_lock	benchmarks/g3/frameworks/pnpm-lock.yaml
pnpm_workspace	benchmarks/g3/frameworks/pnpm-workspace.yaml
react_surface_oil	benchmarks/g3/frameworks/framework-react-surface.oil
EOF

while IFS=$'\t' read -r key path; do
  expected="$(jq -r --arg key "$key" '.oauth[$key]' "$lock")"
  actual="$(hash_file "$repo_root/$path")"
  test "$actual" = "$expected" || {
    echo "G3 OAuth fixture hash mismatch for $path" >&2
    exit 1
  }
done <<'EOF'
server	test-harness/scenarios/g3-auth/api.js
existing_session_sso	benchmarks/g3/oauth/existing-session-sso.oil
mfa_handoff	benchmarks/g3/oauth/mfa-handoff.oil
popup_close_recovery	benchmarks/g3/oauth/popup-close-recovery.oil
popup_pkce	benchmarks/g3/oauth/popup-pkce.oil
redirect	benchmarks/g3/oauth/redirect.oil
EOF

while IFS=$'\t' read -r key path; do
  expected="$(jq -r --arg key "$key" '.harness[$key]' "$lock")"
  actual="$(hash_file "$repo_root/$path")"
  if [[ "$actual" != "$expected" ]]; then
    echo "G3 harness hash mismatch for $path: expected $expected, got $actual" >&2
    exit 1
  fi
done <<'EOF'
chromium	test-harness/g3-chromium.js
controlled_regression_validator	scripts/validate-g3-controlled-regressions.sh
framework_validator	scripts/validate-g3-frameworks.sh
limitations_validator	scripts/validate-g3-limitations.sh
oauth_validator	scripts/validate-g3-oauth.sh
wpt_manifest	benchmarks/g3/wpt-subset-v1.json
wpt_validator	scripts/validate-g3-wpt.sh
EOF

expected_manifest="$(jq -r '.g3_manifest' "$lock")"
actual_manifest="$(hash_file "$repo_root/benchmarks/conformance/g3-manifest-v1.json")"
test "$actual_manifest" = "$expected_manifest" || {
  echo "G3 conformance manifest changed after fixture lock" >&2
  exit 1
}

echo "G3 framework, OAuth, workspace, and manifest fixture hashes are valid"
