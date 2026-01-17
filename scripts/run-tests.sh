#!/bin/bash
# scripts/run-tests.sh
# Runs all Rust tests: unit, integration, E2E, and use-case tests
# Automatically starts/stops the test harness for E2E tests

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

log_info()  { echo -e "${BLUE}[INFO]${NC} $1"; }
log_pass()  { echo -e "${GREEN}[PASS]${NC} $1"; }
log_fail()  { echo -e "${RED}[FAIL]${NC} $1"; }
log_warn()  { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_step()  { echo -e "${CYAN}[STEP]${NC} $1"; }

HARNESS_PID=""
HARNESS_LOG="$PROJECT_ROOT/test-harness.log"

# Cleanup function
cleanup() {
    if [ -n "$HARNESS_PID" ]; then
        log_info "Stopping test harness (PID: $HARNESS_PID)..."
        kill "$HARNESS_PID" 2>/dev/null || true
        wait "$HARNESS_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

# Start test harness
start_harness() {
    log_step "Starting test harness server..."

    cd "$PROJECT_ROOT/test-harness"

    # Check if node_modules exists
    if [ ! -d "node_modules" ]; then
        log_info "Installing test harness dependencies..."
        npm install --silent
    fi

    # Start server
    node server.js > "$HARNESS_LOG" 2>&1 &
    HARNESS_PID=$!

    # Wait for server to be ready
    log_info "Waiting for test harness to start..."
    for i in {1..30}; do
        if curl -s http://localhost:3000/ping > /dev/null 2>&1; then
            log_pass "Test harness is running on http://localhost:3000"
            return 0
        fi
        sleep 0.5
    done

    log_fail "Test harness failed to start within 15 seconds"
    cat "$HARNESS_LOG"
    return 1
}

# Print header
print_header() {
    echo ""
    echo -e "${CYAN}╔════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${CYAN}║              ORYN TEST SUITE RUNNER                            ║${NC}"
    echo -e "${CYAN}╚════════════════════════════════════════════════════════════════╝${NC}"
    echo ""
}

# Print summary
print_summary() {
    local status=$1
    echo ""
    echo -e "${CYAN}════════════════════════════════════════════════════════════════${NC}"
    if [ "$status" -eq 0 ]; then
        echo -e "${GREEN}  ALL TESTS PASSED${NC}"
    else
        echo -e "${RED}  TESTS FAILED${NC}"
    fi
    echo -e "${CYAN}════════════════════════════════════════════════════════════════${NC}"
    echo ""
}

# Main
main() {
    print_header
    cd "$PROJECT_ROOT"

    # 1. Format check
    log_step "Checking code formatting..."
    if cargo fmt --all -- --check 2>/dev/null; then
        log_pass "Code is properly formatted"
    else
        log_warn "Code is not formatted. Running cargo fmt..."
        cargo fmt --all
        log_pass "Code formatted"
    fi
    echo ""

    # 2. Clippy lint
    log_step "Running clippy linter..."
    if cargo clippy --workspace --quiet 2>&1 | grep -E "^error" > /dev/null; then
        log_fail "Clippy found errors"
        cargo clippy --workspace
        exit 1
    else
        log_pass "Clippy passed (no errors)"
    fi
    echo ""

    # 3. Start test harness (needed for E2E tests)
    start_harness
    echo ""

    # 4. Run all tests
    log_step "Running all tests..."
    echo ""

    echo -e "${CYAN}─── Unit & Integration Tests ───${NC}"
    if cargo test --workspace 2>&1 | tee /tmp/test-output.txt; then
        TEST_STATUS=0
    else
        TEST_STATUS=1
    fi

    # 5. Run weston tests if weston is available
    if command -v weston &> /dev/null; then
        echo ""
        echo -e "${CYAN}─── Weston Headless Tests ───${NC}"
        log_info "Weston found, running weston-headless tests..."
        if cargo test -p oryn-e weston -- --ignored 2>&1 | tee -a /tmp/test-output.txt; then
            log_pass "Weston tests passed"
        else
            log_warn "Weston tests failed (non-fatal)"
        fi
    else
        log_info "Weston not available, skipping weston-headless tests"
    fi

    # Parse results
    echo ""
    echo -e "${CYAN}─── Test Summary ───${NC}"
    grep -E "^test result:" /tmp/test-output.txt | while read line; do
        echo "  $line"
    done

    # Count totals
    PASSED=$(grep -oP '\d+ passed' /tmp/test-output.txt | awk '{sum += $1} END {print sum}')
    FAILED=$(grep -oP '\d+ failed' /tmp/test-output.txt | awk '{sum += $1} END {print sum}')
    IGNORED=$(grep -oP '\d+ ignored' /tmp/test-output.txt | awk '{sum += $1} END {print sum}')

    echo ""
    echo -e "  ${GREEN}Passed:${NC}  ${PASSED:-0}"
    echo -e "  ${RED}Failed:${NC}  ${FAILED:-0}"
    echo -e "  ${YELLOW}Ignored:${NC} ${IGNORED:-0}"

    print_summary $TEST_STATUS

    # Cleanup temp file
    rm -f /tmp/test-output.txt

    return $TEST_STATUS
}

# Run main
main "$@"
