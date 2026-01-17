# Oryn User Guide

Oryn is a semantic browser automation system for AI agents. This guide covers how to install and use the unified `oryn` CLI.

## Installation

### Prerequisites
- **Rust Toolchain**: [Install Rust](https://rustup.rs/) (1.70+ recommended).
- **Headless Mode**: Chromium browser installed (`/usr/bin/google-chrome` or similar).
- **Embedded Mode**: WPE WebKit/COG installed (Linux/IoT only).
- **Remote Mode**: Browser Extension loaded in your browser (Chrome/Firefox).

### Building the CLI

Clone the repository and build the unified binary:

```bash
git clone https://github.com/dragonscale/oryn.git
cd oryn
cargo build --release -p oryn
```

The binary will be available at `target/release/oryn`.

## Usage

The `oryn` CLI provides a unified interface for all backends.

```bash
oryn <MODE> [OPTIONS]
```

### Modes

#### 1. Headless Mode (`headless`)
Best for cloud automation, scraping, and CI/CD. Uses a local Chromium instance via CDP.

```bash
oryn headless
```

Once launched, you will enter the **Intent REPL**.

#### 2. Embedded Mode (`embedded`)
Best for IoT/Edge devices running WPE WebKit. Connects to a WebDriver instance (COG).

```bash
# Default URL: http://localhost:8080
oryn embedded --driver-url http://localhost:8080
```

#### 3. Remote Mode (`remote`)
Best for debugging or human-in-the-loop workflows. Connects to the Oryn Browser Extension.

1.  Start the remote server:
    ```bash
    # Default port: 9001
    oryn remote --port 9001
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
RUST_LOG=info oryn headless
# or for more detail
RUST_LOG=debug oryn headless
```

## Architecture

- **oryn**: Unified CLI wrapper.
- **oryn-core**: Protocol, Parser, and Translator logic.
- **oryn-h**: Headless backend (Chromium/CDP).
- **oryn-e**: Embedded backend (WebDriver).
- **oryn-r**: Remote backend (WebSocket Server).
- **oryn-scanner**: Universal JavaScript payload.
