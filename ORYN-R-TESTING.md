# Testing Oryn-R (Remote Mode) with Local Browser

## Overview

The `oryn-r` mode allows Oryn to control a real browser via a WebSocket connection and browser extension. This document explains how to test it locally with the auto-connect mechanism.

## Auto-Connect Mechanism

The browser extension supports automatic connection when a page loads:

1. **Config File**: Create `extension/config.json` with:
   ```json
   {
     "autoConnect": true,
     "websocketUrl": "ws://127.0.0.1:9001"
   }
   ```

2. **Event Trigger**: When a tab finishes loading (`changeInfo.status === 'complete'`), the extension:
   - Checks if `autoConnect` is enabled in storage
   - Checks if the tab is currently disconnected
   - Automatically calls `connect(tabId, defaultUrl)`

3. **Connection Flow**:
   ```
   Page Load → chrome.tabs.onUpdated event → Auto-connect check → WebSocket.connect()
   ```

## Local Testing Script

### Prerequisites

- Chromium or Google Chrome installed locally
- Oryn built: `cargo build --release --package oryn`
- Test harness dependencies: `cd test-harness && npm install`

### Usage

```bash
# Test with default script (01_static.oil)
./scripts/test-oryn-r-local.sh

# Test with specific script
./scripts/test-oryn-r-local.sh test-harness/scripts/02_forms.oil

# Show help
./scripts/test-oryn-r-local.sh --help
```

### What the Script Does

1. **Prepares Extension**: Copies extension to `/tmp/oryn_ext_test` and creates `config.json` with auto-connect enabled
2. **Starts Test Harness**: Launches the Node.js server on port 3000
3. **Starts Oryn Server**: Launches WebSocket server on port 9001
4. **Launches Browser**: Opens Chromium with the extension loaded (visible window)
5. **Monitors Connection**: Watches for auto-connect to trigger
6. **Executes Script**: Runs the .oil script commands
7. **Reports Results**: Shows connection status and execution logs

**Note**: The browser window will be visible during testing. You can watch:
- The extension icon change color when it connects (green/blue)
- The page being automated in real-time
- DevTools if you want to inspect the extension behavior

### Manual Verification

To manually inspect the auto-connect mechanism:

1. **Prepare Extension**:
   ```bash
   mkdir -p /tmp/oryn_ext_manual
   cp -r extension/* /tmp/oryn_ext_manual/
   echo '{"autoConnect": true, "websocketUrl": "ws://127.0.0.1:9001"}' > /tmp/oryn_ext_manual/config.json
   ```

2. **Start Oryn Server**:
   ```bash
   ./target/release/oryn --file test-harness/scripts/01_static.oil remote --port 9001
   ```

3. **Launch Browser** (in another terminal):
   ```bash
   chromium-browser \
     --load-extension=/tmp/oryn_ext_manual \
     --user-data-dir=/tmp/oryn_chrome_manual \
     --remote-debugging-port=9222 \
     http://localhost:3000/
   ```

4. **Verify Auto-Connect**:
   - Extension icon should change color (green/blue) when connected
   - Check browser DevTools console for extension logs:
     - Right-click extension icon → "Inspect popup" → Console tab
     - Look for: `[System] Config loaded: autoConnect=true`
     - Look for: `[AutoConnect] Triggered for Tab X`
     - Look for: `Connected`

5. **Check Server Logs**:
   - Should see: `WebSocket Handshake Successful`
   - Should see: `New WebSocket connection: established`

## Troubleshooting

### Extension Doesn't Auto-Connect

1. **Check config.json**:
   ```bash
   cat /tmp/oryn_ext_manual/config.json
   ```
   Should show `"autoConnect": true`

2. **Check Extension Storage**:
   - Open browser DevTools (F12)
   - Go to Application → Storage → Extension Storage
   - Verify `autoConnect` and `websocketUrl` are set

3. **Check Background Service Worker**:
   - Go to `chrome://extensions`
   - Find "Oryn Agent" extension
   - Click "service worker" link to see background logs
   - Should see config loading and auto-connect messages

### WebSocket Connection Fails

1. **Verify Server Running**:
   ```bash
   lsof -ti:9001
   ```

2. **Check Server Logs**:
   - Look for "Remote Server listening on: 127.0.0.1:9001"
   - Check for handshake errors

3. **Network Issues**:
   - Open browser DevTools → Network tab → WS filter
   - Should see WebSocket connection to `ws://127.0.0.1:9001`
   - Check connection status and frames

## Known Limitations

### Docker/Headless Mode

Browser extensions have limited functionality in headless Chromium, especially in Docker:

- **Issue**: Service workers may not start properly in headless mode
- **Impact**: Auto-connect doesn't trigger
- **Solution**: Use local browser testing (this script) for development

### CI/CD Testing

For automated testing in CI/CD:

1. **Option 1**: Skip oryn-r tests (test other 4 variants)
2. **Option 2**: Use browser automation services (BrowserStack, Sauce Labs)
3. **Option 3**: Use Xvfb with full Chrome (not headless)

## Test Results Location

- Server logs: `e2e-results/oryn_<script>.log`
- Chrome logs: `e2e-results/chrome_<script>.log`
- Test harness log: `e2e-results/test-harness.log`

## Architecture

```
┌─────────────┐                    ┌──────────────┐
│   Oryn CLI  │ ←─── WebSocket ──→ │   Browser    │
│  (Server)   │    (port 9001)     │  Extension   │
└─────────────┘                    └──────────────┘
       │                                   │
       │ Sends actions                     │ Executes on page
       │ (navigate, scan, etc.)            │ (via content script)
       │                                   │
       │                                   ↓
       │                            ┌──────────────┐
       │                            │   Web Page   │
       └────────────────────────────│ (localhost)  │
                                    └──────────────┘
```

Auto-connect flow:
```
Page Load Event
      ↓
chrome.tabs.onUpdated
      ↓
Check: status === 'complete'
      ↓
Check: autoConnect enabled
      ↓
Check: tab disconnected
      ↓
connect(tabId, websocketUrl)
      ↓
WebSocket established
      ↓
Extension ready for commands
```

