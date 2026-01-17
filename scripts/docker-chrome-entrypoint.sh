#!/bin/bash
set -e

# Start Chrome with xvfb-run
echo "Launch Chrome inside Docker (xvfb-run)..."
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
  "http://localhost:3000/shared/diag.html"
