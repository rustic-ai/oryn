# Lemmascope User Guide

Lemmascope is a semantic browser automation system for AI agents. This guide covers how to install and use the unified `lscope` CLI.

## Installation

### Prerequisites
- **Rust Toolchain**: [Install Rust](https://rustup.rs/) (1.70+ recommended).
- **Headless Mode**: Chromium browser installed (`/usr/bin/google-chrome` or similar).
- **Embedded Mode**: WPE WebKit/COG installed (Linux/IoT only).
- **Remote Mode**: Browser Extension loaded in your browser (Chrome/Firefox).

### Building the CLI

Clone the repository and build the unified binary:

```bash
git clone https://github.com/dragonscale/lemmascope.git
cd lemmascope
cargo build --release -p lscope
```

The binary will be available at `target/release/lscope`.

## Usage

The `lscope` CLI provides a unified interface for all backends.

```bash
lscope <MODE> [OPTIONS]
```

### Modes

#### 1. Headless Mode (`headless`)
Best for cloud automation, scraping, and CI/CD. Uses a local Chromium instance via CDP.

```bash
lscope headless
```

Once launched, you will enter the **Intent REPL**.

#### 2. Embedded Mode (`embedded`)
Best for IoT/Edge devices running WPE WebKit. Connects to a WebDriver instance (COG).

```bash
# Default URL: http://localhost:8080
lscope embedded --driver-url http://localhost:8080
```

#### 3. Remote Mode (`remote`)
Best for debugging or human-in-the-loop workflows. Connects to the Lemmascope Browser Extension.

1.  Start the remote server:
    ```bash
    # Default port: 9001
    lscope remote --port 9001
    ```
2.  Open your browser with the extension installed.
3.  Click the extension icon to connect.

## Intent Language REPL

Once a backend is connected, you can issue **Intent Commands**.

### Basic Commands

| Command            | Description                                 | Example           |
| :----------------- | :------------------------------------------ | :---------------- |
| `goto <url>`       | Navigate to a URL                           | `goto google.com` |
| `observe` / `scan` | Scan the page and list interactive elements | `scan`            |
| `click <id>`       | Click an element by ID                      | `click 5`         |
| `type <id> "text"` | Type text into an input                     | `type 2 "hello"`  |
| `scroll`           | Scroll the page                             | `scroll`          |
| `exit`             | Quit the session                            | `exit`            |

### Semantic Targeting
You can target elements by ID (fastest) or by their semantic attributes.

```text
click "Sign Up"
type "Email" "user@example.com"
click "Submit" inside "Login Form"
```

### Advanced Commands

- **Wait**: `wait visible "Success"`
- **Storage**: `storage clear`
- **Modifiers**: `click "Remove" near "Item 1"`

## Debugging

Enable verbose logging by setting the `RUST_LOG` environment variable:

```bash
RUST_LOG=info lscope headless
# or for more detail
RUST_LOG=debug lscope headless
```

## Architecture

- **lscope**: Unified CLI wrapper.
- **lscope-core**: Protocol, Parser, and Translator logic.
- **lscope-h**: Headless backend (Chromium/CDP).
- **lscope-e**: Embedded backend (WebDriver).
- **lscope-r**: Remote backend (WebSocket Server).
- **lscope-scanner**: Universal JavaScript payload.
