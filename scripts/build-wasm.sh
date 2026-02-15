#!/bin/bash
set -e

echo "Building Oryn-W WASM module..."

# Check if wasm-pack is installed
if ! command -v wasm-pack &> /dev/null; then
    echo "Error: wasm-pack is not installed"
    echo "Install with: curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh"
    exit 1
fi

# Create extension-w directory if it doesn't exist
mkdir -p extension-w/wasm

cd crates/oryn-core

# Build WASM module
echo "Running wasm-pack build..."
wasm-pack build \
    --target web \
    --out-dir ../../extension-w/wasm \
    --release \
    --no-typescript

cd ../..

# Check WASM size
if [ -f extension-w/wasm/oryn_core_bg.wasm ]; then
    echo ""
    echo "WASM build successful!"
    echo "WASM size:"
    ls -lh extension-w/wasm/oryn_core_bg.wasm | awk '{print $5, $9}'

    # Optimize with wasm-opt if available
    if command -v wasm-opt &> /dev/null; then
        echo ""
        echo "Optimizing with wasm-opt..."
        wasm-opt -Oz \
            extension-w/wasm/oryn_core_bg.wasm \
            -o extension-w/wasm/oryn_core_bg.wasm
        echo "Optimized size:"
        ls -lh extension-w/wasm/oryn_core_bg.wasm | awk '{print $5, $9}'
    else
        echo ""
        echo "Note: wasm-opt not found. Install for better compression:"
        echo "  npm install -g wasm-opt"
        echo "  or use your package manager (e.g., apt install binaryen)"
    fi

    echo ""
    echo "WASM module ready at: extension-w/wasm/"
else
    echo "Error: WASM build failed"
    exit 1
fi
