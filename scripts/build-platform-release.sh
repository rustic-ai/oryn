#!/usr/bin/env bash
set -euo pipefail

# Full-plan generation admission, including direct invocation.
python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
cargo_target="${CARGO_TARGET_DIR:-$repo_root/target}"
case "$cargo_target" in /*) ;; *) cargo_target="$repo_root/$cargo_target" ;; esac
export CARGO_TARGET_DIR="$cargo_target"

case "$(uname -s)" in
  Darwin)
    exec "$repo_root/scripts/package-macos-app.sh"
    ;;
  Linux)
    python3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"
    cargo build --release --locked -p oryn --bin oryn \
      --no-default-features --features "native native-v8 core-full webdriver-server"
    binary="$cargo_target/release/oryn"
    if [[ ! -x "$binary" ]]; then
      echo "Linux production binary was not created" >&2
      exit 1
    fi
    if [[ -n "${ORYN_ARTIFACT_ROOT:-}" ]]; then
      artifact_root="$ORYN_ARTIFACT_ROOT"
      case "$artifact_root" in /*) ;; *) artifact_root="$repo_root/$artifact_root" ;; esac
      mkdir -p "$artifact_root"
      if [[ "$artifact_root/oryn" != "$binary" ]]; then
        cp "$binary" "$artifact_root/oryn"
      fi
      binary="$artifact_root/oryn"
    fi
    # Validate the same WebDriver role used by the exec-only driver.
    "$binary" webdriver --help >/dev/null
    jq -n \
      --arg binary "$binary" \
      --arg sha256 "$(sha256sum "$binary" | awk '{print $1}')" \
      --argjson bytes "$(wc -c < "$binary" | tr -d '[:space:]')" \
      '{
        product: $binary,
        packaging: "single_linux_multicall_binary",
        roles: ["parent", "sandboxed_page_worker"],
        sha256: $sha256,
        bytes: $bytes
      }'
    ;;
  *)
    echo "Unsupported production packaging platform: $(uname -s)" >&2
    exit 2
    ;;
esac
