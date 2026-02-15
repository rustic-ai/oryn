#!/bin/bash
# Pack Oryn-W Extension for Distribution
# Creates both .zip (for Chrome Web Store) and .crx (for direct distribution)

set -e

# Get script directory (project root)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

EXTENSION_DIR="${SCRIPT_DIR}/extension-w"
BUILD_DIR="${SCRIPT_DIR}/build"
DIST_DIR="${SCRIPT_DIR}/dist"
VERSION=$(node -p "require('${EXTENSION_DIR}/manifest.json').version")
EXTENSION_NAME="oryn-w-${VERSION}"

echo "========================================="
echo "Packing Oryn-W Extension v${VERSION}"
echo "========================================="
echo ""

# Step 1: Verify extension is built
echo "Step 1: Verifying extension build..."
if [ ! -f "${EXTENSION_DIR}/wasm/oryn_core_bg.wasm" ]; then
    echo "❌ WASM module not found. Please run ./scripts/build-extension-w.sh first"
    exit 1
fi

if [ ! -f "${EXTENSION_DIR}/background.js" ]; then
    echo "❌ Extension files not found. Please run ./scripts/build-extension-w.sh first"
    exit 1
fi

echo "✓ Extension files verified"
echo ""

# Step 2: Create distribution directory
echo "Step 2: Creating distribution directory..."
mkdir -p "${DIST_DIR}"
echo "✓ Distribution directory ready: ${DIST_DIR}/"
echo ""

# Step 3: Create .zip package for Chrome Web Store
echo "Step 3: Creating .zip package..."

# Files to include in the package
FILES_TO_PACK=(
    "manifest.json"
    "background.js"
    "content.js"
    "sidepanel.html"
    "sidepanel.js"
    "popup.html"
    "popup.js"
    "scanner.js"
    "suppress_alerts.js"
    "icons/"
    "wasm/"
    "llm/"
    "agent/"
    "ui/"
)

# Files to exclude (development/documentation)
EXCLUDE_PATTERNS=(
    "*.md"
    "*.txt"
    ".git*"
    "node_modules"
    "package.json"
    "package-lock.json"
    ".eslintrc.json"
    "test/"
    "BUILD_STATUS.md"
    "BUILD_COMPLETE.md"
    "DEV_GUIDE.md"
    "E2E_TEST_RESULTS.md"
    "LAUNCH.md"
    "LAUNCH_README.md"
    "README.md"
    "TESTING.md"
    "TEST_RESULTS.md"
    "WASM_TESTING.md"
)

# Create temporary directory for clean packaging
TEMP_DIR=$(mktemp -d)
PACK_DIR="${TEMP_DIR}/${EXTENSION_NAME}"
mkdir -p "${PACK_DIR}"

# Copy files to pack directory
for file in "${FILES_TO_PACK[@]}"; do
    SRC="${EXTENSION_DIR}/$file"
    if [ -e "$SRC" ]; then
        # Preserve directory structure
        if [ -d "$SRC" ]; then
            cp -r "$SRC" "${PACK_DIR}/"
        else
            cp "$SRC" "${PACK_DIR}/"
        fi
        echo "  ✓ Packed: $file"
    fi
done

# Create .zip archive
cd "${TEMP_DIR}"
ZIP_FILE="${DIST_DIR}/${EXTENSION_NAME}.zip"
zip -r -q "${ZIP_FILE}" "${EXTENSION_NAME}"

cd - > /dev/null
echo ""
echo "✓ .zip package created: ${DIST_DIR}/${EXTENSION_NAME}.zip"

# Get file size
ZIP_SIZE=$(du -h "${DIST_DIR}/${EXTENSION_NAME}.zip" | cut -f1)
echo "  Size: ${ZIP_SIZE}"
echo ""

# Clean up temp directory
rm -rf "${TEMP_DIR}"

