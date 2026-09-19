#!/bin/bash
set -e

# Full-plan generation admission, including direct invocation.
python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"

echo "Building Oryn-W WASM module..."
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
wasm_output="${ORYN_WASM_OUTPUT_DIR:-$repo_root/extension-w/wasm}"
mkdir -p "$wasm_output"
wasm_output="$(cd "$wasm_output" && pwd)"

# Check if wasm-pack is installed
if ! command -v wasm-pack &> /dev/null; then
    echo "Error: wasm-pack is not installed"
    echo "Install with: curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh"
    exit 1
fi

# Create extension-w directory if it doesn't exist
mkdir -p "$wasm_output"

cd "$repo_root/crates/oryn-core"

# Build WASM module
echo "Running wasm-pack build..."
python3 "$repo_root/scripts/generation_gate.py"
wasm-pack build \
    --target web \
    --out-dir "$wasm_output" \
    --release \
    --no-typescript

cd "$repo_root"

# Check WASM size
if [ -f "$wasm_output/oryn_core_bg.wasm" ]; then
    echo ""
    echo "WASM build successful!"
    echo "WASM size:"
    ls -lh "$wasm_output/oryn_core_bg.wasm" | awk '{print $5, $9}'

    # Optimize with wasm-opt if available
    if command -v wasm-opt &> /dev/null; then
        echo ""
        echo "Optimizing with wasm-opt..."
        wasm-opt -Oz \
            "$wasm_output/oryn_core_bg.wasm" \
            -o "$wasm_output/oryn_core_bg.wasm"
        echo "Optimized size:"
        ls -lh "$wasm_output/oryn_core_bg.wasm" | awk '{print $5, $9}'
    else
        echo ""
        echo "Note: wasm-opt not found. Install for better compression:"
        echo "  npm install -g wasm-opt"
        echo "  or use your package manager (e.g., apt install binaryen)"
    fi

    echo ""
    echo "WASM module ready at: $wasm_output/"
else
    echo "Error: WASM build failed"
    exit 1
fi
