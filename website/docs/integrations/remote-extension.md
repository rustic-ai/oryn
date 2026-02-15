# Remote Extension (`extension/`)

Use `extension/` with `oryn remote` when you want Oryn to operate through a user's live browser session.

## Overview

Remote mode architecture:

1. `oryn remote --port <PORT>` starts local WebSocket server.
2. Browser extension connects to that server.
3. Commands execute in the active browser tab through scanner content scripts.

## Start Oryn Remote Server

```bash
oryn remote --port 9001
```

Current bind host is `127.0.0.1`.

## Load Extension in Chrome

1. Open `chrome://extensions`
2. Enable Developer mode
3. Click Load unpacked
4. Select repository folder: `extension/`

## Connect Extension

- Open extension popup
- Set URL (default is usually `ws://127.0.0.1:9001`)
- Click Connect

Once connected, run commands from Oryn REPL as usual.

## Verify End-to-End

```text
> goto https://example.com
> observe
> click "More information..."
```

## Permissions and Scope

`extension/manifest.json` requests:

- `activeTab`, `scripting`, `tabs`, `storage`, `sidePanel`
- host permissions including `<all_urls>` and websocket/http(s) endpoints

## Common Issues

### Waiting for connection forever

- Ensure `oryn remote --port 9001` is running.
- Ensure popup URL matches (`ws://127.0.0.1:9001`).
- Reload extension after updates.

### Commands no-op on page

- Refresh page to reinject scripts if needed.
- Run `observe` first to refresh element map.

## Related

- [Backend Modes](../concepts/backend-modes.md)
- [WASM Extension](wasm-extension.md)
