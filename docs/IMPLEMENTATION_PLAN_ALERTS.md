# Implementation Plan: Alert Dialog Handling

This document outlines the step-by-step plan to implement robust handling for blocking JavaScript dialogs (`alert`, `confirm`, `prompt`) across all Oryn backends.

## Phase 1: Headless Backend (`oryn-h`)

**Strategy**: CDP Event Listeners
**Objective**: Prevent `Page.evaluate` from timing out by auto-accepting dialogs via the Chrome DevTools Protocol.

### Steps
1.  **Modify `crates/oryn-h/src/cdp.rs`**:
    *   Import `chromiumoxide::cdp::browser_protocol::page::EventJavascriptDialogOpening`.
    *   In `CdpClient::launch`, subscribe to the dialog opening event stream.
    *   Spawn a background `tokio` task (parallel to the console listener) that consumes this stream.
    *   For each event, execute `page.handle_javascript_dialog(true, None)` to accept the dialog.

### Verification
*   Run `scripts/run-e2e-tests.sh oryn-h`
*   **Success Criteria**: Tests `06_edge_cases.oil`, `07_intents_builtin.oil`, and `09_target_resolution.oil` must PASS (currently failing with timeouts).

---

## Phase 2: Embedded Backend (`oryn-e`)

**Strategy**: WebDriver Capabilities
**Objective**: Instruct WPEWebDriver to automatically accept alerts.

### Steps
1.  **Modify `crates/oryn-e/src/cog.rs`**:
    *   Update the `wpe_capabilities()` function.
    *   Insert the standard WebDriver capability: `caps.insert("unhandledPromptBehavior".to_string(), json!("accept"));`.

### Verification
*   Run `scripts/run-e2e-tests.sh oryn-e-debian`
*   **Success Criteria**: Tests involving alerts should proceed without hanging or throwing `UnexpectedAlertOpen` errors.

---

## Phase 3: Remote Backend (`oryn-r`)

**Strategy**: JavaScript Monkey Patching
**Objective**: Suppress blocking dialogs in the user's browser without requiring invasive `debugger` permissions.

### Steps
1.  **Create `extension/suppress_alerts.js`**:
    *   Implement overrides for `window.alert`, `window.confirm`, and `window.prompt`.
    *   `alert`: Log to console, return undefined.
    *   `confirm`: Log to console, return `true`.
    *   `prompt`: Log to console, return empty string (or default value).
2.  **Update `extension/manifest.json`**:
    *   Add `suppress_alerts.js` to the `content_scripts` array.
    *   **Crucial**: Set `"run_at": "document_start"` to ensure it loads before page scripts can trigger alerts.

### Verification
*   Run `scripts/run-e2e-tests.sh oryn-r`
*   **Success Criteria**: The browser window should NOT show visible alerts during test execution, and the tests should pass.

---

## Execution Order

1.  **Phase 1 (Headless)**: Highest priority as it fixes the immediate CI failures reported in diagnostics.
2.  **Phase 3 (Remote)**: High priority for developer experience/local testing.
3.  **Phase 2 (Embedded)**: Verification requires Dockerized WPE environment.

## Risks & Mitigations

*   **Risk**: `unhandledPromptBehavior` is ignored by WPEWebDriver.
    *   *Mitigation*: If Phase 2 fails, fallback to injecting the `suppress_alerts.js` script in `oryn-e` (similar to Remote mode) using `Page.addScriptToEvaluateOnNewDocument` equivalent or standard injection.
*   **Risk**: Monkey patch runs too late.
    *   *Mitigation*: Ensure `document_start` is correctly respected by the extension runner.
