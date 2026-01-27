#!/bin/bash
# CI check: Verify scanner.js files are in sync
# Exits with error if any files differ from source of truth

SOURCE="crates/oryn-scanner/src/scanner.js"
DEST1="extension/scanner.js"
DEST2="extension-w/scanner.js"

echo "Checking scanner.js sync..."

# Check all files exist
for file in "$SOURCE" "$DEST1" "$DEST2"; do
    if [ ! -f "$file" ]; then
        echo "❌ Error: File not found: $file"
        exit 1
    fi
done

# Get checksums
SOURCE_HASH=$(md5sum "$SOURCE" | awk '{print $1}')
DEST1_HASH=$(md5sum "$DEST1" | awk '{print $1}')
DEST2_HASH=$(md5sum "$DEST2" | awk '{print $1}')

echo "  Source:      $SOURCE_HASH"
echo "  extension/:  $DEST1_HASH"
echo "  extension-w/: $DEST2_HASH"

# Check if all match
if [ "$SOURCE_HASH" != "$DEST1_HASH" ] || [ "$SOURCE_HASH" != "$DEST2_HASH" ]; then
    echo ""
    echo "❌ Error: scanner.js files are out of sync!"
    echo ""
    echo "Run this to sync:"
    echo "  ./scripts/sync-scanner.sh"
    echo ""
    exit 1
fi

echo "✓ All scanner.js files are in sync"
exit 0
