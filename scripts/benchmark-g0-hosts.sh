#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTPUT_DIR="${1:-$ROOT_DIR/artifacts/g0-hosts}"
SAMPLES="${SAMPLES:-30}"
ITERATIONS="${ITERATIONS:-100}"
mkdir -p "$OUTPUT_DIR"

cd "$ROOT_DIR"

build_probe() {
    local feature="$1"
    local target_dir="$2"
    CARGO_TARGET_DIR="$target_dir" cargo build --release --locked -p oryn-native --features "$feature"
}

run_probe() {
    local binary="$1"
    local output="$2"
    for _ in $(seq 1 5); do "$binary" "$ITERATIONS" >/dev/null; done
    : > "$output"
    for _ in $(seq 1 "$SAMPLES"); do "$binary" "$ITERATIONS" >> "$output"; done
}

RAW_TARGET="$OUTPUT_DIR/target-raw"
DENO_TARGET="$OUTPUT_DIR/target-deno"
build_probe v8-host "$RAW_TARGET"
build_probe deno-host "$DENO_TARGET"

RAW_BINARY="$RAW_TARGET/release/oryn-host-probe"
DENO_BINARY="$DENO_TARGET/release/oryn-deno-host-probe"
run_probe "$RAW_BINARY" "$OUTPUT_DIR/raw-v8.jsonl"
run_probe "$DENO_BINARY" "$OUTPUT_DIR/deno-core.jsonl"

wc -c "$RAW_BINARY" "$DENO_BINARY" > "$OUTPUT_DIR/installed-size.txt"
git rev-parse HEAD > "$OUTPUT_DIR/commit.txt"
sw_vers > "$OUTPUT_DIR/os.txt"
uname -m >> "$OUTPUT_DIR/os.txt"

echo "G0 host measurements written to $OUTPUT_DIR"
