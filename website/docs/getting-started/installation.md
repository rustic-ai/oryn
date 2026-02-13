# Installation

Build and run Oryn from source.

## Prerequisites

- Rust toolchain
- A browser/runtime for your chosen mode:
  - Headless: Chromium/Chrome
  - Embedded: WebDriver + COG/WPE setup
  - Remote: browser extension (`extension/`)

## Build

```bash
git clone https://github.com/dragonscale/oryn.git
cd oryn
cargo build --release -p oryn
```

Binary path:

```text
target/release/oryn
```

## Verify

```bash
./target/release/oryn --version
./target/release/oryn headless
```

In REPL:

```text
> goto example.com
> observe
> exit
```

## Remote Extension Setup (`oryn-r`)

1. Open `chrome://extensions/`
2. Enable Developer mode
3. Load unpacked extension from `extension/`
4. Start server:

```bash
oryn remote --port 9001
```

## Headless Runtime Variables

```bash
export CHROME_BIN=/path/to/chrome
# optional
export ORYN_USER_DATA_DIR=/tmp/oryn-profile
export ORYN_ENABLE_NETWORK_LOG=1
```

## Optional Test Commands

```bash
cargo test --workspace
./scripts/run-tests.sh
./scripts/run-e2e-tests.sh --quick
```
