# Headless Mode Guide (oryn-h)

**Oryn Headless Mode** (`oryn-h`) provides a backend for executing Intent Language commands against a headless Chromium instance. It is designed for automated testing, scraping, and verifiable execution without a visible UI.

## Prerequisites

- **Rust Toolchain**: 1.70+
- **Chromium or Google Chrome**: Must be installed and reachable in your system PATH.
  - Linux: `chromium` or `google-chrome`
  - macOS: `Google Chrome.app`

## Installation

Build the binary from source:

```bash
cargo build --bin oryn-h --release
# Binary will be at target/release/oryn-h
```

## Basic Usage

Run the tool to enter the Interactive REPL:

```bash
./target/release/oryn-h
```

You will see:
```text
Starting Oryn Headless Backend...
Backend launched. Enter commands (e.g., 'goto google.com', 'scan').
>
```

### Supported Commands

`oryn-h` accepts **Intent Language** commands.

| Command  | Usage                        | Description                                              |
| -------- | ---------------------------- | -------------------------------------------------------- |
| `goto`   | `goto "https://example.com"` | Navigate to a URL.                                       |
| `scan`   | `scan`                       | Analyze the current page and list interactive elements.  |
| `click`  | `click "Submit"`             | Click an element by text, ID, or role.                   |
| `type`   | `type "Search" "hello"`      | Type text into an input field.                           |
| `scroll` | `scroll down`                | Scroll the page.                                         |
| `wait`   | `wait for "Result"`          | Wait for an element to appear.                           |
| `pdf`    | `pdf "output.pdf"`           | **(Headless Exclusive)** Save the current page as a PDF. |

### Example Session

```bash
> goto "https://google.com"
Navigated to https://www.google.com/
> type "Search" "Rust programming"
OK Typed "Rust programming" into Search
> click "Google Search"
OK Clicked Google Search
> wait for "The Rust Programming Language"
OK Waited for "The Rust Programming Language" (visible)
> pdf "results.pdf"
Generating PDF to results.pdf...
PDF generated successfully.
```

## Features

### 1. Automatic Logging
`oryn-h` automatically captures and logs:
- **Console Messages**: Browser console logs (`console.log`, `console.error`) are printed to stdout.
- **Network Activity**: Fetch/XHR requests are logged via the `tracing` framework.

To see verbose logs, run with:
```bash
RUST_LOG=info ./oryn-h
```

### 2. PDF Generation
Unique to Headless Mode, you can render the full page to a PDF file useful for snapshots/archiving.

### 3. Resilience
The backend manages the browser process lifecycle automatically. If the browser crashes, `oryn-h` will report the error (automatic restart is planned for future versions).

## Configuration

Currently, `oryn-h` uses the default system browser installation.
- To use a specific Chrome binary or user profile, configuration support is planned for future releases.

## Troubleshooting

**"Failed to launch browser"**:
- Ensure Chrome/Chromium is installed.
- On Linux server environments (CI/CD), ensure you have necessary shared libraries installed.

**"Timeout waiting for selector"**:
- The page might be loading slowly. Use `wait for "text"` to explicit wait for content.
