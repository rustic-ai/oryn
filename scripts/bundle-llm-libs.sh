#!/bin/bash
set -e

echo "Bundling LLM libraries for extension-w..."

VENDOR_DIR="extension-w/llm/vendor"
WASM_DIR="$VENDOR_DIR/wllama-wasm"

# Clean previous bundles
rm -rf "$VENDOR_DIR"
mkdir -p "$VENDOR_DIR"
mkdir -p "$WASM_DIR/single-thread"
mkdir -p "$WASM_DIR/multi-thread"

# A) Bundle web-llm
echo "  Bundling web-llm..."
npx esbuild node_modules/@mlc-ai/web-llm/lib/index.js \
  --bundle --format=esm --outfile="$VENDOR_DIR/webllm.bundle.js" --minify

# B) Bundle wllama
echo "  Bundling wllama..."
npx esbuild node_modules/@wllama/wllama/esm/index.js \
  --bundle --format=esm --outfile="$VENDOR_DIR/wllama.bundle.js" \
  --external:*.wasm --minify

# C) Copy wllama WASM runtime files
echo "  Copying wllama WASM runtime files..."
WLLAMA_ESM="node_modules/@wllama/wllama/esm"

cp "$WLLAMA_ESM/single-thread/wllama.js"   "$WASM_DIR/single-thread/"
cp "$WLLAMA_ESM/single-thread/wllama.wasm" "$WASM_DIR/single-thread/"
cp "$WLLAMA_ESM/multi-thread/wllama.js"    "$WASM_DIR/multi-thread/"
cp "$WLLAMA_ESM/multi-thread/wllama.wasm"  "$WASM_DIR/multi-thread/"
cp "$WLLAMA_ESM/multi-thread/wllama.worker.mjs" "$WASM_DIR/multi-thread/"

echo "  LLM bundles ready:"
ls -lh "$VENDOR_DIR"/*.js 2>/dev/null | awk '{print "    ", $9, "-", $5}'
echo "  WASM files:"
find "$WASM_DIR" -type f | while read f; do
  echo "    $f ($(du -h "$f" | cut -f1))"
done
