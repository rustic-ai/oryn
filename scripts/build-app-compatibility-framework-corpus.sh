#!/usr/bin/env bash
set -euo pipefail
exec python3 "$(dirname "$0")/build_app_compatibility_framework_corpus.py" "$@"
