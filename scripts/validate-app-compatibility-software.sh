#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_dir="${ORYN_APP_COMPATIBILITY_ARTIFACT_DIR:-$repo_root/artifacts/app-compatibility/software}"
poetry="$repo_root/artifacts/tools/poetry/bin/poetry"
node_bin="${ORYN_NODE_BIN:-$(command -v node || true)}"
scanner_chromium_bin="${ORYN_SCANNER_CHROMIUM_BIN:-}"
extension_chromium_bin="${ORYN_EXTENSION_CHROMIUM_BIN:-}"
if [[ -z "$node_bin" && -x "/Users/rohit/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node" ]]; then
  node_bin="/Users/rohit/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node"
fi
for chromium_bin in "$scanner_chromium_bin" "$extension_chromium_bin"; do
  if [[ -n "$chromium_bin" && ! -x "$chromium_bin" ]]; then
    echo "Configured Chromium binary is not executable: $chromium_bin" >&2
    exit 2
  fi
done
mkdir -p "$artifact_dir"
artifact_dir="$(cd "$artifact_dir" && pwd)"

run_logged() {
  local name="$1"
  shift
  echo "Running application compatibility software check: $name"
  "$@" >"$artifact_dir/$name.log" 2>&1
}

cd "$repo_root"
run_logged rust-fmt cargo fmt --all -- --check
run_logged rust-clippy-default cargo clippy --workspace --all-targets -- -D warnings
run_logged rust-clippy-v8 cargo clippy -p oryn-native -p oryn --all-targets \
  --features oryn-native/v8-host,oryn-native/in-process-probe,oryn/native-v8,oryn/in-process-probe \
  -- -D warnings
run_logged rust-workspace cargo test --workspace
run_logged rust-v8 cargo test -p oryn-native --all-targets \
  --features v8-host,in-process-probe
run_logged scanner-sync "$repo_root/scripts/check-scanner-sync.sh"

if [[ ! -x "$poetry" ]]; then
  poetry="$(command -v poetry || true)"
fi
if [[ -z "$poetry" ]]; then
  echo "Poetry is required for the Python SDK and IntentGym checks" >&2
  exit 2
fi
(
  cd "$repo_root/oryn-python"
  run_logged python-ruff "$poetry" run ruff check .
  run_logged python-format "$poetry" run ruff format --check .
  run_logged python-tests "$poetry" run pytest -q
)
(
  cd "$repo_root/intentgym"
  run_logged intentgym-black "$poetry" run black --check .
  run_logged intentgym-isort "$poetry" run isort --check-only .
  run_logged intentgym-tests "$poetry" run pytest -q
)

if [[ -z "$node_bin" ]]; then
  echo "Node.js is required for scanner and extension regressions" >&2
  exit 2
fi
(
  cd "$repo_root/crates/oryn-scanner"
  if [[ -n "$scanner_chromium_bin" ]]; then
    export PUPPETEER_EXECUTABLE_PATH="$scanner_chromium_bin"
  fi
  run_logged scanner-lint "$node_bin" node_modules/eslint/bin/eslint.js src/ tests/
  run_logged scanner-format "$node_bin" node_modules/prettier/bin/prettier.cjs --check src/ tests/
  run_logged scanner-tests "$node_bin" node_modules/jest/bin/jest.js --runInBand
)

run_logged extension-wasm "$repo_root/scripts/build-wasm.sh"
(
  cd "$repo_root/extension-w"
  if [[ -n "$extension_chromium_bin" ]]; then
    export PUPPETEER_EXECUTABLE_PATH="$extension_chromium_bin"
  fi
  run_logged extension-lint "$node_bin" node_modules/eslint/bin/eslint.js '*.js' 'test/**/*.js'
  run_logged extension-unit "$node_bin" node_modules/jest/bin/jest.js test/unit --runInBand
  run_logged extension-integration "$node_bin" node_modules/jest/bin/jest.js test/integration --runInBand
  run_logged extension-e2e "$node_bin" node_modules/jest/bin/jest.js test/e2e --runInBand
)

echo "application compatibility Rust, Python, scanner, and extension software checks passed"
