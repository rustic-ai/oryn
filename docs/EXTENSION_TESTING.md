# Oryn Extension Testing Guide

This guide details how to configure the Oryn Browser Extension for automated testing, specifically focusing on the **Auto-Connect** feature.

## Auto-Connect Feature

For automated tests (e.g., Selenium, Playwright), you often need the extension to connect to the Oryn server (`ws://127.0.0.1:9001`) immediately upon browser startup, without any manual interaction.

### Enabling Auto-Connect

The extension looks for a `config.json` file in its root directory (`extension/config.json`) during startup.

**To Enable:**
1.  Create or ensure `extension/config.json` exists.
2.  Set `"autoConnect": true`.

**Example `config.json`:**
```json
{
    "autoConnect": true,
    "websocketUrl": "ws://127.0.0.1:9001"
}
```

**Behavior:**
-   When the extension loads, it reads this configuration.
-   When any tab finishes loading (status: `complete`), the extension checks this flag.
-   If `true`, it automatically initiates a connection to the specified `websocketUrl`.

### Disabling Auto-Connect

**To Disable (for Production or Manual Testing):**
1.  **Option A (Recommended)**: Delete or rename `extension/config.json`.
    -   If the file is missing, the extension defaults to `autoConnect: false`.
2.  **Option B**: Edit `config.json` and set `"autoConnect": false`.

### Runtime Override

You can override the auto-connect setting at runtime using the Popup UI:
1.  Click the Oryn Extension icon.
2.  Toggle the "Auto-Connect" checkbox.
3.  This user preference overrides the `config.json` default for the current session.

## Testing Workflow

1.  **Build/Prepare**: Copy your test `config.json` (with `autoConnect: true`) into the `extension/` folder.
2.  **Launch**: Start Chrome with the extension loaded (unpacked).
3.  **Run**: Your test script opens a target URL.
4.  **Connect**: The extension detects the page load and connects instantly.
5.  **Execute**: Proceed with your test steps.
