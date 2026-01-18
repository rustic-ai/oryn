#!/bin/bash
set -e

# Default values
ORYN_SERVER_URL="${ORYN_SERVER_URL:-ws://localhost:9001}"
START_URL="${START_URL:-http://localhost:3000/shared/diag.html}"

echo "Chrome Extension E2E Runner"
echo "  ORYN_SERVER_URL: $ORYN_SERVER_URL"
echo "  START_URL: $START_URL"

# Patch extension background.js to use configured server URL
if [ -f /app/extension/background.js ]; then
    echo "Patching extension to connect to $ORYN_SERVER_URL..."
    # Extract port from URL or use the full URL
    if [[ "$ORYN_SERVER_URL" =~ :([0-9]+)$ ]]; then
        PORT="${BASH_REMATCH[1]}"
        # Replace default port with configured port
        sed -i "s/:9001/:$PORT/g" /app/extension/background.js
    fi
fi

echo "Launching Chrome with extension..."
xvfb-run --server-args="-screen 0 1280x1024x24" \
  google-chrome-stable \
  --no-sandbox \
  --disable-gpu \
  --disable-dev-shm-usage \
  --disable-web-security \
  --no-first-run \
  --no-default-browser-check \
  --enable-logging \
  --v=1 \
  --user-data-dir=/tmp/chrome-data \
  --load-extension=/app/extension \
  --remote-allow-origins=* \
  --remote-debugging-port=9002 \
  "$START_URL"
