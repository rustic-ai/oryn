#!/usr/bin/env bash
set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTPUT_DIR="${1:-$ROOT_DIR/artifacts/p0-baseline}"
mkdir -p "$OUTPUT_DIR"

run_logged() {
    local name="$1"
    shift
    echo "[P0] $name"
    if "$@" 2>&1 | tee "$OUTPUT_DIR/$name.log"; then
        return 0
    fi
    local status="${PIPESTATUS[0]}"
    printf '%s\n' "$status" > "$OUTPUT_DIR/$name.failed"
    echo "[P0] $name failed with status $status" >&2
    return 0
}

cd "$ROOT_DIR"
git rev-parse HEAD > "$OUTPUT_DIR/commit.txt"
rustc --version > "$OUTPUT_DIR/rust-toolchain.txt"
cargo --version >> "$OUTPUT_DIR/rust-toolchain.txt"

run_logged rust-format cargo fmt --all -- --check
run_logged rust-clippy cargo clippy --workspace --locked -- -D warnings
run_logged rust-tests cargo test --workspace --locked

if command -v npm >/dev/null 2>&1; then
    (
        cd crates/oryn-scanner
        run_logged scanner-install npm ci
        run_logged scanner npm run check
    )
    (
        cd extension-w
        run_logged extension-install npm ci
        run_logged extension npm run test:all
    )
else
    echo "npm is required for the complete P0 baseline" | tee "$OUTPUT_DIR/node-missing.log"
    printf '2\n' > "$OUTPUT_DIR/node.failed"
fi

if command -v poetry >/dev/null 2>&1; then
    (
        cd oryn-python
        run_logged python-sdk-install poetry install
        run_logged python-sdk poetry run pytest
    )
    (
        cd intentgym
        run_logged intentgym-install poetry install
        run_logged intentgym poetry run pytest
    )
else
    echo "poetry is required for the complete P0 baseline" | tee "$OUTPUT_DIR/poetry-missing.log"
    printf '2\n' > "$OUTPUT_DIR/poetry.failed"
fi

if compgen -G "$OUTPUT_DIR/*.failed" >/dev/null; then
    echo "P0 baseline captured with failures:" >&2
    for marker in "$OUTPUT_DIR"/*.failed; do
        echo "  $(basename "$marker" .failed) (status $(<"$marker"))" >&2
    done
    exit 1
fi

echo "P0 baseline complete: $OUTPUT_DIR"
