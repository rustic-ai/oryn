#!/bin/bash
# Sync scanner.js from source of truth to extension directories
# Source: crates/oryn-scanner/src/scanner.js
# Destinations: extension/scanner.js, extension-w/scanner.js

set -e

SOURCE="crates/oryn-scanner/src/scanner.js"
DEST1="extension/scanner.js"
DEST2="extension-w/scanner.js"

echo "========================================="
echo "Syncing Scanner.js"
echo "========================================="
echo ""

# Check source exists
if [ ! -f "$SOURCE" ]; then
    echo "Error: Source file not found: $SOURCE"
    exit 1
fi

# Get source info
SOURCE_SIZE=$(wc -c < "$SOURCE")
SOURCE_LINES=$(wc -l < "$SOURCE")

echo "Source: $SOURCE"
echo "  Size: $SOURCE_SIZE bytes"
echo "  Lines: $SOURCE_LINES"
echo ""

# Copy to extension/
echo "Syncing to: $DEST1"
cp "$SOURCE" "$DEST1"
echo "  ✓ Copied"

# Copy to extension-w/
echo "Syncing to: $DEST2"
cp "$SOURCE" "$DEST2"
echo "  ✓ Copied"

echo ""
echo "========================================="
echo "Sync Complete!"
echo "========================================="
echo ""
echo "Verification:"
md5sum "$SOURCE" "$DEST1" "$DEST2"
echo ""
echo "All three files should have identical MD5 hashes."
echo ""
echo "Next steps:"
echo "  1. Test extension (oryn-r): Load extension/ in Chrome and test"
echo "  2. Test extension-w: ./scripts/build-extension-w.sh && test"
echo "  3. Test backends: ./scripts/run-e2e-tests.sh"
echo ""
