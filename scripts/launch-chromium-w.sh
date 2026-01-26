#!/bin/bash
# Launch Chromium/Chrome with extension-w loaded in development mode

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXTENSION_DIR="$SCRIPT_DIR"

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${GREEN}=== Oryn-W Extension Dev Launcher ===${NC}"
echo ""

# Check if WASM module exists
if [ ! -f "$EXTENSION_DIR/wasm/oryn_core_bg.wasm" ]; then
    echo -e "${RED}ERROR: WASM module not found!${NC}"
    echo ""
    echo "Please build the WASM module first:"
    echo "  cd ../crates/oryn-core"
    echo "  wasm-pack build --target web --out-dir ../../extension-w/wasm --release"
    echo ""
    exit 1
fi

# Find Chrome/Chromium binary
CHROME_BIN=""

if command -v google-chrome &> /dev/null; then
    CHROME_BIN="google-chrome"
elif command -v chromium &> /dev/null; then
    CHROME_BIN="chromium"
elif command -v chromium-browser &> /dev/null; then
    CHROME_BIN="chromium-browser"
elif [ -f "/usr/bin/google-chrome-stable" ]; then
    CHROME_BIN="/usr/bin/google-chrome-stable"
elif [ -f "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" ]; then
    CHROME_BIN="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
elif [ -f "/Applications/Chromium.app/Contents/MacOS/Chromium" ]; then
    CHROME_BIN="/Applications/Chromium.app/Contents/MacOS/Chromium"
else
    echo -e "${RED}ERROR: Chrome/Chromium not found!${NC}"
    echo ""
    echo "Please install Chrome or Chromium:"
    echo "  - Ubuntu/Debian: sudo apt install chromium-browser"
    echo "  - Fedora: sudo dnf install chromium"
    echo "  - macOS: brew install --cask chromium"
    echo ""
    exit 1
fi

echo -e "${GREEN}✓${NC} Found Chrome/Chromium: ${CHROME_BIN}"
echo -e "${GREEN}✓${NC} Extension directory: ${EXTENSION_DIR}"
echo -e "${GREEN}✓${NC} WASM module: $(du -h "$EXTENSION_DIR/wasm/oryn_core_bg.wasm" | cut -f1)"
echo ""

# Create a temporary user data directory for clean testing
USER_DATA_DIR="/tmp/oryn-w-dev-$(date +%s)"
mkdir -p "$USER_DATA_DIR"

echo -e "${YELLOW}Launching Chrome with extension...${NC}"
echo ""
echo "Extension will be loaded from: ${EXTENSION_DIR}"
echo "User data directory: ${USER_DATA_DIR}"
echo ""
echo "Press Ctrl+C to stop Chrome and clean up"
echo ""

# Launch Chrome with extension
"$CHROME_BIN" \
    --user-data-dir="$USER_DATA_DIR" \
    --disable-extensions-except="$EXTENSION_DIR" \
    --load-extension="$EXTENSION_DIR" \
    --no-first-run \
    --no-default-browser-check \
    --disable-features=ExtensionsToolbarMenu \
    --enable-logging=stderr \
    --v=0 \
    "https://example.com" \
    2>&1 | grep -E "(Oryn|WASM|Extension|ERROR)" || true

# Cleanup on exit
echo ""
echo -e "${GREEN}Cleaning up temporary user data...${NC}"
rm -rf "$USER_DATA_DIR"
echo -e "${GREEN}Done!${NC}"