# Step 4: Create .crx package (optional, requires Chrome)
echo "Step 4: Creating .crx package (optional)..."
echo ""
echo "To create a .crx package for direct distribution:"
echo "  1. Open Chrome and go to chrome://extensions"
echo "  2. Enable 'Developer mode'"
echo "  3. Click 'Pack extension'"
echo "  4. Browse to: ${EXTENSION_DIR}"
echo "  5. Leave 'Private key file' empty (first time)"
echo "  6. Click 'Pack Extension'"
echo ""
echo "Or use the command line (requires Chrome):"
echo "  chrome --pack-extension=${EXTENSION_DIR}"
echo ""
echo "This will create:"
echo "  - ${EXTENSION_DIR}.crx (the packed extension)"
echo "  - ${EXTENSION_DIR}.pem (private key - KEEP SAFE!)"
echo ""

# Step 5: Generate package info
echo "Step 5: Generating package info..."

cat > "${DIST_DIR}/${EXTENSION_NAME}.txt" << EOF
Oryn-W Extension Package v${VERSION}
========================================

Package Details:
  Name:          Oryn Agent (WASM)
  Version:       ${VERSION}
  Package Date:  $(date)
  Package Size:  ${ZIP_SIZE}

Contents:
  - Core Extension Files (background.js, sidepanel, content scripts)
  - WASM Module (oryn_core_bg.wasm 2.0 MB)
  - LLM Infrastructure (Chrome AI, OpenAI, Claude, Gemini adapters)
  - Ralph Agent (autonomous task execution)
  - Trajectory Store (IndexedDB with seed examples)
  - UI Components (dual-mode interface)

Installation:
  1. Unzip the package
  2. Open Chrome → chrome://extensions
  3. Enable "Developer mode"
  4. Click "Load unpacked"
  5. Select the unzipped directory

For Chrome Web Store submission:
  - Use the .zip file: ${EXTENSION_NAME}.zip
  - Upload to Chrome Developer Dashboard
  - Complete store listing information
  - Submit for review

For direct distribution (.crx):
  - Pack the extension using Chrome
  - Distribute the .crx file
  - Users can drag-and-drop into chrome://extensions

Documentation:
  - See BUILD_COMPLETE.md in extension directory
  - See website/docs/integrations/extension-w/ for extension docs

Support:
  - GitHub: https://github.com/anthropics/oryn
  - Issues: https://github.com/anthropics/oryn/issues
EOF

echo "✓ Package info saved: ${DIST_DIR}/${EXTENSION_NAME}.txt"
echo ""

# Step 6: Create checksums
echo "Step 6: Creating checksums..."
pushd "${DIST_DIR}" > /dev/null
sha256sum "${EXTENSION_NAME}.zip" > "${EXTENSION_NAME}.sha256"
echo "✓ SHA256 checksum: ${EXTENSION_NAME}.sha256"
echo ""
popd > /dev/null

# Final summary
echo "========================================="
echo "Packaging Complete!"
echo "========================================="
echo ""
echo "Package Location: ${DIST_DIR}/"
echo ""
echo "Files Created:"
echo "  ✓ ${EXTENSION_NAME}.zip        - Chrome Web Store package"
echo "  ✓ ${EXTENSION_NAME}.txt        - Package information"
echo "  ✓ ${EXTENSION_NAME}.sha256     - Checksum for verification"
echo ""
echo "Next Steps:"
echo ""
echo "  For Chrome Web Store:"
echo "    1. Go to https://chrome.google.com/webstore/devconsole"
echo "    2. Click 'New Item'"
echo "    3. Upload ${EXTENSION_NAME}.zip"
echo "    4. Fill out store listing"
echo "    5. Submit for review"
echo ""
echo "  For Direct Distribution:"
echo "    1. Pack extension using Chrome (see instructions above)"
echo "    2. Distribute the .crx file"
echo "    3. Include installation instructions"
echo ""
echo "  For Testing:"
echo "    1. Unzip ${EXTENSION_NAME}.zip"
echo "    2. Load unpacked in chrome://extensions"
echo "    3. Test thoroughly before distribution"
echo ""
echo "✨ Ready to distribute!"
echo ""
