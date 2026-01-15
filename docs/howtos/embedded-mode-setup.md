# Embedded Mode Guide (lscope-e)

**Lemmascope Embedded Mode** (`lscope-e`) provides a backend for executing Intent Language commands against embedded browsers like **WPE WebKit** or **Cognition (COG)** via the WebDriver protocol. It is designed for low-resource environments (IoT, Set-top boxes) where a full desktop browser is not available.

## Prerequisites

- **Rust Toolchain**: 1.70+
- **WebDriver-compatible Embedded Browser**:
  - **COG** (WPE WebKit Launcher): Recommended.
  - **WPE WebKit**: With a WebDriver interface enabled.
  - Any standard WebDriver implementation (chromedriver, geckodriver) can theoretically work, but `lscope-e` is optimized for embedded use cases.

## Installation

### 1. Install COG (Recommended)

On Linux (e.g., Ubuntu/Debian):

```bash
sudo apt-get install cog
```

Or build from source via [WPE WebKit](https://wpewebkit.org/).

### 2. Build lscope-e

Build the binary from source:

```bash
cargo build --bin lscope-e --release
# Binary will be at target/release/lscope-e
```

## Setup & Running

### 1. Start the WebDriver

You must start the browser's WebDriver server before launching `lscope-e`.

For **COG**:
```bash
# Launch COG with WebDriver enabled on port 8080 (default)
cog --automation
# OR explicitly:
/usr/libexec/wpe-webkit-driver --port=8080
```

*Note: Determining the exact command differs by distribution. Consult your WPE/COG documentation.*

### 2. Run lscope-e

Connect `lscope-e` to the running WebDriver instance.

```bash
# Default connects to http://localhost:8080
./target/release/lscope-e

# Or specify a custom URL
./target/release/lscope-e --webdriver-url "http://localhost:4444"
```

You will see:
```text
Connecting to WebDriver at http://localhost:8080...
Connected!
Backend launched. Enter commands.
>
```

## Basic Usage

`lscope-e` accepts **Intent Language** commands, just like Headless and Remote modes.

| Command | Usage                        | Description                                             |
| ------- | ---------------------------- | ------------------------------------------------------- |
| `goto`  | `goto "https://example.com"` | Navigate to a URL.                                      |
| `scan`  | `scan`                       | Analyze the current page and list interactive elements. |
| `click` | `click "Login"`              | Click an element by text, ID, or role.                  |
| `type`  | `type "User" "admin"`        | Type text into an input field.                          |
| `wait`  | `wait for "Welcome"`         | Wait for an element to appear.                          |

### Example Session

```bash
> goto "https://wpewebkit.org"
Navigated to https://wpewebkit.org/
> scan
Scanning...
Found 15 elements.
...
> click "Download"
OK Clicked Download
> wait for "Releases"
OK Waited for "Releases" (visible)
```

## Troubleshooting

**"Connection refused"**:
- Ensure the WebDriver server (e.g., `wpe-webkit-driver`) is running.
- Verify the port (default 8080).

**"Session not created"**:
- Ensure the installed WebDriver version matches your browser version.
- `lscope-e` requests capability `browserName: "wpe"` by default. If using another browser, this might fail (future versions will allow custom capabilities).

**No visual output?**:
- WPE is often used full-screen handling the framebuffer directly. If running on a desktop, you might need a windowed backend (e.g., `WPE_BACKEND=fdo`).
