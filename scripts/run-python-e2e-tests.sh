#!/bin/bash
# scripts/run-python-e2e-tests.sh
# Python E2E test runner - runs .oil scripts via oryn-python client
#
# This script:
#   1. Builds the Rust oryn binary
#   2. Starts the test harness (Node.js server)
#   3. Builds the Python E2E Docker image
#   4. Runs pytest inside Docker against the test harness
#   5. Cleans up
#
# Usage:
#   ./scripts/run-python-e2e-tests.sh              # Run all tests
#   ./scripts/run-python-e2e-tests.sh --quick      # Skip rebuild, run tests only
#   ./scripts/run-python-e2e-tests.sh --no-docker  # Run locally (requires oryn in PATH)

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
NC='\033[0m'
BOLD='\033[1m'

# Get script directory and project root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
RESULTS_DIR="$PROJECT_ROOT/e2e-results/python"

# Logging
log_info()    { echo -e "${BLUE}[INFO]${NC} $1"; }
log_pass()    { echo -e "${GREEN}[PASS]${NC} $1"; }
log_fail()    { echo -e "${RED}[FAIL]${NC} $1"; }
log_warn()    { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_step()    { echo -e "${CYAN}[STEP]${NC} $1"; }

# Configuration
HARNESS_PORT=3000
HARNESS_PID=""
HARNESS_LOG="$PROJECT_ROOT/test-harness-python.log"
DOCKER_IMAGE="oryn-python-e2e:latest"

# Cleanup function
cleanup() {
    log_info "Cleaning up..."

    # Stop test harness
    if [ -n "$HARNESS_PID" ]; then
        kill "$HARNESS_PID" 2>/dev/null || true
        wait "$HARNESS_PID" 2>/dev/null || true
    fi

    # Stop any running test containers
    docker rm -f oryn-python-e2e-runner 2>/dev/null || true

    log_info "Cleanup complete"
}

trap cleanup EXIT

# Print header
print_header() {
    echo ""
    echo -e "${CYAN}╔════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${CYAN}║           ORYN PYTHON E2E TEST SUITE                           ║${NC}"
    echo -e "${CYAN}╚════════════════════════════════════════════════════════════════╝${NC}"
    echo ""
}

# Start test harness (local Node.js server)
start_harness() {
    log_step "Starting test harness..."

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
        if curl -s "http://localhost:${HARNESS_PORT}/ping" > /dev/null 2>&1; then
            log_pass "Test harness running on port $HARNESS_PORT"
            cd "$PROJECT_ROOT"
            return 0
        fi
        sleep 0.5
    done

    log_fail "Test harness failed to start within 15 seconds"
    cat "$HARNESS_LOG"
    cd "$PROJECT_ROOT"
    return 1
}

# Build Rust binaries
build_rust() {
    log_step "Building Rust binaries..."
    cd "$PROJECT_ROOT"

    if [ "$SKIP_BUILD" != "true" ]; then
        cargo build --release --package oryn --quiet
        log_pass "Rust build complete"
    else
        log_info "Skipping Rust build (SKIP_BUILD=true)"
    fi

    # Verify binary exists
    if [ ! -x "$PROJECT_ROOT/target/release/oryn" ]; then
        log_fail "oryn binary not found at target/release/oryn"
        return 1
    fi
}

# Build Docker image
build_docker_image() {
    log_step "Building Python E2E Docker image..."
    cd "$PROJECT_ROOT"

    # Generate poetry.lock if it doesn't exist
    if [ ! -f "oryn-python/poetry.lock" ]; then
        log_info "Generating poetry.lock..."
        cd oryn-python && poetry lock && cd ..
    fi

    docker build \
        -t "$DOCKER_IMAGE" \
        -f docker/python-e2e/Dockerfile \
        . \
        --quiet

    log_pass "Built $DOCKER_IMAGE"
}

# Run tests in Docker
run_tests_docker() {
    log_step "Running Python E2E tests in Docker..."

    mkdir -p "$RESULTS_DIR"
    local results_file="$RESULTS_DIR/results-$(date +%Y%m%d-%H%M%S).log"

    echo "=== Python E2E Test Results ===" > "$results_file"
    echo "Started: $(date)" >> "$results_file"
    echo "" >> "$results_file"

    # Run Docker container with host network to access test harness
    # Use --shm-size for Chrome, --security-opt label=disable for SELinux compatibility
    local exit_code=0
    set -o pipefail
    docker run --rm \
        --name oryn-python-e2e-runner \
        --network host \
        --shm-size=2gb \
        --security-opt label=disable \
        -e "TEST_HARNESS_URL=http://localhost:$HARNESS_PORT" \
        -e "ORYN_MODE=headless" \
        "$DOCKER_IMAGE" \
        poetry run pytest tests/e2e/test_oil_scripts.py -v --tb=short 2>&1 | tee -a "$results_file" || exit_code=$?
    set +o pipefail

    echo "" >> "$results_file"
    echo "Finished: $(date)" >> "$results_file"

    if [ $exit_code -eq 0 ]; then
        log_pass "All Python E2E tests passed"
    else
        log_fail "Some Python E2E tests failed (exit code: $exit_code)"
    fi

    echo ""
    echo "Results saved to: $results_file"

    return $exit_code
}

# Run tests locally (no Docker)
run_tests_local() {
    log_step "Running Python E2E tests locally..."

    cd "$PROJECT_ROOT/oryn-python"

    mkdir -p "$RESULTS_DIR"
    local results_file="$RESULTS_DIR/results-local-$(date +%Y%m%d-%H%M%S).log"

    echo "=== Python E2E Test Results (Local) ===" > "$results_file"
    echo "Started: $(date)" >> "$results_file"
    echo "" >> "$results_file"

    local exit_code=0
    TEST_HARNESS_URL="http://localhost:$HARNESS_PORT" \
    poetry run pytest tests/e2e/test_oil_scripts.py -v --tb=short 2>&1 | tee -a "$results_file" || exit_code=$?

    echo "" >> "$results_file"
    echo "Finished: $(date)" >> "$results_file"

    cd "$PROJECT_ROOT"

    if [ $exit_code -eq 0 ]; then
        log_pass "All Python E2E tests passed"
    else
        log_fail "Some Python E2E tests failed (exit code: $exit_code)"
    fi

    echo ""
    echo "Results saved to: $results_file"

    return $exit_code
}

# Print summary
print_summary() {
    local exit_code=$1

    echo ""
    echo -e "${CYAN}════════════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}                 PYTHON E2E TEST SUMMARY${NC}"
    echo -e "${CYAN}════════════════════════════════════════════════════════════════${NC}"
    echo ""

    if [ $exit_code -eq 0 ]; then
        echo -e "  ${GREEN}✓ ALL PYTHON E2E TESTS PASSED${NC}"
    else
        echo -e "  ${RED}✗ SOME PYTHON E2E TESTS FAILED${NC}"
    fi

    echo ""
    echo -e "${CYAN}════════════════════════════════════════════════════════════════${NC}"
    echo ""
    echo "Detailed logs available in: $RESULTS_DIR/"
    echo ""
}

# Show help
show_help() {
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --quick         Skip Rust build, assume binaries exist"
    echo "  --no-docker     Run tests locally instead of in Docker"
    echo "  --help, -h      Show this help"
    echo ""
    echo "Environment Variables:"
    echo "  SKIP_BUILD=true   Skip Rust build step"
    echo ""
    echo "Examples:"
    echo "  $0                    # Full run: build + Docker tests"
    echo "  $0 --quick            # Skip build, run Docker tests"
    echo "  $0 --no-docker        # Run tests locally (requires oryn in PATH)"
    echo ""
}

# Main
main() {
    print_header

    # Parse arguments
    local use_docker=true
    local quick_mode=false

    for arg in "$@"; do
        case "$arg" in
            --quick)
                quick_mode=true
                export SKIP_BUILD=true
                ;;
            --no-docker)
                use_docker=false
                ;;
            --help|-h)
                show_help
                exit 0
                ;;
            *)
                log_warn "Unknown argument: $arg"
                ;;
        esac
    done

    local exit_code=0

    # Build Rust binaries (unless skipped)
    if [ "$use_docker" = true ]; then
        build_rust || exit 1
    fi

    # Start test harness
    start_harness || exit 1

    echo ""

    # Run tests
    if [ "$use_docker" = true ]; then
        build_docker_image || exit 1
        run_tests_docker || exit_code=$?
    else
        run_tests_local || exit_code=$?
    fi

    print_summary $exit_code

    return $exit_code
}

# Run main
main "$@"
