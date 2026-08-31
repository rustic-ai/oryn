#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

POETRY="$ROOT_DIR/artifacts/tools/poetry/bin/poetry"
if [[ ! -x "$POETRY" ]]; then
  POETRY="$(command -v poetry || true)"
fi
if [[ -z "$POETRY" ]]; then
  echo "Poetry is required for production/profile Python checks" >&2
  exit 2
fi

cargo run -q -p oryn-webidl -- check \
  crates/oryn-webidl/idl/browser.webidl \
  crates/oryn-js/src/generated/browser.rs

"$ROOT_DIR/scripts/validate-production-parent-purity.sh"

cargo test -p oryn-native --lib --bins --tests
cargo test -p oryn-native --lib --bins --tests --features "v8-host v8-sandbox core-full"
cargo test -p oryn-native --lib --bins --tests --features "v8-host v8-sandbox core-lite"
cargo test -p oryn --features "native-v8 core-full"

cargo check -p oryn --no-default-features --features native
cargo check -p oryn --no-default-features --features "native native-v8 core-lite"

(
  cd oryn-python
  "$POETRY" run pytest -q
  "$POETRY" run ruff check src tests
  "$POETRY" run ruff format --check src tests
)

echo "production/profile equivalence software matrix passed"
