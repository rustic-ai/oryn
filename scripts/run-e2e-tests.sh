#!/bin/bash
# scripts/run-e2e-tests.sh
# Comprehensive E2E test runner that executes all oil scripts against all backend variants
#
# Backend variants tested:
#   - oryn-h       : Chromium headless (Docker)
#   - oryn-e-debian: WPE WebKit on Debian (Docker)
#   - oryn-e-weston: WPE with Weston compositor (Docker)
#   - oryn-r       : Remote mode - Chromium (Docker) with browser extension
#
# Usage:
#   ./scripts/run-e2e-tests.sh              # Run all variants
#   ./scripts/run-e2e-tests.sh --quick      # Run only oryn-h (fastest)
#   ./scripts/run-e2e-tests.sh oryn-h       # Run specific variant
#   ./scripts/run-e2e-tests.sh oryn-r       # Remote mode with Chromium in Docker

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
DOCKER_DIR="$PROJECT_ROOT/docker"
RESULTS_DIR="$PROJECT_ROOT/e2e-results"

# Logging
log_info()    { echo -e "${BLUE}[INFO]${NC} $1"; }
log_pass()    { echo -e "${GREEN}[PASS]${NC} $1"; }
log_fail()    { echo -e "${RED}[FAIL]${NC} $1"; }
log_warn()    { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_step()    { echo -e "${CYAN}[STEP]${NC} $1"; }
log_variant() { echo -e "${MAGENTA}[${1}]${NC} $2"; }

# Configuration
HARNESS_PORT=3000
REMOTE_PORT=9001
CHROME_DEBUG_PORT=9002

# Test harness
HARNESS_PID=""
HARNESS_LOG="$PROJECT_ROOT/test-harness.log"

# Track results
declare -A VARIANT_RESULTS
TOTAL_PASSED=0
TOTAL_FAILED=0

# Cleanup function
cleanup() {
    log_info "Cleaning up..."

    # Stop test harness
    if [ -n "$HARNESS_PID" ]; then
        kill "$HARNESS_PID" 2>/dev/null || true
        wait "$HARNESS_PID" 2>/dev/null || true
    fi

    # Stop any running Docker containers
    docker rm -f e2e-test-harness e2e-backend 2>/dev/null || true

    # Clean up temp directories
    rm -rf /tmp/oryn_ext_* /tmp/oryn_chrome_data_* /tmp/oryn-wpe-* 2>/dev/null || true

    # Free ports
    fuser -k $REMOTE_PORT/tcp 2>/dev/null || true

    log_info "Cleanup complete"
}

# Kill stale WPE/MiniBrowser processes that might block new sessions
cleanup_wpe_processes() {
    log_info "Cleaning up stale WPE processes..."

    # Kill stale Docker containers running WPE
    docker ps -q --filter "ancestor=oryn-e:debian" --filter "ancestor=oryn-e:weston" --filter "ancestor=oryn-e:latest" 2>/dev/null | xargs -r docker rm -f 2>/dev/null || true

    # Kill local WPE processes
    pkill -9 -f "MiniBrowser" 2>/dev/null || true
    pkill -9 -f "WPEWebDriver" 2>/dev/null || true
    pkill -9 -f "WPENetworkProcess" 2>/dev/null || true
    pkill -9 -f "cog" 2>/dev/null || true

    # Clean up temp directories
    rm -rf /tmp/oryn-wpe-* 2>/dev/null || true

    sleep 1
}

trap cleanup EXIT

# Print header
print_header() {
    echo ""
    echo -e "${CYAN}╔════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${CYAN}║           ORYN E2E TEST SUITE - ALL VARIANTS                   ║${NC}"
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
        log_info "Skipping build (SKIP_BUILD=true)"
    fi
}

# Build Docker images for a variant
build_docker_image() {
    local variant=$1

    log_info "Building Docker image for $variant..."
    cd "$PROJECT_ROOT"

    case "$variant" in
        oryn-h)
            docker build -t oryn-h:latest -f docker/Dockerfile.oryn-h . --quiet
            ;;
        oryn-e-debian)
            docker build -t oryn-e:debian -f docker/Dockerfile.oryn-e.debian . --quiet
            ;;
        oryn-e-weston)
            docker build -t oryn-e:weston -f docker/Dockerfile.oryn-e.weston . --quiet
            ;;
    esac

    log_pass "Built $variant"
}

