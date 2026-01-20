# CLI Reference

Complete reference for the `oryn` command-line interface.

## Synopsis

```bash
oryn <MODE> [OPTIONS]
```

## Modes

### Headless Mode

```bash
oryn headless [OPTIONS]
```

Run Oryn with a headless Chromium browser. Best for cloud automation, CI/CD, and web scraping.

**Options:**

| Option | Default | Description |
|--------|---------|-------------|
| `--chrome-path <PATH>` | auto-detect | Path to Chrome/Chromium executable |
| `--user-data-dir <DIR>` | temp | Chrome user data directory |
| `--headless` | true | Run in headless mode (use `--no-headless` for visible browser) |
| `--disable-gpu` | false | Disable GPU acceleration |
| `--window-size <WxH>` | 1920x1080 | Browser window size |

**Example:**

```bash
oryn headless --no-headless --window-size 1280x720
```

### Embedded Mode

```bash
oryn embedded [OPTIONS]
```

Run Oryn with WPE WebKit via WebDriver. Best for IoT devices, containers, and edge computing.

**Options:**

| Option | Default | Description |
|--------|---------|-------------|
| `--driver-url <URL>` | http://localhost:8080 | WebDriver server URL |

**Example:**

```bash
oryn embedded --driver-url http://localhost:9515
```

### Remote Mode

```bash
oryn remote [OPTIONS]
```

Run Oryn as a WebSocket server that connects to a browser extension. Best for user assistance, debugging, and authenticated sessions.

**Options:**

| Option | Default | Description |
|--------|---------|-------------|
| `--port <PORT>` | 9001 | WebSocket server port |
| `--host <HOST>` | 127.0.0.1 | Host to bind to |

**Example:**

```bash
oryn remote --port 9001 --host 0.0.0.0
```

## Global Options

| Option | Description |
|--------|-------------|
| `--help`, `-h` | Show help message |
| `--version`, `-V` | Show version information |
| `--config <FILE>` | Path to configuration file |
| `--log-level <LEVEL>` | Log level: error, warn, info, debug, trace |

## Environment Variables

| Variable | Description |
|----------|-------------|
| `RUST_LOG` | Set log level (e.g., `RUST_LOG=debug`) |
| `CHROME_PATH` | Path to Chrome executable |
| `ORYN_CONFIG` | Path to configuration file |

## REPL Commands

Once Oryn is running, you enter the Intent Language REPL. See the [Intent Commands Reference](../reference/intent-commands.md) for complete command documentation.

### Quick Reference

| Command | Description | Example |
|---------|-------------|---------|
| `goto <url>` | Navigate to URL | `goto google.com` |
| `observe` / `scan` | List interactive elements | `observe` |
| `click <target>` | Click an element | `click 5` or `click "Login"` |
| `type <target> <text>` | Type into input | `type 1 "hello"` |
| `scroll [direction] [amount]` | Scroll the page | `scroll down 500` |
| `wait <condition>` | Wait for condition | `wait visible "Success"` |
| `login <user> <pass>` | Execute login intent | `login "me@example.com" "pass"` |
| `search <query>` | Execute search intent | `search "rust programming"` |
| `accept_cookies` | Dismiss cookie banner | `accept_cookies` |
| `exit` | Exit Oryn | `exit` |

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Invalid arguments |
| 3 | Browser launch failed |
| 4 | Connection failed |

## Configuration File

Oryn can be configured via a YAML file:

```yaml
# ~/.oryn/config.yaml

intent_engine:
  default_timeout: 30s
  step_timeout: 10s
  max_retries: 3

logging:
  log_actions: true
  redact_sensitive: true

packs:
  auto_load: true
  pack_paths:
    - ~/.oryn/packs
```

See [Configuration Reference](../reference/configuration.md) for all options.

## Examples

### Basic Web Scraping

```bash
oryn headless << 'EOF'
goto example.com
observe
click "More information..."
observe
extract links
exit
EOF
```

### Login Automation

```bash
oryn headless << 'EOF'
goto myapp.com/login
login "user@example.com" "password123"
observe
exit
EOF
```

### CI/CD Testing

```bash
#!/bin/bash
set -e

# Start Oryn and run test script
oryn headless < tests/e2e/login-test.oil

echo "Test passed!"
```

### Remote Assistance

```bash
# Terminal 1: Start server
oryn remote --port 9001

# User: Opens browser with extension and connects

# Terminal 1: Now control user's browser
> goto myapp.com
> observe
> click "Help"
```
