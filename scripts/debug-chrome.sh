#!/bin/bash
# scripts/debug-chrome.sh
# Isolated script to debug extension loading issues.

set -e

# Setup identical environment
EXT_SRC="extension"
EXT_DIR="/tmp/oryn_debug_ext_$(date +%s)"
USER_DATA="/tmp/oryn_debug_profile_$(date +%s)"

echo "Preparing extension in $EXT_DIR..."
mkdir -p "$EXT_DIR"
cp -r "$EXT_SRC/"* "$EXT_DIR/"

# Patch extension (same as runner)
sed -i "s|ws://127.0.0.1:9001|ws://127.0.0.1:9001|g" "$EXT_DIR/background.js"

chmod -R 777 "$EXT_DIR"

echo "Directory Content:"
ls -F "$EXT_DIR"

echo "Launching Chrome..."
echo "1. GO TO: chrome://version"
echo "2. CHECK: Look at the 'Command Line' row."
echo "   - Does it contain '--load-extension'?"
echo "   - Does it point to the /tmp/oryn_debug_ext_... path?"

CHROME_BIN="/usr/lib64/chromium-browser/chromium-browser"

"$CHROME_BIN" \
    --user-data-dir="$USER_DATA" \
    --load-extension="$EXT_DIR" \
    --no-first-run \
    --enable-logging --v=1 \
    --enable-unsafe-extension-debugging "about:blank"
