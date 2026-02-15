# Backend Modes

Oryn provides three runtime modes with one command language.

## Mode Comparison

| Aspect | `oryn-e` (Embedded) | `oryn-h` (Headless) | `oryn-r` (Remote) |
|--------|----------------------|---------------------|-------------------|
| Browser engine | WPE WebKit | Chromium | User browser + extension |
| Transport | WebDriver | CDP | WebSocket |
| Best for | constrained devices | CI/cloud automation | assisted browsing sessions |
| CLI entry | `oryn embedded` | `oryn headless` | `oryn remote` |

## `oryn-h`: Headless

```bash
oryn headless
```

Visible mode:

```bash
oryn headless --visible
```

Environment variables used by headless backend:

- `CHROME_BIN`
- `ORYN_USER_DATA_DIR`
- `ORYN_ENABLE_NETWORK_LOG`

## `oryn-e`: Embedded

```bash
oryn embedded
```

Using an external WebDriver endpoint:

```bash
oryn embedded --driver-url http://localhost:8080
```

## `oryn-r`: Remote

```bash
oryn remote --port 9001
```

Remote mode uses the browser extension in `extension/`.

Setup flow:

1. Load `extension/` as an unpacked extension.
2. Start Oryn remote server: `oryn remote --port 9001`.
3. Connect extension to `localhost:9001`.

!!! note
    Current remote server binding is `127.0.0.1` with configurable port.

## Choosing a Mode

- Use `headless` for stable automation and CI.
- Use `embedded` for smaller environments.
- Use `remote` when actions must happen in a user's active browser session.
