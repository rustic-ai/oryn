#!/bin/bash
# scripts/test-extension-w.sh
# Test extension-w with Chromium and capture console logs

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

# Get script directory and project root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Logging
log_info()    { echo -e "${BLUE}[INFO]${NC} $1"; }
log_pass()    { echo -e "${GREEN}[PASS]${NC} $1"; }
log_fail()    { echo -e "${RED}[FAIL]${NC} $1"; }
log_warn()    { echo -e "${YELLOW}[WARN]${NC} $1"; }

# Paths
EXT_DIR="$PROJECT_ROOT/extension-w"
USER_DATA_DIR="/tmp/oryn_w_test_profile"
LOG_FILE="$PROJECT_ROOT/extension-w-test.log"

# Cleanup function
cleanup() {
    log_info "Cleaning up..."
    # Kill any chromium processes using our test profile
    pkill -f "$USER_DATA_DIR" 2>/dev/null || true
    # Clean up temp directory
    rm -rf "$USER_DATA_DIR" 2>/dev/null || true
}

trap cleanup EXIT

# Print header
echo ""
echo -e "${CYAN}╔════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║           Testing Oryn-W Extension                             ║${NC}"
echo -e "${CYAN}╚════════════════════════════════════════════════════════════════╝${NC}"
echo ""

# Find Chromium binary
find_chromium() {
    for bin in chromium chromium-browser google-chrome-unstable google-chrome-beta google-chrome; do
        if command -v "$bin" &> /dev/null; then
            echo $(command -v "$bin")
            return 0
        fi
    done
    return 1
}

CHROME_BIN=$(find_chromium)
if [ $? -ne 0 ]; then
    log_fail "No Chromium/Chrome browser found"
    exit 1
fi

log_info "Found browser: $CHROME_BIN"

# Check if extension directory exists
if [ ! -d "$EXT_DIR" ]; then
    log_fail "Extension directory not found: $EXT_DIR"
    log_info "Please run: ./scripts/build-extension-w.sh"
    exit 1
fi

log_pass "Extension directory found: $EXT_DIR"

# Create fresh user data directory
log_info "Creating fresh browser profile..."
rm -rf "$USER_DATA_DIR"
mkdir -p "$USER_DATA_DIR"

# Launch Chromium with extension
log_info "Launching browser with extension-w..."
log_info "Extension will auto-open first-run wizard"
log_info ""
log_warn "INSTRUCTIONS:"
log_warn "1. Check browser console for errors"
log_warn "2. Open DevTools on the wizard tab (F12)"
log_warn "3. Click 'service worker' link on chrome://extensions to see background logs"
log_warn "4. Complete the wizard flow and observe logs"
log_warn "5. Close browser when done - logs will be saved"
log_warn ""
log_info "Log file: $LOG_FILE"
log_info ""
log_info "Press Ctrl+C to stop the browser"
log_info ""

# Launch with remote debugging to capture logs
"$CHROME_BIN" \
    --user-data-dir="$USER_DATA_DIR" \
    --load-extension="$EXT_DIR" \
    --no-first-run \
    --no-default-browser-check \
    --enable-logging=stderr \
    --v=1 \
    --remote-debugging-port=9222 \
    --new-window \
    "about:blank" \
    2>&1 | tee "$LOG_FILE"

log_info ""
log_pass "Browser closed"
log_info "Logs saved to: $LOG_FILE"
log_info ""
log_info "To inspect logs:"
log_info "  grep 'LLM Manager' $LOG_FILE"
log_info "  grep 'Wizard' $LOG_FILE"
log_info "  grep 'WebLLM' $LOG_FILE"
log_info "  grep -i error $LOG_FILE"
