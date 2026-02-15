#!/bin/bash
# scripts/test-oryn-r-local.sh
# Test oryn-r (Remote mode) with local Chromium browser

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
NC='\033[0m'

# Get script directory and project root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
RESULTS_DIR="$PROJECT_ROOT/e2e-results"

# Logging
log_info()    { echo -e "${BLUE}[INFO]${NC} $1"; }
log_pass()    { echo -e "${GREEN}[PASS]${NC} $1"; }
log_fail()    { echo -e "${RED}[FAIL]${NC} $1"; }
log_warn()    { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_step()    { echo -e "${CYAN}[STEP]${NC} $1"; }

# Configuration
REMOTE_PORT=9001
HARNESS_PORT=3000
CHROME_DEBUG_PORT=9222

# Paths
ORYN_BIN="$PROJECT_ROOT/target/release/oryn"
EXT_DIR="$PROJECT_ROOT/extension"
SCRIPTS_DIR="$PROJECT_ROOT/test-harness/scripts"

# Test harness PID
HARNESS_PID=""
CHROME_PID=""
ORYN_PID=""

# Cleanup function
cleanup() {
    log_info "Cleaning up..."
    
    # Stop test harness
    if [ -n "$HARNESS_PID" ]; then
        kill "$HARNESS_PID" 2>/dev/null || true
        wait "$HARNESS_PID" 2>/dev/null || true
    fi
    
    # Stop Chrome
    if [ -n "$CHROME_PID" ]; then
        kill "$CHROME_PID" 2>/dev/null || true
        wait "$CHROME_PID" 2>/dev/null || true
    fi
    
    # Stop oryn server
    if [ -n "$ORYN_PID" ]; then
        kill "$ORYN_PID" 2>/dev/null || true
        wait "$ORYN_PID" 2>/dev/null || true
    fi
    
    # Clean up temp directories
    rm -rf /tmp/oryn_ext_test /tmp/oryn_chrome_test 2>/dev/null || true
    
    log_info "Cleanup complete"
}

trap cleanup EXIT

# Print header
print_header() {
    echo ""
    echo -e "${CYAN}╔════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${CYAN}║       ORYN-R LOCAL TEST - Chromium with Extension             ║${NC}"
    echo -e "${CYAN}╚════════════════════════════════════════════════════════════════╝${NC}"
    echo ""
}

# Find Chromium binary
find_chromium() {
    local chrome_bin=""

    # Try Chromium variants (not Chrome branded, as they don't allow unpacked extensions)
    for bin in chromium chromium-browser; do
        if command -v "$bin" &> /dev/null; then
            chrome_bin=$(command -v "$bin")
            # Log to stderr so it doesn't interfere with command substitution
            log_info "Found Chromium: $chrome_bin" >&2
            # Only output the path to stdout
            echo "$chrome_bin"
            return 0
        fi
    done

    log_fail "Chromium not found. Please install chromium or chromium-browser" >&2
    log_fail "Note: Google Chrome (branded) cannot load unpacked extensions from CLI" >&2
    return 1
}

# Start test harness
start_harness() {
    log_step "Starting test harness..."
    
    cd "$PROJECT_ROOT/test-harness"
    
    # Check if node_modules exists
    if [ ! -d "node_modules" ]; then
        log_info "Installing test harness dependencies..."
        npm install --silent
    fi
    
    # Start server
    node server.js > "$RESULTS_DIR/test-harness.log" 2>&1 &
    HARNESS_PID=$!
    
    # Wait for server to be ready
    log_info "Waiting for test harness to start..."
    for i in {1..30}; do
        if curl -s "http://localhost:${HARNESS_PORT}/ping" > /dev/null 2>&1; then
            log_pass "Test harness running on port $HARNESS_PORT"
            cd "$PROJECT_ROOT"
            return 0
        fi
        sleep 0.5
    done
    
    log_fail "Test harness failed to start"
    cd "$PROJECT_ROOT"
    return 1
}

# Prepare extension
prepare_extension() {
    log_step "Preparing extension with auto-connect enabled..."
    
    local ext_patch_dir="/tmp/oryn_ext_test"
    
    # Clean and create temp directory
    rm -rf "$ext_patch_dir"
    mkdir -p "$ext_patch_dir"
    
    # Copy extension files
    cp -r "$EXT_DIR/"* "$ext_patch_dir/"
    
    # Create config.json for auto-connect
    cat > "$ext_patch_dir/config.json" << EXTEOF
{
    "autoConnect": true,
    "websocketUrl": "ws://127.0.0.1:$REMOTE_PORT"
}
EXTEOF
    
    log_info "Extension prepared at: $ext_patch_dir"
    log_info "Auto-connect enabled with URL: ws://127.0.0.1:$REMOTE_PORT"
    
    echo "$ext_patch_dir"
}

# Test auto-connect mechanism
test_autoconnect() {
    local script="$1"
    local script_name=$(basename "$script")
    
    log_step "Testing: $script_name"
    
    # Prepare extension
    local ext_dir=$(prepare_extension)
    local user_data_dir="/tmp/oryn_chrome_test"
    rm -rf "$user_data_dir"
    mkdir -p "$user_data_dir"
    
    # Find Chrome
    local chrome_bin=$(find_chromium)
    if [ -z "$chrome_bin" ]; then
        return 1
    fi
    
    # Start oryn server
    log_info "Starting oryn server on port $REMOTE_PORT..."
    RUST_LOG=info "$ORYN_BIN" --file "$script" remote --port $REMOTE_PORT \
        > "$RESULTS_DIR/oryn_$script_name.log" 2>&1 &
    ORYN_PID=$!
    
    # Wait for server to be ready
    for i in {1..30}; do
        if lsof -ti:$REMOTE_PORT > /dev/null 2>&1; then
            log_pass "Oryn server listening on port $REMOTE_PORT"
            break
        fi
        sleep 0.5
    done
    
    # Launch Chrome with extension
    log_info "Launching Chromium with extension..."
    log_info "  Extension dir: $ext_dir"
    log_info "  User data dir: $user_data_dir"
    log_info "  Config file: $ext_dir/config.json"

    # Show config for debugging
    if [ -f "$ext_dir/config.json" ]; then
        log_info "  Config contents: $(cat $ext_dir/config.json)"
    fi

    # Launch Chrome in the foreground (visible window) but detach it
    # Redirect stderr to log file but keep the window visible
    "$chrome_bin" \
        --user-data-dir="$user_data_dir" \
        --load-extension="$ext_dir" \
        --no-first-run \
        --no-default-browser-check \
        --disable-background-timer-throttling \
        --disable-backgrounding-occluded-windows \
        --disable-renderer-backgrounding \
        --remote-debugging-port=$CHROME_DEBUG_PORT \
        "http://localhost:$HARNESS_PORT/" \
        2>> "$RESULTS_DIR/chrome_$script_name.log" &

    CHROME_PID=$!

    # Give Chrome time to fully start and render the window
    sleep 3
    
    log_info "Chrome launched (PID: $CHROME_PID)"

    # Check if Chrome DevTools is accessible
    sleep 1
    if curl -s "http://localhost:$CHROME_DEBUG_PORT/json/version" > /dev/null 2>&1; then
        log_pass "Chrome DevTools available on port $CHROME_DEBUG_PORT"
        log_info "  You can inspect at: http://localhost:$CHROME_DEBUG_PORT"
    fi

    log_info "Waiting for auto-connect to trigger..."
    log_info "(The browser window should open and the extension should auto-connect when the page loads)"

    # Monitor oryn server logs for connection
    local connected=false
    for i in {1..30}; do
        if grep -q "WebSocket Handshake Successful" "$RESULTS_DIR/oryn_$script_name.log" 2>/dev/null; then
            log_pass "Extension auto-connected successfully!"
            connected=true
            break
        fi
        if grep -q "Extension connected" "$RESULTS_DIR/oryn_$script_name.log" 2>/dev/null; then
            log_pass "Extension auto-connected successfully!"
            connected=true
            break
        fi
        if grep -q "New WebSocket connection: established" "$RESULTS_DIR/oryn_$script_name.log" 2>/dev/null; then
            log_pass "Extension auto-connected successfully!"
            connected=true
            break
        fi
        sleep 1
        echo -n "."
    done
    echo ""
    
    if [ "$connected" = false ]; then
        log_warn "Auto-connect may not have triggered yet, waiting for script execution..."
    fi
    
    # Wait for oryn to complete
    log_info "Waiting for script to complete..."
    if wait "$ORYN_PID"; then
        log_pass "Script completed: $script_name"
        
        # Show summary from logs
        echo ""
        log_info "Server log summary:"
        grep -E "INFO|Extension|WebSocket|Navigated|Value:" "$RESULTS_DIR/oryn_$script_name.log" 2>/dev/null | tail -20 || true
        
        return 0
    else
        log_fail "Script failed: $script_name"
        
        echo ""
        log_info "Server log (last 30 lines):"
        tail -30 "$RESULTS_DIR/oryn_$script_name.log" 2>/dev/null || true
        
        return 1
    fi
}

# Main
main() {
    print_header
    
    # Check prerequisites
    if [ ! -x "$ORYN_BIN" ]; then
        log_fail "oryn binary not found at $ORYN_BIN"
        log_info "Please build with: cargo build --release --package oryn"
        exit 1
    fi
    
    mkdir -p "$RESULTS_DIR"
    
    # Start test harness
    start_harness
    
    echo ""
    
    # Parse arguments
    local test_script="$1"
    
    if [ -n "$test_script" ]; then
        # Run specific script
        test_autoconnect "$test_script"
    else
        # Run first script as demo
        local demo_script="$SCRIPTS_DIR/01_static.oil"
        log_info "No script specified, running demo: $demo_script"
        log_warn "This will open a browser window - check the extension icon and console"
        echo ""
        sleep 2
        test_autoconnect "$demo_script"
    fi
}

# Help
if [ "$1" = "--help" ] || [ "$1" = "-h" ]; then
    echo "Usage: $0 [SCRIPT]"
    echo ""
    echo "Test oryn-r (Remote mode) with local Chromium browser and extension"
    echo ""
    echo "Options:"
    echo "  SCRIPT    Path to .oil script to run (default: 01_static.oil)"
    echo "  --help    Show this help"
    echo ""
    echo "Examples:"
    echo "  $0                                          # Run demo with 01_static.oil"
    echo "  $0 test-harness/scripts/02_forms.oil       # Run specific script"
    echo ""
    echo "This script will:"
    echo "  1. Start the test harness server"
    echo "  2. Prepare the extension with auto-connect enabled"
    echo "  3. Start the oryn WebSocket server"
    echo "  4. Launch Chromium with the extension"
    echo "  5. Verify auto-connect triggers when page loads"
    echo "  6. Execute the .oil script commands"
    echo ""
    exit 0
fi

main "$@"
