#!/bin/bash
# Test script for Oryn Docker images
# Usage: ./test-images.sh [alpine|debian|ubuntu|all]

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

passed=0
failed=0

log_info() { echo -e "${YELLOW}[INFO]${NC} $1"; }
log_pass() { echo -e "${GREEN}[PASS]${NC} $1"; ((passed++)); }
log_fail() { echo -e "${RED}[FAIL]${NC} $1"; ((failed++)); }

# ============================================================================
# Test Functions
# ============================================================================

test_binary_exists() {
    local image=$1
    local binary=$2

    if docker run --rm "$image" which "$binary" >/dev/null 2>&1; then
        log_pass "$image: $binary exists"
        return 0
    else
        log_fail "$image: $binary not found"
        return 1
    fi
}

test_binary_runs() {
    local image=$1
    local binary=$2
    local args=${3:---help}

    if docker run --rm "$image" "$binary" $args >/dev/null 2>&1; then
        log_pass "$image: $binary runs successfully"
        return 0
    else
        log_fail "$image: $binary failed to run"
        return 1
    fi
}

test_version_output() {
    local image=$1
    local binary=$2
    local version_flag=${3:---version}

    local output
    if output=$(docker run --rm "$image" "$binary" "$version_flag" 2>&1); then
        log_pass "$image: $binary version: $(echo "$output" | head -1)"
        return 0
    else
        log_fail "$image: $binary version check failed"
        return 1
    fi
}

test_wpe_components() {
    local image=$1

    log_info "Testing WPE components in $image..."

    # Test WPEWebDriver
    test_binary_exists "$image" "WPEWebDriver"

    # Test COG
    test_binary_exists "$image" "cog"

    # Test libraries
    if docker run --rm "$image" sh -c "ls /usr/lib/libwpe* 2>/dev/null | head -1" | grep -q libwpe; then
        log_pass "$image: libwpe library found"
    else
        log_fail "$image: libwpe library not found"
    fi

    if docker run --rm "$image" sh -c "ls /usr/lib/libWPEBackend* 2>/dev/null | head -1" | grep -q libWPE; then
        log_pass "$image: libWPEBackend library found"
    else
        log_fail "$image: libWPEBackend library not found"
    fi
}

test_chromium_components() {
    local image=$1

    log_info "Testing Chromium components in $image..."

    # Test Chromium binary
    if docker run --rm "$image" sh -c "which chromium-browser || which chromium" >/dev/null 2>&1; then
        log_pass "$image: Chromium binary found"
    else
        log_fail "$image: Chromium binary not found"
    fi

    # Test Chromium version
    local version
    if version=$(docker run --rm "$image" sh -c "chromium-browser --version 2>/dev/null || chromium --version 2>/dev/null" | head -1); then
        log_pass "$image: Chromium version: $version"
    else
        log_fail "$image: Chromium version check failed"
    fi
}

test_headless_navigation() {
    local image=$1
    local binary=$2
    local timeout=${3:-30}

    log_info "Testing headless navigation in $image..."

    # Run a simple navigation test
    local result
    if result=$(docker run --rm \
        --shm-size=256m \
        --security-opt seccomp=unconfined \
        -e COG_PLATFORM_NAME=headless \
        -e XDG_RUNTIME_DIR=/run/user/1000 \
        "$image" \
        timeout "$timeout" "$binary" --url "https://example.com" --command "observe" 2>&1); then

        if echo "$result" | grep -qi "example\|domain\|elements\|scan"; then
            log_pass "$image: Navigation test succeeded"
            return 0
        else
            log_fail "$image: Navigation returned unexpected output"
            echo "$result" | head -10
            return 1
        fi
    else
        log_fail "$image: Navigation test failed or timed out"
        echo "$result" | tail -5
        return 1
    fi
}

test_environment() {
    local image=$1

    log_info "Testing environment in $image..."

    # Test user
    local user
    user=$(docker run --rm "$image" whoami)
    if [ "$user" = "oryn" ]; then
        log_pass "$image: Running as non-root user 'oryn'"
    else
        log_fail "$image: Expected user 'oryn', got '$user'"
    fi

    # Test XDG_RUNTIME_DIR
    if docker run --rm "$image" sh -c '[ -d "$XDG_RUNTIME_DIR" ]'; then
        log_pass "$image: XDG_RUNTIME_DIR exists"
    else
        log_fail "$image: XDG_RUNTIME_DIR missing"
    fi

    # Test CA certificates
    if docker run --rm "$image" sh -c "ls /etc/ssl/certs/*.pem 2>/dev/null | head -1" | grep -q pem; then
        log_pass "$image: CA certificates installed"
    else
        log_fail "$image: CA certificates missing"
    fi
}

# ============================================================================
# Image-specific test suites
# ============================================================================

