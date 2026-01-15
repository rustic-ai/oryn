# How to Setup and Use Remote Mode (lscope-r)

**Remote Mode** allows Lemmascope to control a standard Chrome-based browser via a lightweight WebSocket extension. This is ideal for visual debugging and development.

## Prerequisites

1.  **Rust Toolchain**: Ensure you have a working Rust environment (`cargo`).
2.  **Browser**: Google Chrome, Microsoft Edge, or any Chromium-based browser supporting Manifest V3 extensions.

## Step 1: Build the Project

Build the `lscope-r` binary using Cargo:

```bash
cargo build -p lscope-r
```

## Step 2: Load the Browser Extension

1.  Open your browser and navigate to the Extensions management page:
    *   Chrome: `chrome://extensions`
    *   Edge: `edge://extensions`
2.  Enable **Developer mode** (toggle usually in the top right).
3.  Click **"Load unpacked"**.
4.  Select the `extension/` directory from the root of the Lemmascope project:
    ```
    /path/to/lemmascope/extension/
    ```
5.  You should see "Lemmascope Agent" in your extensions list.

**Note**: The extension will automatically attempt to connect to `ws://localhost:9001`. It will retry every 3 seconds if the connection fails.

## Step 3: Run the Remote Backend

Run the `lscope-r` server. By default, it listens on port `9001`.

```bash
cargo run -p lscope-r
# Or with a custom port:
# cargo run -p lscope-r -- --port 9001
```

You should see output indicating the server has started:
```
Starting Lemmascope Remote Backend on port 9001...
Please connect the browser extension to ws://localhost:9001
Backend launched. Enter commands (e.g., 'goto google.com', 'scan').
> 
```

## Step 4: Verify Connection

1.  Check your browser extension icon or the background page devtools (Inspect views: Service Worker).
2.  Look for "Connected to Lemmascope Server" in the console.
3.  The server terminal logs might also show a new peer connection.

## Step 5: Execute Commands

You can now type [Intent Language](../SPEC-INTENT-LANGUAGE.md) commands directly into the `lscope-r` CLI.

### Navigation
Navigate the active tab to a URL:
```
> goto https://www.google.com
```

### Observation
Scan the page and list interactive elements:
```
> scan
```

### Interaction
Click buttons or links (ensure you have scanned first or know the ID/Target):
```
> click "Search"
> type "Rust lang" into "Search Input"
```

## Troubleshooting

- **"WebSocket error" in Extension**: Ensure `lscope-r` is running. Check if port 9001 is available.
- **Commands timeout**: Check the specific tab you are interacting with is active. The extension currently targets the `active` tab of the `currentWindow`.
- **Changes in code**: If you modify `extension/` files (js/json), remember to click the **Reload** (refresh icon) on the extension card in `chrome://extensions`.
