# Recommendation: Handling Alert Dialogs Across Execution Modes

This document provides a technical analysis and specific recommendations for handling blocking JavaScript dialogs (`alert()`, `confirm()`, `prompt()`) in Oryn's three execution modes.

## The Problem
JavaScript dialogs pause the renderer's main thread.
1.  **Headless (`oryn-h`)**: The CDP `Page.evaluate` command hangs until the dialog is closed. Since no user can click it, the test times out.
2.  **Embedded (`oryn-e`)**: The WebDriver `execute_script` command hangs or fails with `UnexpectedAlertOpen`, depending on the driver's configuration.
3.  **Remote (`oryn-r`)**: The user sees a visible dialog. If testing in CI (headless Docker), it hangs indefinitely until the test runner kills the container.

---

## 1. Headless Mode (`oryn-h`)
**Mechanism**: Chrome DevTools Protocol (CDP).

### Recommended Fix: Auto-Dismiss via CDP Events
We cannot use `Page.evaluate` to dismiss the dialog because `Page.evaluate` itself is blocked by the dialog. We must use an asynchronous event listener.

**Implementation Steps**:
1.  In `crates/oryn-h/src/cdp.rs`, inside `CdpClient::launch`:
2.  Subscribe to `Page.javascriptDialogOpening` events.
3.  Spawn a background `tokio` task to listen for these events.
4.  When an event is received, immediately call `Page.handleJavaScriptDialog(accept=true)`.

**Code Snippet**:
```rust
// Subscribe to dialog events
let mut dialog_events = page
    .event_listener::<chromiumoxide::cdp::browser_protocol::page::EventJavascriptDialogOpening>()
    .await?;

let page_clone = page.clone();
tokio::spawn(async move {
    while let Some(event) = dialog_events.next().await {
        tracing::info!("Auto-accepting dialog: {}", event.message);
        if let Err(e) = page_clone.handle_javascript_dialog(true, None).await {
            tracing::error!("Failed to handle dialog: {}", e);
        }
    }
});
```

---

## 2. Embedded Mode (`oryn-e`)
**Mechanism**: WebDriver (WPEWebDriver / COG).

### Recommended Fix: WebDriver Capabilities
WebDriver has a standard capability `unhandledPromptBehavior` that controls how the driver handles unexpected alerts.

**Implementation Steps**:
1.  In `crates/oryn-e/src/cog.rs`, modify the `wpe_capabilities()` function.
2.  Add the `unhandledPromptBehavior` capability set to `"accept"` (or `"dismiss"`).

**Code Snippet**:
```rust
pub fn wpe_capabilities() -> serde_json::Map<String, serde_json::Value> {
    let mut caps = serde_json::Map::new();
    // specific to WPE/standard WebDriver
    caps.insert("unhandledPromptBehavior".to_string(), serde_json::Value::String("accept".to_string()));
    caps
}
```
*Note: Support for this capability depends on the specific WPEWebDriver version. If it is ignored, we may need to use the "Monkey Patch" strategy described below.*

---

## 3. Remote Mode (`oryn-r`)
**Mechanism**: Chrome Extension (Content Script + Background Service Worker).

### Recommended Fix: Monkey Patching `window.alert`
Since `oryn-r` targets real user browsers where we might not have `debugger` permissions (CDP access), and `chrome.tabs` cannot dismiss alerts programmatically, the most robust solution is to prevent the alert from blocking the thread in the first place.

**Why not CDP?**
The extension manifest (`extension/manifest.json`) currently lacks the `"debugger"` permission. Adding it triggers a scary warning to the user ("This extension can read and modify all data...").

**Implementation Steps**:
1.  Create a new content script `extension/suppress_alerts.js`.
2.  Inject it at `document_start` (before any page scripts run).
3.  Override the native functions.

**Code Snippet (`suppress_alerts.js`)**:
```javascript
// Overwrite native dialogs to be non-blocking
window.alert = function(msg) {
    console.log('[Oryn] Suppressed alert:', msg);
};
window.confirm = function(msg) {
    console.log('[Oryn] Suppressed confirm:', msg);
    return true; // Auto-accept
};
window.prompt = function(msg, defaultText) {
    console.log('[Oryn] Suppressed prompt:', msg);
    return defaultText || ''; // Return default or empty
};
```

**Manifest Update (`extension/manifest.json`)**:
```json
"content_scripts": [
    {
        "matches": ["<all_urls>"],
        "js": ["suppress_alerts.js", "scanner.js", "content.js"],
        "run_at": "document_start"
    }
]
```

---

## Summary of Actions

| Mode | Strategy | Complexity | Robustness |
|------|----------|------------|------------|
| **Headless** | CDP Event Listener | Medium | High (Native handling) |
| **Embedded** | WebDriver Capability | Low | High (Standard API) |
| **Remote** | JS Injection (Monkey Patch) | Low | High (Prevents blocking entirely) |

## Immediate Next Steps
1.  Apply the CDP fix to `oryn-h` (as previously identified).
2.  Update `oryn-e` capabilities to include `unhandledPromptBehavior`.
3.  Add `suppress_alerts.js` to the browser extension for `oryn-r`.
