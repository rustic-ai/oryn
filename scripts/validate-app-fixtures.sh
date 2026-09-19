#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
lock="$repo_root/benchmarks/app-compatibility/fixtures.lock.json"

hash_file() {
  shasum -a 256 "$1" | awk '{print $1}'
}

hash_app() {
  local app="$1"
  (
    cd "$repo_root"
    find "benchmarks/app-compatibility/frameworks/apps/$app" \
      \( -type d \( -name node_modules -o -name dist -o -name out -o -name build \
        -o -name .next -o -name .nuxt -o -name .output -o -name .angular \
        -o -name .svelte-kit -o -name ssr-dist \) -prune \) -o \
      -type f -print | sort | xargs shasum -a 256 | shasum -a 256 | awk '{print $1}'
  )
}

jq -e '.schema_version == 1 and .locked == true and .hash_algorithm == "sha256"' "$lock" >/dev/null
python3 "$repo_root/scripts/framework_fixture_lock.py" "$repo_root"

for app in angular next nuxt preact react svelte sveltekit vue; do
  expected="$(jq -r --arg app "$app" '.frameworks[$app].source_hash' "$lock")"
  actual="$(hash_app "$app")"
  test "$actual" = "$expected" || {
    echo "application compatibility fixture hash mismatch for $app: expected $expected, got $actual" >&2
    exit 1
  }
done

while IFS=$'\t' read -r key path; do
  expected="$(jq -r --arg key "$key" '.workspace[$key]' "$lock")"
  actual="$(hash_file "$repo_root/$path")"
  test "$actual" = "$expected" || {
    echo "application compatibility workspace fixture hash mismatch for $path" >&2
    exit 1
  }
done <<'EOF'
package_json	benchmarks/app-compatibility/frameworks/package.json
pnpm_lock	benchmarks/app-compatibility/frameworks/pnpm-lock.yaml
pnpm_workspace	benchmarks/app-compatibility/frameworks/pnpm-workspace.yaml
react_surface_oil	benchmarks/app-compatibility/frameworks/framework-react-surface.oil
EOF

while IFS=$'\t' read -r key path; do
  expected="$(jq -r --arg key "$key" '.oauth[$key]' "$lock")"
  actual="$(hash_file "$repo_root/$path")"
  test "$actual" = "$expected" || {
    echo "application compatibility OAuth fixture hash mismatch for $path" >&2
    exit 1
  }
done <<'EOF'
server	test-harness/scenarios/auth-flows/api.js
existing_session_sso	benchmarks/app-compatibility/oauth/existing-session-sso.oil
mfa_handoff	benchmarks/app-compatibility/oauth/mfa-handoff.oil
popup_close_recovery	benchmarks/app-compatibility/oauth/popup-close-recovery.oil
popup_pkce	benchmarks/app-compatibility/oauth/popup-pkce.oil
redirect	benchmarks/app-compatibility/oauth/redirect.oil
EOF

while IFS=$'\t' read -r key path; do
  expected="$(jq -r --arg key "$key" '.harness[$key]' "$lock")"
  actual="$(hash_file "$repo_root/$path")"
  if [[ "$actual" != "$expected" ]]; then
    echo "application compatibility harness hash mismatch for $path: expected $expected, got $actual" >&2
    exit 1
  fi
done <<'EOF'
chromium	test-harness/app-compatibility-chromium.js
controlled_regression_validator	scripts/validate-app-controlled-regressions.sh
framework_validator	scripts/validate-frameworks.sh
limitations_validator	scripts/validate-app-limitations.sh
oauth_validator	scripts/validate-auth-flows.sh
oauth_outcome_validator	scripts/validate_auth_flow.jq
wpt_manifest	benchmarks/app-compatibility/wpt-subset-v1.json
wpt_validator	scripts/validate-app-platform-subset.sh
EOF

expected_manifest="$(jq -r '.app_compatibility_manifest' "$lock")"
actual_manifest="$(hash_file "$repo_root/benchmarks/conformance/app-compatibility-manifest-v1.json")"
test "$actual_manifest" = "$expected_manifest" || {
  echo "application compatibility conformance manifest changed after fixture lock" >&2
  exit 1
}

echo "application compatibility framework, OAuth, workspace, and manifest fixture hashes are valid"
