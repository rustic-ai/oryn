# CLI Reference

Current reference for the unified `oryn` CLI.

## Synopsis

```bash
oryn [--file <PATH>] <mode>
```

Modes:

- `headless`
- `embedded`
- `remote`

## Global Options

| Option | Description |
|--------|-------------|
| `--file <PATH>` | Run commands from a script file instead of interactive REPL |
| `--help`, `-h` | Show help |
| `--version`, `-V` | Show version |

## Modes

### headless

```bash
oryn headless [--visible]
```

Run Oryn with Chromium via CDP.

| Option | Description |
|--------|-------------|
| `--visible` | Launch browser with a visible window (not headless) |

Examples:

```bash
oryn headless
oryn headless --visible
```

### embedded

```bash
oryn embedded [--driver-url <URL>]
```

Run Oryn with WebDriver/COG backend.

| Option | Description |
|--------|-------------|
| `--driver-url <URL>` | Connect to an external WebDriver endpoint |

Examples:

```bash
oryn embedded
oryn embedded --driver-url http://localhost:9515
```

### remote

```bash
oryn remote [--port <PORT>]
```

Run Oryn as a WebSocket server for the remote browser extension (`extension/`).

| Option | Default | Description |
|--------|---------|-------------|
| `--port <PORT>` | `9001` | WebSocket port |

Examples:

```bash
oryn remote
oryn remote --port 9010
```

!!! note
    Current server binding is `127.0.0.1` with configurable port only.

## Environment Variables

| Variable | Description |
|----------|-------------|
| `RUST_LOG` | Rust log level |
| `CHROME_BIN` | Custom Chromium/Chrome executable path for headless mode |
| `ORYN_USER_DATA_DIR` | Reuse a specific Chromium profile directory |
| `ORYN_ENABLE_NETWORK_LOG` | Enable network logging when set to `1/true/yes/on` |

## REPL and Script Mode

### Interactive REPL

```bash
oryn headless
```

### File Execution

```bash
oryn --file scripts/example.oil headless
```

`--file` executes non-empty, non-comment lines from the script.

## Common Examples

### Run a script in headless mode

```bash
oryn --file test-harness/scripts/01_static.oil headless
```

### Remote assistance session

```bash
# Terminal 1
oryn remote --port 9001

# Browser
# Load extension/ and connect to localhost:9001
```

## Related References

- [Intent Commands](../reference/intent-commands.md)
- [Backend Modes](../concepts/backend-modes.md)
- [Troubleshooting](../guides/troubleshooting.md)
