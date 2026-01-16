#!/bin/bash
# Quick smoke test for pre-built Lemmascope Docker images
# Usage: ./smoke-test.sh <image-name>
#
# Example:
#   ./smoke-test.sh lscope-e:alpine
#   ./smoke-test.sh lscope-h:ubuntu

set -e

IMAGE=${1:-"lscope-e:alpine"}

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info() { echo -e "${YELLOW}[INFO]${NC} $1"; }
log_pass() { echo -e "${GREEN}[PASS]${NC} $1"; }
log_fail() { echo -e "${RED}[FAIL]${NC} $1"; exit 1; }

echo "=============================================="
echo "Smoke Test: $IMAGE"
echo "=============================================="

# Detect image type
if echo "$IMAGE" | grep -q "lscope-h"; then
    BINARY="lscope-h"
    BROWSER="chromium"
else
    BINARY="lscope-e"
    BROWSER="wpe"
fi

log_info "Binary: $BINARY, Browser: $BROWSER"

# Test 1: Image exists
log_info "Checking image exists..."
if docker image inspect "$IMAGE" >/dev/null 2>&1; then
    log_pass "Image exists"
else
    log_fail "Image not found: $IMAGE"
fi

# Test 2: Binary runs
log_info "Testing binary..."
if docker run --rm "$IMAGE" "$BINARY" --help >/dev/null 2>&1; then
    log_pass "Binary runs"
else
    log_fail "Binary failed to run"
fi

# Test 3: Browser dependencies
log_info "Checking browser dependencies..."
if [ "$BROWSER" = "wpe" ]; then
    if docker run --rm "$IMAGE" which WPEWebDriver >/dev/null 2>&1; then
        log_pass "WPEWebDriver found"
    else
        log_fail "WPEWebDriver not found"
    fi

    if docker run --rm "$IMAGE" which cog >/dev/null 2>&1; then
        log_pass "COG found"
    else
        log_fail "COG not found"
    fi
else
    if docker run --rm "$IMAGE" sh -c "which chromium-browser || which chromium" >/dev/null 2>&1; then
        log_pass "Chromium found"
    else
        log_fail "Chromium not found"
    fi
fi

# Test 4: Security - non-root user
log_info "Checking security..."
USER=$(docker run --rm "$IMAGE" whoami)
if [ "$USER" = "lscope" ]; then
    log_pass "Running as non-root user"
else
    log_fail "Running as root (insecure)"
fi

# Test 5: Navigation test (optional, requires --nav flag)
if [ "${2:-}" = "--nav" ]; then
    log_info "Running navigation test..."

    RESULT=$(docker run --rm \
        --shm-size=256m \
        --security-opt seccomp=unconfined \
        -e COG_PLATFORM_NAME=headless \
        -e XDG_RUNTIME_DIR=/run/user/1000 \
        "$IMAGE" \
        timeout 30 "$BINARY" --url "https://example.com" --command "observe" 2>&1) || true

    if echo "$RESULT" | grep -qi "example\|domain\|elements"; then
        log_pass "Navigation succeeded"
    else
        log_fail "Navigation failed"
    fi
fi

echo ""
echo -e "${GREEN}All smoke tests passed!${NC}"
