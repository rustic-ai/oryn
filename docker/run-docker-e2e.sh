#!/bin/bash
# run-docker-e2e.sh: Run Oryn Docker images against the test harness
# This script builds the images and executes the .lemma test suite.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
SCRIPTS_DIR="$PROJECT_DIR/test-harness/scripts"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
NC='\033[0m'

log_info() { echo -e "${YELLOW}[INFO]${NC} $1"; }
log_pass() { echo -e "${GREEN}[PASS]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

cleanup() {
    log_info "Cleaning up containers..."
    docker-compose -f "$SCRIPT_DIR/docker-compose.test-env.yml" down >/dev/null 2>&1
}

trap cleanup EXIT

# 1. Build and start test harness
log_info "Starting test harness..."
docker-compose -p oryn-test -f "$SCRIPT_DIR/docker-compose.test-env.yml" up -d --build
log_info "Waiting for test harness to be healthy..."
until [ "$(docker inspect -f '{{.State.Health.Status}}' test-harness)" == "healthy" ]; do
    sleep 1
done
HARNESS_IP=$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' test-harness)
log_pass "Test harness is up at $HARNESS_IP"

# 2. Define images to test
IMAGES=(
    "oryn-h:latest"
    "oryn-h:headless"
    "oryn-e:debian"
    "oryn-e:latest"
    "oryn-e:weston"
)

# 3. Execution loop
failed_images=0
REPORT_FILE="$SCRIPT_DIR/test-report.md"

# Initialize Report
cat <<EOF > "$REPORT_FILE"
# Oryn Docker E2E Test Report
Generated on: $(date)

## Summary
| Image | Status | Passed | Failed |
|-------|--------|--------|--------|
EOF

NETWORK_NAME="oryn-test_oryn-test-net"

for IMAGE in "${IMAGES[@]}"; do
    echo -e "\n${BOLD}================================================================${NC}"
    echo -e "${BOLD}Testing Image: $IMAGE${NC}"
    echo -e "${BOLD}================================================================${NC}"

    # Verify build
    log_info "Building $IMAGE..."
    DOCKERFILE=""
    case "$IMAGE" in
        "oryn-h:latest")   DOCKERFILE="Dockerfile.oryn-h" ;;
        "oryn-h:headless") DOCKERFILE="Dockerfile.oryn-h.headless" ;;
        "oryn-e:latest")   DOCKERFILE="Dockerfile.oryn-e" ;;
        "oryn-e:debian")   DOCKERFILE="Dockerfile.oryn-e.debian" ;;
        "oryn-e:weston")   DOCKERFILE="Dockerfile.oryn-e.weston" ;;
    esac

    docker build -t "$IMAGE" -f "$SCRIPT_DIR/$DOCKERFILE" "$PROJECT_DIR" >/dev/null

    # Decide binary
    BINARY="oryn-e"
    if [[ "$IMAGE" == oryn-h* ]]; then BINARY="oryn-h"; fi

    # Run each script
    script_count=0
    script_failed=0
    image_results=""
    
    for SCRIPT_FILE in "$SCRIPTS_DIR"/*.lemma; do
        SCRIPT_NAME=$(basename "$SCRIPT_FILE")
        log_info "Running $SCRIPT_NAME against $IMAGE..."
        
        if timeout 60 docker run --rm --user root --privileged \
            --network "$NETWORK_NAME" \
            --add-host localhost:"$HARNESS_IP" \
            --shm-size=1g \
            --security-opt seccomp=unconfined \
            -v "$SCRIPTS_DIR":/scripts:z \
            -e COG_PLATFORM_NAME=headless \
            "$IMAGE" \
            "$BINARY" --file "/scripts/$SCRIPT_NAME" > /tmp/oryn_out.log 2>&1; then
            log_pass "$SCRIPT_NAME passed"
            image_results="$image_results\n| $SCRIPT_NAME | ✅ PASS |"
            ((script_count++)) || true
        else
            log_error "$SCRIPT_NAME failed"
            tail -n 20 /tmp/oryn_out.log
            image_results="$image_results\n| $SCRIPT_NAME | ❌ FAIL |"
            ((script_failed++)) || true
        fi
    done

    # Update Report Summary
    IMAGE_STATUS="✅ PASS"
    if [ "$script_failed" -gt 0 ]; then
        IMAGE_STATUS="❌ FAIL"
        ((failed_images++)) || true
    fi
    
    # Append to main summary table (using temporary file to avoid complex sed)
    echo "| $IMAGE | $IMAGE_STATUS | $script_count | $script_failed |" >> "$REPORT_FILE"
    
    # Add detailed results for this image
    echo -e "\n### Details: $IMAGE" >> "$REPORT_FILE"
    echo -e "| Script | Result |" >> "$REPORT_FILE"
    echo -e "|--------|--------|" >> "$REPORT_FILE"
    echo -e "$image_results" >> "$REPORT_FILE"
done

echo -e "\n=============================================="
if [ "$failed_images" -eq 0 ]; then
    log_pass "ALL DOCKER IMAGES PASSED ALL TESTS"
    log_info "Report generated at: $REPORT_FILE"
    exit 0
else
    log_error "$failed_images images failed their test suites."
    log_info "Report generated at: $REPORT_FILE"
    exit 1
fi