# ============================================================================
# Test oryn-h (Chromium headless in Docker)
# ============================================================================
test_oryn_h() {
    log_step "Testing oryn-h (Chromium headless)..."

    build_docker_image "oryn-h"

    local scripts_dir="$PROJECT_ROOT/test-harness/scripts"
    local passed=0
    local failed=0
    local results_file="$RESULTS_DIR/oryn-h.log"

    mkdir -p "$RESULTS_DIR"
    echo "=== E2E Test Results for oryn-h ===" > "$results_file"
    echo "Started: $(date)" >> "$results_file"

    for script in $(ls -1 "$scripts_dir"/*.oil 2>/dev/null | sort); do
        local script_name=$(basename "$script")
        log_variant "oryn-h" "Running: $script_name"

        echo "--- Script: $script_name ---" >> "$results_file"

        # Run in Docker with host network to access test harness
        if docker run --rm \
            --network host \
            --shm-size=2gb \
            -e "RUST_LOG=info" \
            -v "$scripts_dir:/scripts:ro" \
            oryn-h:latest \
            /usr/local/bin/oryn-h --file "/scripts/$script_name" >> "$results_file" 2>&1; then
            log_pass "  $script_name"
            ((passed++))
            echo "Result: PASS" >> "$results_file"
        else
            log_fail "  $script_name"
            ((failed++))
            echo "Result: FAIL" >> "$results_file"
        fi
    done

    VARIANT_RESULTS["oryn-h"]="$passed passed, $failed failed"
    ((TOTAL_PASSED += passed))
    ((TOTAL_FAILED += failed))

    [ $failed -eq 0 ]
}

# ============================================================================
# Test oryn-e-debian (WPE WebKit in Docker)
# ============================================================================
test_oryn_e_debian() {
    log_step "Testing oryn-e-debian (WPE WebKit)..."

    build_docker_image "oryn-e-debian"

    local scripts_dir="$PROJECT_ROOT/test-harness/scripts"
    local passed=0
    local failed=0
    local results_file="$RESULTS_DIR/oryn-e-debian.log"

    mkdir -p "$RESULTS_DIR"
    echo "=== E2E Test Results for oryn-e-debian ===" > "$results_file"
    echo "Started: $(date)" >> "$results_file"

    for script in $(ls -1 "$scripts_dir"/*.oil 2>/dev/null | sort); do
        local script_name=$(basename "$script")
        log_variant "oryn-e-debian" "Running: $script_name"

        echo "--- Script: $script_name ---" >> "$results_file"

        # Note: Don't override XDG_RUNTIME_DIR - Dockerfile sets it correctly
        if docker run --rm \
            --network host \
            --shm-size=1gb \
            --security-opt seccomp:unconfined \
            --cap-add SYS_ADMIN \
            -e "COG_PLATFORM_NAME=headless" \
            -v "$scripts_dir:/scripts:ro" \
            oryn-e:debian \
            /usr/local/bin/oryn-e --file "/scripts/$script_name" >> "$results_file" 2>&1; then
            log_pass "  $script_name"
            ((passed++))
            echo "Result: PASS" >> "$results_file"
        else
            log_fail "  $script_name"
            ((failed++))
            echo "Result: FAIL" >> "$results_file"
        fi
    done

    VARIANT_RESULTS["oryn-e-debian"]="$passed passed, $failed failed"
    ((TOTAL_PASSED += passed))
    ((TOTAL_FAILED += failed))

    [ $failed -eq 0 ]
}

# ============================================================================
# Test oryn-e-weston (WPE + Weston in Docker)
# ============================================================================
test_oryn_e_weston() {
    log_step "Testing oryn-e-weston (WPE + Weston)..."

    build_docker_image "oryn-e-weston"

    local scripts_dir="$PROJECT_ROOT/test-harness/scripts"
    local passed=0
    local failed=0
    local results_file="$RESULTS_DIR/oryn-e-weston.log"

    mkdir -p "$RESULTS_DIR"
    echo "=== E2E Test Results for oryn-e-weston ===" > "$results_file"
    echo "Started: $(date)" >> "$results_file"

    for script in $(ls -1 "$scripts_dir"/*.oil 2>/dev/null | sort); do
        local script_name=$(basename "$script")
        log_variant "oryn-e-weston" "Running: $script_name"

        echo "--- Script: $script_name ---" >> "$results_file"

        # Weston needs privileged mode for bubblewrap sandbox
        # The entrypoint script handles starting weston and setting WAYLAND_DISPLAY
        if docker run --rm \
            --network host \
            --privileged \
            --shm-size=1gb \
            -v "$scripts_dir:/scripts:ro" \
            oryn-e:weston \
            /usr/local/bin/oryn-e --file "/scripts/$script_name" >> "$results_file" 2>&1; then
            log_pass "  $script_name"
            ((passed++))
            echo "Result: PASS" >> "$results_file"
        else
            log_fail "  $script_name"
            ((failed++))
            echo "Result: FAIL" >> "$results_file"
        fi
    done

    VARIANT_RESULTS["oryn-e-weston"]="$passed passed, $failed failed"
    ((TOTAL_PASSED += passed))
    ((TOTAL_FAILED += failed))

    [ $failed -eq 0 ]
}

# ============================================================================
# Test oryn-r (Remote mode - Chromium in Docker + Extension)
# ============================================================================
test_oryn_r() {
    log_step "Testing oryn-r (Remote mode with Chromium + Extension)..."

    # Ensure we have oryn binary
    local ORYN_BIN="$PROJECT_ROOT/target/release/oryn"
    if [ ! -x "$ORYN_BIN" ]; then
        log_fail "oryn binary not found at $ORYN_BIN"
        return 1
    fi

    # Check Docker is available
    if ! command -v docker &> /dev/null; then
        log_fail "Docker is required for oryn-r tests (uses zenika/alpine-chrome)"
        return 1
    fi

    local scripts_dir="$PROJECT_ROOT/test-harness/scripts"
    local ext_dir="$PROJECT_ROOT/extension"
    local passed=0
    local failed=0
    local results_file="$RESULTS_DIR/oryn-r.log"

    mkdir -p "$RESULTS_DIR"
    echo "=== E2E Test Results for oryn-r ===" > "$results_file"
    echo "Started: $(date)" >> "$results_file"
    echo "Browser: zenika/alpine-chrome (Chromium)" >> "$results_file"

    # Create temp directories for patched extension and Chrome profile
    local ext_patch_dir="/tmp/oryn_ext_$$"
    local user_data_dir="/tmp/oryn_chrome_data_$$"

    # Patch extension to use our port
    log_info "Preparing extension..."
    rm -rf "$ext_patch_dir"
    mkdir -p "$ext_patch_dir"
    cp -r "$ext_dir/"* "$ext_patch_dir/"
    # We still patch background.js as a fallback, but config.json is the primary way now
    sed -i "s|ws://127.0.0.1:9001|ws://127.0.0.1:$REMOTE_PORT|g" "$ext_patch_dir/background.js"

    # Create config.json for auto-connect per EXTENSION_TESTING.md
    echo "{\"autoConnect\": true, \"websocketUrl\": \"ws://127.0.0.1:$REMOTE_PORT\"}" > "$ext_patch_dir/config.json"

    # Make extension accessible to Docker
    chmod -R 755 "$ext_patch_dir"

    # Pull the chrome image if needed
    log_info "Ensuring zenika/alpine-chrome image is available..."
    docker pull zenika/alpine-chrome:with-node --quiet || true

    for script in $(ls -1 "$scripts_dir"/*.oil 2>/dev/null | sort); do
        local script_name=$(basename "$script")
        log_variant "oryn-r" "Running: $script_name"

        echo "--- Script: $script_name ---" >> "$results_file"

        # Clean up from previous iteration
        fuser -k $REMOTE_PORT/tcp 2>/dev/null || true
        docker rm -f oryn-chrome-$$ 2>/dev/null || true
        rm -rf "$user_data_dir"
        mkdir -p "$user_data_dir"
        chmod -R 777 "$user_data_dir"
        sleep 1

        # Start oryn server with the script (runs WebSocket server + executes script)
        log_info "  Starting oryn server..."
        RUST_LOG=info "$ORYN_BIN" --file "$script" remote --port $REMOTE_PORT > "$RESULTS_DIR/oryn_$script_name.log" 2>&1 &
        local ORYN_PID=$!

        # Wait for server to be listening
        for i in {1..30}; do
            if nc -z 127.0.0.1 $REMOTE_PORT 2>/dev/null; then
                break
            fi
            sleep 0.5
        done

        # Launch Chromium in Docker with extension
        log_info "  Launching Chromium (Docker)..."
        docker run --rm -d \
            --name oryn-chrome-$$ \
            --network host \
            -v "$ext_patch_dir:$ext_patch_dir:z" \
            -v "$user_data_dir:$user_data_dir:z" \
            zenika/alpine-chrome:with-node \
            chromium-browser \
            --headless=new \
            --no-sandbox \
            --disable-gpu \
            --disable-dev-shm-usage \
            --disable-first-run-ui \
            --no-first-run \
            --no-default-browser-check \
            --user-data-dir="$user_data_dir" \
            --load-extension="$ext_patch_dir" \
            --remote-debugging-port=$CHROME_DEBUG_PORT \
            "http://localhost:$HARNESS_PORT/" > "$RESULTS_DIR/chrome_$script_name.log" 2>&1

        # Wait for oryn to complete (it exits after script finishes)
        log_info "  Waiting for script to complete..."
        if wait "$ORYN_PID"; then
            log_pass "  $script_name"
            ((passed++))
            echo "Result: PASS" >> "$results_file"
        else
            log_fail "  $script_name"
            ((failed++))
            echo "Result: FAIL" >> "$results_file"
            echo "--- oryn output ---" >> "$results_file"
            cat "$RESULTS_DIR/oryn_$script_name.log" >> "$results_file" 2>/dev/null || true
            echo "--- chrome output ---" >> "$results_file"
            cat "$RESULTS_DIR/chrome_$script_name.log" >> "$results_file" 2>/dev/null || true
            echo "--- docker logs ---" >> "$results_file"
            docker logs oryn-chrome-$$ >> "$results_file" 2>&1 || true
        fi

        # Stop Chromium container
        docker rm -f oryn-chrome-$$ 2>/dev/null || true
        sleep 1
    done

    # Cleanup temp dirs
    rm -rf "$ext_patch_dir" "$user_data_dir"

    VARIANT_RESULTS["oryn-r"]="$passed passed, $failed failed"
    ((TOTAL_PASSED += passed))
    ((TOTAL_FAILED += failed))

    [ $failed -eq 0 ]
}

# Print final summary
print_summary() {
    echo ""
    echo -e "${CYAN}════════════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}                      E2E TEST SUMMARY${NC}"
    echo -e "${CYAN}════════════════════════════════════════════════════════════════${NC}"
    echo ""

    for variant in "${!VARIANT_RESULTS[@]}"; do
        local result="${VARIANT_RESULTS[$variant]}"
        if [[ "$result" == *"0 failed"* ]]; then
            echo -e "  ${GREEN}✓${NC} ${BOLD}$variant${NC}: $result"
        else
            echo -e "  ${RED}✗${NC} ${BOLD}$variant${NC}: $result"
        fi
    done

    echo ""
    echo -e "${CYAN}────────────────────────────────────────────────────────────────${NC}"
    echo -e "  ${BOLD}Total:${NC} $TOTAL_PASSED passed, $TOTAL_FAILED failed"
    echo ""

    if [ $TOTAL_FAILED -eq 0 ]; then
        echo -e "${GREEN}  ✓ ALL E2E TESTS PASSED${NC}"
    else
        echo -e "${RED}  ✗ SOME E2E TESTS FAILED${NC}"
    fi

    echo -e "${CYAN}════════════════════════════════════════════════════════════════${NC}"
    echo ""
    echo "Detailed logs available in: $RESULTS_DIR/"
    echo ""
}

# Main
main() {
    print_header

    # Parse arguments
    local variants=()
    local quick_mode=false

    for arg in "$@"; do
        case "$arg" in
            --quick)
                quick_mode=true
                ;;
            --help|-h)
                echo "Usage: $0 [OPTIONS] [VARIANTS...]"
                echo ""
                echo "Options:"
                echo "  --quick         Run only oryn-h (fastest variant)"
                echo "  --help          Show this help"
                echo ""
                echo "Environment Variables:"
                echo "  SKIP_BUILD=true   Skip Rust build step"
                echo ""
                echo "Variants:"
                echo "  oryn-h          Chromium headless (Docker)"
                echo "  oryn-e-debian   WPE WebKit on Debian (Docker)"
                echo "  oryn-e-weston   WPE with Weston compositor (Docker)"
                echo "  oryn-r          Remote mode - Chromium (Docker) with extension"
                echo ""
                echo "If no variants specified, all variants are tested."
                echo ""
                echo "Examples:"
                echo "  $0                    # Run all variants"
                echo "  $0 --quick            # Run only oryn-h"
                echo "  $0 oryn-r             # Run only remote mode"
                echo "  $0 oryn-h oryn-r      # Run specific variants"
                exit 0
                ;;
            oryn-h|oryn-e-debian|oryn-e-weston|oryn-r)
                variants+=("$arg")
                ;;
            *)
                log_warn "Unknown argument: $arg"
                ;;
        esac
    done

    # Default variants
    if [ ${#variants[@]} -eq 0 ]; then
        if [ "$quick_mode" = true ]; then
            variants=("oryn-h")
        else
            variants=("oryn-h" "oryn-e-debian" "oryn-e-weston" "oryn-r")
        fi
    fi

    log_info "Testing variants: ${variants[*]}"
    echo ""

    # Setup
    cleanup_wpe_processes
    build_rust
    start_harness

    echo ""

    # Run tests for each variant
    local exit_code=0

    for variant in "${variants[@]}"; do
        echo ""
        echo -e "${CYAN}────────────────────────────────────────────────────────────────${NC}"

        case "$variant" in
            oryn-h)
                test_oryn_h || exit_code=1
                ;;
            oryn-e-debian)
                test_oryn_e_debian || exit_code=1
                ;;
            oryn-e-weston)
                test_oryn_e_weston || exit_code=1
                ;;
            oryn-r)
                test_oryn_r || exit_code=1
                ;;
        esac
    done

    print_summary

    return $exit_code
}

# Run main
main "$@"
