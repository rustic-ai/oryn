#!/bin/bash
# scripts/run-unified-suite.sh
# Runs the Lemma Test Harness suite across all backends: Embedded, Headless, Remote.

set -e

# Configuration
PORT=9001
DEBUG_PORT=9002
LCOPE_BIN="./target/debug/oryn"
ENV_EXT_DIR="extension"

# Directories
BUILD_DIR="$(pwd)"
EXT_PATCH_DIR="/tmp/oryn_ext_unified_$(date +%s)"
USER_DATA_DIR="/tmp/oryn_chrome_data_unified_$(date +%s)"
RESULTS_FILE="unified-test-results.md"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_pass() { echo -e "${GREEN}[PASS]${NC} $1"; }
log_fail() { echo -e "${RED}[FAIL]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }

# Cleanup Function
cleanup() {
    log_info "Cleaning up..."
    if [ -n "$CHROME_PID" ]; then kill "$CHROME_PID" 2>/dev/null || true; fi
    if [ -n "$HARNESS_PID" ]; then kill "$HARNESS_PID" 2>/dev/null || true; fi
    fuser -k $PORT/tcp 2>/dev/null || true
    rm -rf "$EXT_PATCH_DIR" "$USER_DATA_DIR"
}
trap cleanup EXIT

# Initialize Results
echo "# Unified Test Results" > "$RESULTS_FILE"
echo "Generated on: $(date)" >> "$RESULTS_FILE"
echo "" >> "$RESULTS_FILE"
echo "| Backend | Script | Status | Notes |" >> "$RESULTS_FILE"
echo "|---------|--------|--------|-------|" >> "$RESULTS_FILE"

# 1. Build
log_info "Building oryn..."
cargo build --package oryn --quiet

# 2. Start Test Harness
log_info "Starting Test Harness Server..."
node test-harness/server.js > test-harness.log 2>&1 &
HARNESS_PID=$!
sleep 2

# Helper: Run Simple Backend (Embedded/Headless)
run_simple_backend() {
    BACKEND_NAME=$1
    CMD_ARG=$2 # 'embedded' or 'headless'
    
    log_info "=== Running Suite: $BACKEND_NAME ==="
    
    for script in test-harness/scripts/*.lemma; do
        script_name=$(basename "$script")
        log_info "TESTING [$BACKEND_NAME]: $script_name"
        
        # Capture output
        LOG_FILE="${BACKEND_NAME}_${script_name}.log"
        
        # Run oryn with timeout
        if timeout 30s $LCOPE_BIN --file "$script" $CMD_ARG > "$LOG_FILE" 2>&1; then
            log_pass "[$BACKEND_NAME] $script_name passed"
            echo "| $BACKEND_NAME | $script_name | ✅ PASS | |" >> "$RESULTS_FILE"
        else
            EXIT_CODE=$?
            if [ $EXIT_CODE -eq 124 ]; then
                log_fail "[$BACKEND_NAME] $script_name TIMEOUT"
                echo "| $BACKEND_NAME | $script_name | ⏱️ TIMEOUT | > 30s |" >> "$RESULTS_FILE"
            else
                log_fail "[$BACKEND_NAME] $script_name failed"
                echo "| $BACKEND_NAME | $script_name | ❌ FAIL | Exit $EXIT_CODE |" >> "$RESULTS_FILE"
            fi
        fi
    done
}

# Helper: Run Remote Backend (Complex Setup)
run_remote_backend() {
    BACKEND_NAME="Remote"
    log_info "=== Running Suite: $BACKEND_NAME ==="
    
    if [ -z "$CHROME_BIN" ]; then
        CHROME_BIN="/usr/lib64/chromium-browser/chromium-browser"
    fi

    # Patch Extension
    rm -rf "$EXT_PATCH_DIR"
    mkdir -p "$EXT_PATCH_DIR"
    cp -r "$ENV_EXT_DIR/"* "$EXT_PATCH_DIR/"
    sed -i "s|ws://127.0.0.1:9001|ws://127.0.0.1:$PORT|g" "$EXT_PATCH_DIR/background.js"

    for script in test-harness/scripts/*.lemma; do
        script_name=$(basename "$script")
        log_info "TESTING [$BACKEND_NAME]: $script_name"
        
        # Reset Ports
        fuser -k $PORT/tcp 2>/dev/null || true
        fuser -k $DEBUG_PORT/tcp 2>/dev/null || true
        sleep 1
        
        # Reset User Data
        rm -rf "$USER_DATA_DIR"
        mkdir -p "$USER_DATA_DIR"

        # Start Server
        $LCOPE_BIN --file "$script" remote --port $PORT > "remote_server_${script_name}.log" 2>&1 &
        LSCOPE_PID=$!
        
        # Wait for Port
        SERVER_UP=false
        for i in {1..30}; do
            if nc -z 127.0.0.1 $PORT; then
                SERVER_UP=true
                break
            fi
            sleep 0.5
        done
        
        if [ "$SERVER_UP" = "false" ]; then
             log_fail "Remote Server failed to start for $script_name"
             kill "$LSCOPE_PID" 2>/dev/null || true
             echo "| $BACKEND_NAME | $script_name | ❌ FAIL | Server init timeout |" >> "$RESULTS_FILE"
             continue
        fi

        # Launch Chrome
        CMD="$CHROME_BIN \
            --headless=new \
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
            
        $CMD > "remote_chrome_${script_name}.log" 2>&1 &
        CHROME_PID=$!

        # Wait for completion (with timeout)
        if timeout 30s wait "$LSCOPE_PID"; then
             EXIT_CODE=$?
        else
             EXIT_CODE=124
        fi
        
        if [ $EXIT_CODE -eq 0 ]; then
            log_pass "[$BACKEND_NAME] $script_name passed"
            echo "| $BACKEND_NAME | $script_name | ✅ PASS | |" >> "$RESULTS_FILE"
        elif [ $EXIT_CODE -eq 124 ]; then
            log_fail "[$BACKEND_NAME] $script_name TIMEOUT"
            echo "| $BACKEND_NAME | $script_name | ⏱️ TIMEOUT | > 30s |" >> "$RESULTS_FILE"
            kill "$LSCOPE_PID" 2>/dev/null || true
        else
            log_fail "[$BACKEND_NAME] $script_name failed"
            echo "| $BACKEND_NAME | $script_name | ❌ FAIL | Exit Code $EXIT_CODE |" >> "$RESULTS_FILE"
        fi
        
        # Cleanup Chrome
        if [ -n "$CHROME_PID" ]; then
            kill "$CHROME_PID" 2>/dev/null || true
            wait "$CHROME_PID" 2>/dev/null || true
        fi
    done
}

# --- EXECUTION ---

# 1. Embedded
# Note: Embedded tests are known to be flaky/broken in some envs. 
# We run them but valididation might show failures.
run_simple_backend "Embedded" "embedded"

# 2. Headless
run_simple_backend "Headless" "headless"

# 3. Remote
run_remote_backend

log_info "All suites completed. Check $RESULTS_FILE for details."
cat "$RESULTS_FILE"
