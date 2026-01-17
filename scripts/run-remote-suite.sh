#!/bin/bash
# scripts/run-remote-suite.sh
# Run all Lemma scripts using the Remote Backend and Chrome Extension (Headed Local)
set -e

# Configuration
PORT=9001
DEBUG_PORT=9002
LCOPE_BIN="./target/debug/lscope"
ENV_EXT_DIR="extension"

# Use /tmp to avoid Snap/Sandbox permission issues
EXT_PATCH_DIR="/tmp/lscope_ext_$(date +%s)"
USER_DATA_DIR="/tmp/lscope_chrome_data_$(date +%s)"
CHROME_BIN="/usr/lib64/chromium-browser/chromium-browser"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info() { echo -e "${YELLOW}[INFO]${NC} $1"; }
log_pass() { echo -e "${GREEN}[PASS]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

cleanup() {
    log_info "Cleaning up..."
    if [ -n "$CHROME_PID" ]; then
        kill "$CHROME_PID" 2>/dev/null || true
        wait "$CHROME_PID" 2>/dev/null || true
    fi
    if [ -n "$HARNESS_PID" ]; then
        kill "$HARNESS_PID" 2>/dev/null || true
    fi
    fuser -k $PORT/tcp 2>/dev/null || true
    rm -rf "$EXT_PATCH_DIR" "$USER_DATA_DIR"
}
# trap cleanup EXIT

# 0. Build Rust
log_info "Building lscope..."
cargo build --package lscope --quiet

# 0.5 Start Harness
log_info "Starting Test Harness..."
node test-harness/server.js > test-harness.log 2>&1 &
HARNESS_PID=$!
sleep 2

# 1. Prepare Ext
log_info "Patching extension..."
rm -rf "$EXT_PATCH_DIR"
mkdir -p "$EXT_PATCH_DIR"
cp -r "$ENV_EXT_DIR/"* "$EXT_PATCH_DIR/"
sed -i "s|ws://127.0.0.1:9001|ws://127.0.0.1:$PORT|g" "$EXT_PATCH_DIR/background.js"

# 2. Results Init
RESULTS_FILE="remote-test-results.md"
echo "# Remote Mode Test Results" > "$RESULTS_FILE"
echo "Generated on: $(date)" >> "$RESULTS_FILE"
echo "| Script | Status |" >> "$RESULTS_FILE"
echo "|--------|--------|" >> "$RESULTS_FILE"

# 3. Loop
for script in test-harness/scripts/*.lemma; do
    script_name=$(basename "$script")
    log_info "TESTING: $script_name"
    
    # Ensure port is clean
    fuser -k $PORT/tcp 2>/dev/null || true
    sleep 1

    # Clean user data dir for isolation
    rm -rf "$USER_DATA_DIR"
    mkdir -p "$USER_DATA_DIR"

    # 1. Start lscope Server (Background)
    log_info "Starting lscope Backend..."
    RUST_LOG=info $LCOPE_BIN --file "$script" remote --port $PORT > "lscope_$script_name.log" 2>&1 &
    LSCOPE_PID=$!
    
    # 2. Wait for Port 9001
    log_info "Waiting for lscope to listen on $PORT..."
    for i in {1..30}; do
        if nc -z 127.0.0.1 $PORT; then
            log_pass "Server is UP"
            break
        fi
        sleep 0.5
    done

    # Debug: List extension files
    log_info "Verifying extension at $EXT_PATCH_DIR:"
    ls -F "$EXT_PATCH_DIR"

    # 3. Launch Chrome (Headed)
    log_info "Launching Chrome (Headed)..."
    CMD="$CHROME_BIN \
        --no-sandbox \
        --disable-gpu \
        --disable-first-run-ui \
        --no-first-run \
        --no-default-browser-check \
        --enable-logging \
        --v=1 \
        --user-data-dir=$USER_DATA_DIR \
        --load-extension=$EXT_PATCH_DIR \
        --remote-debugging-port=$DEBUG_PORT \
        http://localhost:3000/"

    echo "Running: $CMD"
    $CMD > "chrome_$script_name.log" 2>&1 &
    CHROME_PID=$!

    # 4. Wait for lscope to finish
    log_info "Waiting for test completion..."
    wait "$LSCOPE_PID"
    EXIT_CODE=$?
    
    if [ $EXIT_CODE -eq 0 ]; then
        log_pass "$script_name passed"
        echo "| $script_name | ✅ PASS |" >> "$RESULTS_FILE"
    else
        log_error "$script_name failed with exit code $EXIT_CODE"
        echo "| $script_name | ❌ FAIL |" >> "$RESULTS_FILE"
        cat "lscope_$script_name.log" | tail -n 20
        cat "chrome_$script_name.log" | tail -n 20
    fi

    log_info "Stopping Chrome..."
    if [ -n "$CHROME_PID" ]; then
        kill "$CHROME_PID" 2>/dev/null || true
        wait "$CHROME_PID" 2>/dev/null || true
    fi
    fuser -k $PORT/tcp 2>/dev/null || true
    sleep 2
done

log_info "All tests completed. See $RESULTS_FILE"
