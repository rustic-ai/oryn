# Oryn Extension Testing Guide

This guide explains how to configure and test the Oryn browser extension for automated and manual testing, with a focus on the oryn-r (remote mode) auto-connect workflow.

## Auto-connect configuration

The extension checks for a config file at `extension/config.json` on startup.

Example:
```json
{
  "autoConnect": true,
  "websocketUrl": "ws://127.0.0.1:9001"
}
```

### Behavior

- On startup, the extension loads this configuration.
- When a tab finishes loading (status: `complete`), the extension checks:
  - autoConnect is enabled
  - the tab is currently disconnected
- If both are true, it calls `connect(tabId, websocketUrl)`.

### Disabling auto-connect

- Recommended: delete or rename `extension/config.json` (defaults to false).
- Or set `"autoConnect": false` in the config file.

### Runtime override

You can override auto-connect at runtime via the popup UI:
1. Click the extension icon
2. Toggle the Auto-Connect checkbox
3. This preference overrides the config file for the current session

## Local testing script (oryn-r)

A helper script wires everything together for a local browser test.

Prerequisites:
- Chromium or Google Chrome installed
- Oryn built: `cargo build --release --package oryn`
- Test harness deps installed: `cd test-harness && npm install`

Usage:
```bash
# Default script (01_static.oil)
./scripts/test-oryn-r-local.sh

# Specific script
./scripts/test-oryn-r-local.sh test-harness/scripts/02_forms.oil

# Help
./scripts/test-oryn-r-local.sh --help
```

What the script does:
1. Copies the extension to `/tmp/oryn_ext_test` and writes a config.json with autoConnect enabled
2. Starts the test harness on port 3000
3. Starts the oryn server on port 9001
4. Launches Chromium with the extension loaded
5. Waits for auto-connect
6. Executes the provided .oil script
7. Writes logs to `e2e-results/`

## Manual verification

### Prepare extension

```bash
mkdir -p /tmp/oryn_ext_manual
cp -r extension/* /tmp/oryn_ext_manual/
echo '{"autoConnect": true, "websocketUrl": "ws://127.0.0.1:9001"}' > /tmp/oryn_ext_manual/config.json
```

### Start oryn server

```bash
./target/release/oryn --file test-harness/scripts/01_static.oil remote --port 9001
```

### Launch browser

```bash
chromium-browser \
  --load-extension=/tmp/oryn_ext_manual \
  --user-data-dir=/tmp/oryn_chrome_manual \
  --remote-debugging-port=9222 \
  http://localhost:3000/
```

### Verify auto-connect

- Extension icon changes color when connected
- Extension popup console logs show:
  - `[System] Config loaded: autoConnect=true`
  - `[AutoConnect] Triggered for Tab X`
  - `Connected`
- Server logs show:
  - `WebSocket Handshake Successful`
  - `New WebSocket connection: established`

## Troubleshooting

### Extension does not auto-connect

1. Confirm config.json:
   ```bash
   cat /tmp/oryn_ext_manual/config.json
   ```
   Should include `"autoConnect": true`.

2. Check extension storage:
   - Open DevTools
   - Application tab -> Storage -> Extension Storage
   - Verify `autoConnect` and `websocketUrl`

3. Check background service worker:
   - chrome://extensions
   - Find "Oryn Agent"
   - Click "service worker" to view logs

### WebSocket connection fails

1. Verify server is running:
   ```bash
   lsof -ti:9001
   ```

2. Check server logs for handshake errors.

3. Inspect browser DevTools -> Network tab -> WS filter
   - Look for `ws://127.0.0.1:9001`

## Known limitations

### Docker/headless mode

Browser extensions are limited in headless Chromium, especially in Docker:
- Service workers may not start correctly
- Auto-connect may not trigger

For development, use a local browser test (script above).

### CI/CD testing

Options:
1. Skip oryn-r tests in CI (test other variants)
2. Use a browser automation service (e.g., BrowserStack, Sauce Labs)
3. Use Xvfb with full Chrome (not headless)

## Test results location

- Server logs: `e2e-results/oryn_<script>.log`
- Chrome logs: `e2e-results/chrome_<script>.log`
- Test harness log: `e2e-results/test-harness.log`

## Architecture

```
Oryn CLI (server)  <--- WebSocket (9001) --->  Browser extension
        |                                         |
        | sends actions                           | executes in page
        v                                         v
   Web page (localhost:3000)                Content script
```

Auto-connect flow:
```
Page load
  -> chrome.tabs.onUpdated (status=complete)
  -> autoConnect enabled?
  -> tab disconnected?
  -> connect(tabId, websocketUrl)
  -> WebSocket established
```
