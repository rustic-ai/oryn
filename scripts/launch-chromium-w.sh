#!/bin/bash
# Launch Chromium browser with extension-w loaded for testing and development

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
EXTENSION_DIR="$PROJECT_ROOT/extension-w"

echo -e "${BLUE}=== Oryn-W Chromium Launcher ===${NC}\n"

# Check if extension directory exists
if [ ! -d "$EXTENSION_DIR" ]; then
    echo -e "${RED}Error: extension-w directory not found at $EXTENSION_DIR${NC}"
    exit 1
fi

# Check for required extension files
if [ ! -f "$EXTENSION_DIR/manifest.json" ]; then
    echo -e "${RED}Error: manifest.json not found in extension-w${NC}"
    exit 1
fi

# Find Chromium binary
CHROMIUM=""
CHROMIUM_PATHS=(
    "/usr/bin/chromium"
    "/usr/bin/chromium-browser"
    "/usr/bin/google-chrome"
    "/usr/bin/google-chrome-stable"
    "/snap/bin/chromium"
    "/opt/google/chrome/google-chrome"
    "/Applications/Chromium.app/Contents/MacOS/Chromium"
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
    "$HOME/.local/bin/chromium"
)

for path in "${CHROMIUM_PATHS[@]}"; do
    if [ -x "$path" ]; then
        CHROMIUM="$path"
        break
    fi
done

if [ -z "$CHROMIUM" ]; then
    echo -e "${RED}Error: Chromium/Chrome browser not found${NC}"
    echo "Please install Chromium or Chrome, or set CHROMIUM_BIN environment variable"
    echo ""
    echo "Installation options:"
    echo "  Ubuntu/Debian: sudo apt install chromium-browser"
    echo "  Fedora: sudo dnf install chromium"
    echo "  Arch: sudo pacman -S chromium"
    echo "  Snap: sudo snap install chromium"
    exit 1
fi

echo -e "${GREEN}✓ Found Chromium: $CHROMIUM${NC}"

# Check if WASM module exists
if [ ! -f "$EXTENSION_DIR/wasm/oryn_core_bg.wasm" ]; then
    echo -e "${YELLOW}⚠ Warning: WASM module not found${NC}"
    echo "The extension will not function without the WASM module."
    echo ""
    echo "Build the WASM module first:"
    echo "  cd $PROJECT_ROOT"
    echo "  ./scripts/build-wasm.sh"
    echo ""
    read -p "Do you want to continue anyway? (y/N) " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        exit 1
    fi
else
    WASM_SIZE=$(wc -c < "$EXTENSION_DIR/wasm/oryn_core_bg.wasm")
    WASM_SIZE_KB=$((WASM_SIZE / 1024))
    echo -e "${GREEN}✓ Found WASM module ($WASM_SIZE_KB KB)${NC}"
fi

# Create temporary user data directory
USER_DATA_DIR="/tmp/chromium-oryn-w-$$"
mkdir -p "$USER_DATA_DIR"

echo -e "${GREEN}✓ Created user data directory: $USER_DATA_DIR${NC}"
echo -e "${GREEN}✓ Loading extension from: $EXTENSION_DIR${NC}"
echo ""

# Parse command line arguments
START_URL="https://example.com"
HEADLESS=false
EXTRA_ARGS=()

while [[ $# -gt 0 ]]; do
    case $1 in
        --url)
            START_URL="$2"
            shift 2
            ;;
        --headless)
            HEADLESS=true
            shift
            ;;
        --help|-h)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --url URL       Open specified URL (default: https://example.com)"
            echo "  --headless      Run in headless mode (limited extension support)"
            echo "  --help, -h      Show this help message"
            echo ""
            echo "Environment variables:"
            echo "  CHROMIUM_BIN    Path to Chromium binary (overrides auto-detection)"
            echo ""
            echo "Examples:"
            echo "  $0"
            echo "  $0 --url https://google.com"
            echo "  CHROMIUM_BIN=/usr/bin/google-chrome $0"
            exit 0
            ;;
        *)
            EXTRA_ARGS+=("$1")
            shift
            ;;
    esac
done

# Build Chromium arguments
CHROMIUM_ARGS=(
    # Extension loading
    "--load-extension=$EXTENSION_DIR"
    "--disable-extensions-except=$EXTENSION_DIR"

    # User data directory
    "--user-data-dir=$USER_DATA_DIR"

    # Development flags
    "--no-first-run"
    "--no-default-browser-check"
    "--disable-default-apps"

    # Enable extension debugging
    "--enable-logging=stderr"
    "--v=1"

    # Security (relaxed for development)
    "--disable-web-security"
    "--disable-features=IsolateOrigins,site-per-process"

    # Performance
    "--disable-background-networking"
    "--disable-background-timer-throttling"
    "--disable-backgrounding-occluded-windows"
    "--disable-renderer-backgrounding"
)

# Add headless flag if requested
if [ "$HEADLESS" = true ]; then
    echo -e "${YELLOW}⚠ Note: Extensions have limited support in headless mode${NC}"
    CHROMIUM_ARGS+=("--headless=new")
fi

# Add extra arguments
if [ ${#EXTRA_ARGS[@]} -gt 0 ]; then
    CHROMIUM_ARGS+=("${EXTRA_ARGS[@]}")
fi

# Add start URL
CHROMIUM_ARGS+=("$START_URL")

echo -e "${BLUE}Starting Chromium with extension-w...${NC}"
echo ""
echo -e "${YELLOW}Extension features:${NC}"
echo "  • Open the extension popup (click extension icon in toolbar)"
echo "  • Open the sidepanel (View > Developer > Side Panel > Oryn Agent)"
echo "  • Try commands: observe, goto \"url\", click \"text\", type \"field\" \"value\""
echo ""
echo -e "${YELLOW}DevTools:${NC}"
echo "  • Extension popup: Right-click extension icon > Inspect popup"
echo "  • Background script: chrome://extensions > Details > Inspect background"
echo "  • Sidepanel: Right-click in sidepanel > Inspect"
echo ""
echo -e "${YELLOW}Press Ctrl+C to stop Chromium and clean up${NC}"
echo ""

# Cleanup function
cleanup() {
    echo ""
    echo -e "${BLUE}Cleaning up...${NC}"
    if [ -d "$USER_DATA_DIR" ]; then
        rm -rf "$USER_DATA_DIR"
        echo -e "${GREEN}✓ Removed temporary user data directory${NC}"
    fi
    echo -e "${GREEN}Done!${NC}"
}

trap cleanup EXIT INT TERM

# Launch Chromium
if [ -n "$CHROMIUM_BIN" ]; then
    CHROMIUM="$CHROMIUM_BIN"
fi

"$CHROMIUM" "${CHROMIUM_ARGS[@]}" 2>&1 | grep -v "DevTools listening" | grep -E "(Extension|WASM|Oryn|Error|Warning)" || true
