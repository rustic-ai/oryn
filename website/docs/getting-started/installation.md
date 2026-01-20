# Installation

This guide covers how to build and install Oryn from source.

## Prerequisites

### Required

- **Rust Toolchain**: Version 1.70 or later
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```

### Mode-Specific Requirements

=== "Headless Mode (oryn-h)"

    - **Chromium Browser**: Google Chrome or Chromium installed
    - **RAM**: ~300MB+ available
    - Works on Linux, macOS, Windows

    ```bash
    # Ubuntu/Debian
    sudo apt install chromium-browser

    # macOS (Chrome is typically pre-installed or download from google.com/chrome)

    # Verify installation
    which chromium-browser || which google-chrome
    ```

=== "Embedded Mode (oryn-e)"

    - **WPE WebKit/COG**: Linux only
    - **RAM**: ~50MB available
    - Best for IoT and edge devices

    ```bash
    # Ubuntu/Debian (requires additional repos for WPE)
    sudo apt install cog libwpewebkit-1.0-dev
    ```

=== "Remote Mode (oryn-r)"

    - **Browser Extension**: Chrome or Firefox
    - **WebSocket**: Network connectivity between CLI and browser
    - Works on any system with a modern browser

## Building from Source

### Clone the Repository

```bash
git clone https://github.com/dragonscale/oryn.git
cd oryn
```

### Build the Unified Binary

```bash
cargo build --release -p oryn
```

The binary will be available at `target/release/oryn`.

### Install (Optional)

```bash
# Copy to a directory in your PATH
sudo cp target/release/oryn /usr/local/bin/

# Or add to PATH
export PATH="$PATH:$(pwd)/target/release"
```

## Verifying Installation

### Check Version

```bash
oryn --version
```

### Test Headless Mode

```bash
oryn headless
# You should see the Oryn REPL prompt
> goto example.com
> observe
> exit
```

### Run Tests

```bash
# Run the full test suite
./scripts/run-tests.sh

# Run E2E tests (requires Docker)
./scripts/run-e2e-tests.sh --quick
```

## Docker Installation

Oryn provides Docker images for each mode:

```bash
# Headless mode
docker pull dragonscale/oryn-h:latest
docker run -it dragonscale/oryn-h

# Embedded mode (Debian)
docker pull dragonscale/oryn-e-debian:latest

# Embedded mode (Weston compositor)
docker run --privileged dragonscale/oryn-e-weston
```

## Browser Extension (Remote Mode)

For remote mode, install the Oryn browser extension:

1. Navigate to `chrome://extensions/` (Chrome) or `about:addons` (Firefox)
2. Enable "Developer mode"
3. Click "Load unpacked" and select the `extension/` directory from the Oryn repo
4. Start the remote server:
   ```bash
   oryn remote --port 9001
   ```
5. Click the Oryn extension icon to connect

## Troubleshooting

### Chromium Not Found

If headless mode fails to find Chrome:

```bash
# Set the Chrome path explicitly
export CHROME_PATH=/path/to/chrome
oryn headless
```

### Build Errors

Ensure you have the latest Rust toolchain:

```bash
rustup update
cargo clean
cargo build --release -p oryn
```

### Permission Issues on Linux

If you see permission errors:

```bash
# Ensure your user can access the browser
sudo usermod -aG video $USER
# Log out and back in
```

## Next Steps

Once installed, proceed to the [Quick Start](quickstart.md) guide to run your first automation.