test_alpine_image() {
    local image="oryn-e:alpine"

    echo ""
    echo "=============================================="
    echo "Testing Alpine image: $image"
    echo "=============================================="

    # Build image
    log_info "Building $image..."
    if docker build -f "$SCRIPT_DIR/Dockerfile.oryn-e" -t "$image" "$PROJECT_DIR" >/dev/null 2>&1; then
        log_pass "Built $image"
    else
        log_fail "Failed to build $image"
        return 1
    fi

    # Check image size
    local size
    size=$(docker images "$image" --format "{{.Size}}")
    log_info "$image size: $size"

    # Run tests
    test_binary_exists "$image" "oryn-e"
    test_binary_runs "$image" "oryn-e" "--help"
    test_wpe_components "$image"
    test_environment "$image"

    # Optional: navigation test (slower)
    if [ "${RUN_NAVIGATION_TESTS:-false}" = "true" ]; then
        test_headless_navigation "$image" "oryn-e"
    fi
}

test_debian_image() {
    local image="oryn-e:debian"

    echo ""
    echo "=============================================="
    echo "Testing Debian image: $image"
    echo "=============================================="

    # Build image
    log_info "Building $image..."
    if docker build -f "$SCRIPT_DIR/Dockerfile.oryn-e.debian" -t "$image" "$PROJECT_DIR" >/dev/null 2>&1; then
        log_pass "Built $image"
    else
        log_fail "Failed to build $image"
        return 1
    fi

    # Check image size
    local size
    size=$(docker images "$image" --format "{{.Size}}")
    log_info "$image size: $size"

    # Run tests
    test_binary_exists "$image" "oryn-e"
    test_binary_runs "$image" "oryn-e" "--help"
    test_wpe_components "$image"
    test_environment "$image"

    # Optional: navigation test
    if [ "${RUN_NAVIGATION_TESTS:-false}" = "true" ]; then
        test_headless_navigation "$image" "oryn-e"
    fi
}

test_ubuntu_image() {
    local image="oryn-h:ubuntu"

    echo ""
    echo "=============================================="
    echo "Testing Ubuntu image: $image"
    echo "=============================================="

    # Build image
    log_info "Building $image..."
    if docker build -f "$SCRIPT_DIR/Dockerfile.oryn-h" -t "$image" "$PROJECT_DIR" >/dev/null 2>&1; then
        log_pass "Built $image"
    else
        log_fail "Failed to build $image"
        return 1
    fi

    # Check image size
    local size
    size=$(docker images "$image" --format "{{.Size}}")
    log_info "$image size: $size"

    # Run tests
    test_binary_exists "$image" "oryn-h"
    test_binary_runs "$image" "oryn-h" "--help"
    test_chromium_components "$image"
    test_environment "$image"

    # Optional: navigation test
    if [ "${RUN_NAVIGATION_TESTS:-false}" = "true" ]; then
        test_headless_navigation "$image" "oryn-h"
    fi
}

# ============================================================================
# Main
# ============================================================================

print_usage() {
    echo "Usage: $0 [OPTIONS] [alpine|debian|ubuntu|all]"
    echo ""
    echo "Options:"
    echo "  --nav    Run navigation tests (slower, requires network)"
    echo "  --help   Show this help message"
    echo ""
    echo "Examples:"
    echo "  $0 alpine          # Test Alpine image only"
    echo "  $0 all             # Test all images"
    echo "  $0 --nav all       # Test all images with navigation tests"
}

main() {
    local target="all"

    # Parse arguments
    while [[ $# -gt 0 ]]; do
        case $1 in
            --nav)
                export RUN_NAVIGATION_TESTS=true
                shift
                ;;
            --help|-h)
                print_usage
                exit 0
                ;;
            alpine|debian|ubuntu|all)
                target=$1
                shift
                ;;
            *)
                echo "Unknown argument: $1"
                print_usage
                exit 1
                ;;
        esac
    done

    echo "=============================================="
    echo "Oryn Docker Image Tests"
    echo "=============================================="
    echo "Project: $PROJECT_DIR"
    echo "Target: $target"
    echo "Navigation tests: ${RUN_NAVIGATION_TESTS:-false}"

    case $target in
        alpine)
            test_alpine_image
            ;;
        debian)
            test_debian_image
            ;;
        ubuntu)
            test_ubuntu_image
            ;;
        all)
            test_alpine_image
            test_debian_image
            test_ubuntu_image
            ;;
    esac

    echo ""
    echo "=============================================="
    echo "Test Summary"
    echo "=============================================="
    echo -e "${GREEN}Passed: $passed${NC}"
    echo -e "${RED}Failed: $failed${NC}"

    if [ $failed -gt 0 ]; then
        exit 1
    fi
}

main "$@"
