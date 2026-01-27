#!/bin/bash
set -e

echo "========================================="
echo "Building Oryn-W Extension"
echo "========================================="
echo ""

# Step 0: Sync scanner.js from source
echo "Step 0: Syncing scanner.js from source..."
./scripts/sync-scanner.sh

# Step 1: Build WASM module
echo "Step 1: Building WASM module..."
./scripts/build-wasm.sh

# Step 2: Verify extension files
echo ""
echo "Step 2: Verifying extension files..."

REQUIRED_FILES=(
    "extension-w/manifest.json"
    "extension-w/background.js"
    "extension-w/popup.html"
    "extension-w/popup.js"
    "extension-w/sidepanel.html"
    "extension-w/sidepanel.js"
    "extension-w/scanner.js"
    "extension-w/content.js"
    "extension-w/suppress_alerts.js"
)

MISSING_FILES=()
for file in "${REQUIRED_FILES[@]}"; do
    if [ ! -f "$file" ]; then
        MISSING_FILES+=("$file")
    fi
done

if [ ${#MISSING_FILES[@]} -gt 0 ]; then
    echo "Error: Missing required files:"
    for file in "${MISSING_FILES[@]}"; do
        echo "  - $file"
    done
    exit 1
fi

echo "All required files present ✓"

# Step 3: Verify WASM files
echo ""
echo "Step 3: Verifying WASM files..."

if [ ! -f "extension-w/wasm/oryn_core_bg.wasm" ]; then
    echo "Error: WASM binary not found"
    exit 1
fi

if [ ! -f "extension-w/wasm/oryn_core.js" ]; then
    echo "Error: WASM JavaScript wrapper not found"
    exit 1
fi

echo "WASM files present ✓"

# Step 4: Display summary
echo ""
echo "========================================="
echo "Build Complete!"
echo "========================================="
echo ""
echo "Extension location: $(pwd)/extension-w"
echo ""
echo "To load in Chrome:"
echo "  1. Open chrome://extensions"
echo "  2. Enable 'Developer mode'"
echo "  3. Click 'Load unpacked'"
echo "  4. Select the extension-w/ directory"
echo ""
echo "Files:"
ls -lh extension-w/*.{html,js,json} 2>/dev/null | awk '{print "  ", $9, "-", $5}'
echo ""
echo "WASM:"
ls -lh extension-w/wasm/*.wasm 2>/dev/null | awk '{print "  ", $9, "-", $5}'
echo ""
