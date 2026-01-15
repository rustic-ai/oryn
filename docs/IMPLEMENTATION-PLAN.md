# Lemmascope Implementation Plan

## Overview

This document outlines the remaining work to complete Lemmascope based on comparison of SPEC-UNIFIED.md, SPEC-INTENT-LANGUAGE.md, and SPEC-SCANNER-PROTOCOL.md against the current codebase.

### Current State Summary

| Component              | Status      | Completeness |
| ---------------------- | ----------- | ------------ |
| Intent Language Parser | Implemented | 100%         |
| Universal Scanner      | Implemented | ~85%         |
| Backend Trait          | Implemented | 100%         |
| lscope-e (Embedded)    | Stub Only   | 0%           |
| lscope-h (Headless)    | Implemented | 100%         |
| lscope-r (Remote)      | Implemented | 100%         |
| Browser Extension      | Implemented | 100%         |
| CLI Interface          | Implemented | 80%          |

---

## Phase 1: Core Infrastructure

### 1.1 Define Backend Trait (lscope-core)

**Location**: `crates/lscope-core/src/backend.rs`

The Backend trait is the unified interface all three binaries must implement. Per SPEC-UNIFIED.md Section 5.1:

**Tasks**:
- [x] Create `Backend` trait with async methods:
  ```rust
  trait Backend {
      async fn launch(&mut self) -> Result<()>;
      async fn close(&mut self) -> Result<()>;
      async fn is_ready(&self) -> bool;
      async fn navigate(&mut self, url: &str) -> Result<NavigationResult>;
      async fn execute_scanner(&mut self, command: ScannerCommand) -> Result<ScannerResponse>;
      async fn screenshot(&mut self) -> Result<Vec<u8>>;
  }
  ```
- [x] Define `ScannerCommand` enum matching scanner protocol (scan, click, type, etc.)
- [x] Define `ScannerResponse` struct with `ok`, `error`, `code`, `data`, `timing` fields
- [x] Define `NavigationResult` with URL, title, load status
- [x] Define error types for backend operations

### 1.2 Scanner Protocol Types (lscope-core)

**Location**: `crates/lscope-core/src/protocol.rs`

Define Rust types matching SPEC-SCANNER-PROTOCOL.md:

**Tasks**:
- [x] Define `ScanRequest` struct with parameters:
  - `max_elements`, `include_hidden`, `near`, `within`, `viewport_only`
- [x] Define `ClickRequest` struct with parameters:
  - `id`, `button`, `click_count`, `modifiers`, `offset`, `force`, `scroll_into_view`
- [x] Define `TypeRequest`, `SelectRequest`, `ScrollRequest`, etc.
- [x] Define `ScanResponse` with:
  - `page` (url, title, viewport, scroll, readyState)
  - `elements` (Vec<Element>)
  - `patterns` (detected UI patterns)
  - `stats` (counts, timing)
- [x] Define `Element` struct matching scanner output:
  - `id`, `type_`, `role`, `text`, `label`, `selector`, `xpath`
  - `rect`, `attributes`, `state`
- [x] Define `Pattern` structs (LoginForm, SearchForm, Pagination, Modal, CookieBanner)
- [x] Define all error codes as enum per Section 2.3

### 1.3 Command Translation Layer (lscope-core)

**Location**: `crates/lscope-core/src/translator.rs`

Bridge between parsed Intent Language commands and Scanner Protocol JSON:

**Tasks**:
- [x] Implement `Command` → `ScannerCommand` translation
- [x] Handle target resolution:
  - `Target::Id(n)` → `{"id": n}`
  - `Target::Text(s)` → requires scan + text matching
  - `Target::Role(r)` → requires scan + role matching
  - `Target::Selector(s)` → use selector directly
- [ ] Implement semantic target resolution (find element by text/role)
- [x] Handle command options → scanner parameters mapping

### 1.4 Response Formatting (lscope-core)

**Location**: `crates/lscope-core/src/formatter.rs`

Format scanner responses into Intent Language output format per SPEC-INTENT-LANGUAGE.md Section 4:

**Tasks**:
- [x] Implement success response format: `ok <command> [details]`
- [x] Implement observation format:
  - Page header: `@ domain.com/path "Page Title"`
  - Element notation: `[id] type/role "text" {modifiers}`
- [x] Implement error response format with recovery hints
- [ ] Implement change notation (+, -, ~, @)
- [ ] Implement verbosity levels (compact, full, minimal)

---

## Phase 2: Remote Mode (lscope-r)

Per SPEC-UNIFIED.md Section 8.1, Remote mode is the first backend to implement for visual debugging.

### 2.1 WebSocket Server (lscope-r)

**Location**: `crates/lscope-r/src/server.rs`

**Tasks**:
- [x] Add dependencies: `tokio`, `tokio-tungstenite`, `serde_json`
- [x] Implement WebSocket server accepting extension connections
- [x] Define message protocol between server and extension:
  - Server → Extension: scanner commands (JSON)
  - Extension → Server: scanner responses (JSON)
- [x] Handle connection lifecycle (connect, disconnect, reconnect)
- [x] Implement heartbeat/keepalive mechanism

### 2.2 Remote Backend Implementation (lscope-r)

**Location**: `crates/lscope-r/src/backend.rs`

**Tasks**:
- [x] Implement `Backend` trait for remote mode
- [x] `launch()`: Start WebSocket server, wait for extension connection
- [x] `close()`: Close WebSocket connection gracefully
- [x] `is_ready()`: Check extension connection status
- [x] `navigate()`: Send navigation command to extension
- [x] `execute_scanner()`: Send command, await response with timeout
- [x] `screenshot()`: Request and receive screenshot from extension
- [x] Handle connection errors and reconnection

### 2.3 Browser Extension - Manifest and Structure

**Location**: `extension/`

**Tasks**:
- [x] Create `manifest.json` (Manifest V3):
  ```json
  {
    "manifest_version": 3,
    "name": "Lemmascope Agent",
    "permissions": ["activeTab", "scripting"],
    "background": { "service_worker": "background.js" },
    "content_scripts": [{ "matches": ["<all_urls>"], "js": ["content.js"] }]
  }
  ```
- [x] Create extension directory structure:
  - `manifest.json`
  - `background.js` (service worker)
  - `content.js` (scanner injection and communication)
  - `popup.html` / `popup.js` (optional status UI)

### 2.4 Browser Extension - Background Service Worker

**Location**: `extension/background.js`

**Tasks**:
- [x] Implement WebSocket client connecting to lscope-r server
- [x] Route messages between server and content script
- [x] Handle tab management (current active tab tracking)
- [x] Implement connection status tracking
- [x] Handle reconnection logic

### 2.5 Browser Extension - Content Script

**Location**: `extension/content.js`

**Tasks**:
- [x] Inject scanner.js into page context
- [x] Listen for messages from background script
- [x] Execute scanner commands via `window.postMessage` or direct call
- [x] Return scanner responses to background script
- [x] Handle page navigation (re-inject scanner on new pages)

### 2.6 lscope-r CLI

**Location**: `crates/lscope-r/src/main.rs`

**Tasks**:
- [x] Add CLI argument parsing (clap)
- [x] Implement REPL mode for interactive commands
- [x] Implement single-command mode
- [x] Implement batch/script mode
- [x] Add `--port` option for WebSocket server
- [x] Add `--verbose` option for debug output

---

## Phase 3: Headless Mode (lscope-h)

### 3.1 CDP Client Setup (lscope-h)

**Location**: `crates/lscope-h/src/cdp.rs`

**Tasks**:
- [x] Add dependency: `chromiumoxide` or `headless_chrome`
- [x] Implement Chrome/Chromium process management:
  - Launch with appropriate flags
  - Connect via DevTools Protocol
  - Handle process lifecycle
- [x] Implement CDP session management

### 3.2 Headless Backend Implementation (lscope-h)

**Location**: `crates/lscope-h/src/backend.rs`

**Tasks**:
- [x] Implement `Backend` trait for headless mode
- [x] `launch()`: Start Chrome with `--headless`, connect via CDP
- [x] `close()`: Terminate Chrome process cleanly
- [x] `is_ready()`: Check CDP connection status
- [x] `navigate()`: Use CDP `Page.navigate`, wait for load
- [x] `execute_scanner()`: Inject scanner via `Runtime.evaluate`, execute commands
- [x] `screenshot()`: Use CDP `Page.captureScreenshot`
- [x] Handle Chrome crash recovery

### 3.3 Scanner Injection via CDP

**Location**: `crates/lscope-h/src/inject.rs`

**Tasks**:
- [x] Implement scanner injection using `Runtime.evaluate`
- [x] Handle injection timing (after DOMContentLoaded)
- [x] Re-inject scanner after page navigation
- [x] Handle iframe injection (if needed)

### 3.4 CDP-Specific Features

**Location**: `crates/lscope-h/src/features.rs`

Per SPEC-UNIFIED.md, headless mode should support:

**Tasks**:
- [ ] Network interception (optional, via CDP `Fetch` domain)
- [ ] PDF generation (`Page.printToPDF`)
- [ ] DevTools debugging integration
- [ ] Console message capture

### 3.5 lscope-h CLI

**Location**: `crates/lscope-h/src/main.rs`

**Tasks**:
- [x] CLI argument parsing
- [x] `--chrome-path` option for custom Chrome location
- [x] `--user-data-dir` option for profile persistence
- [x] REPL, single-command, and batch modes

---

## Phase 4: Embedded Mode (lscope-e)

### 4.1 WebDriver Client Setup (lscope-e)

**Location**: `crates/lscope-e/src/webdriver.rs`

**Tasks**:
- [ ] Add dependency: `fantoccini`
- [ ] Implement WebDriver session management
- [ ] Handle geckodriver/chromedriver/COG WebDriver connection
- [ ] Implement capability negotiation

### 4.2 Embedded Backend Implementation (lscope-e)

**Location**: `crates/lscope-e/src/backend.rs`

**Tasks**:
- [ ] Implement `Backend` trait for embedded mode
- [ ] `launch()`: Connect to WebDriver server (COG)
- [ ] `close()`: End WebDriver session
- [ ] `is_ready()`: Check WebDriver connection
- [ ] `navigate()`: Use WebDriver navigation commands
- [ ] `execute_scanner()`: Inject scanner via `execute_script`
- [ ] `screenshot()`: Use WebDriver screenshot command

### 4.3 COG/WPE WebKit Integration

**Location**: `crates/lscope-e/src/cog.rs`

**Tasks**:
- [ ] Document COG setup and requirements
- [ ] Implement COG process management (if needed)
- [ ] Handle WPE-specific quirks
- [ ] Test on resource-constrained environment

### 4.4 lscope-e CLI

**Location**: `crates/lscope-e/src/main.rs`

**Tasks**:
- [ ] CLI argument parsing
- [ ] `--webdriver-url` option for WebDriver server
- [ ] REPL, single-command, and batch modes

---

## Phase 5: Scanner Protocol Gaps

Based on SCANNER-GAPS.md and spec comparison:

### 5.1 Missing Response Fields

**Location**: `crates/lscope-scanner/src/scanner.js`

**Tasks**:
- [ ] Add `navigation` boolean to click response
- [ ] Add `dom_changes` details to click response (elements added/removed/modified with IDs)
- [ ] Add `selector` field to action responses (type, clear, check, etc.)
- [ ] Add `timing` object to all responses
- [ ] Add `index` field to select response when selecting by index
- [ ] Add `form_selector` and `form_id` to submit response

### 5.2 Missing Command Parameters

**Location**: `crates/lscope-scanner/src/scanner.js`

**Tasks**:
- [ ] Add `offset` parameter support to `hover` command
- [ ] Make `poll_interval` configurable in `wait_for` command
- [ ] Add `include_iframes` parameter to scan (currently always true)

### 5.3 Enhanced Element Attributes

**Location**: `crates/lscope-scanner/src/scanner.js`

**Tasks**:
- [ ] Add more `data-*` attribute capture
- [ ] Improve selector generation for edge cases
- [ ] Add `aria-describedby` attribute capture

### 5.4 Navigation Condition Enhancement

**Location**: `crates/lscope-scanner/src/scanner.js`

Per SPEC-SCANNER-PROTOCOL.md Section 3.14:

**Tasks**:
- [ ] Implement `navigation` wait condition properly
- [ ] Return previous and current URL in navigation wait response

---

## Phase 6: Intent Language Parser Gaps

### 6.1 Missing Commands

**Location**: `crates/lscope-core/src/parser.rs`

Per SPEC-INTENT-LANGUAGE.md, these commands need implementation:

**Tasks**:
- [x] Add `submit` command parsing (currently missing)
- [x] Add `storage` command parsing for localStorage/sessionStorage
- [x] Verify all command aliases are implemented

### 6.2 Intent Commands (Level 3)

Per SPEC-INTENT-LANGUAGE.md Section 5, Level 3 Intent Commands:

**Tasks**:
- [x] Implement `login <email> <password>` composite command
- [x] Implement `search <query>` composite command
- [x] Implement `dismiss popups` composite command
- [x] Implement `accept cookies` composite command
- [x] Implement `scroll until <target>` composite command

These require:
- Pattern detection (already in scanner)
- Multi-step command execution
- State tracking between steps

### 6.3 Modifier Support

**Location**: `crates/lscope-core/src/parser.rs`

Per SPEC-INTENT-LANGUAGE.md Section 8:

**Tasks**:
- [x] Implement `near` modifier: `click "Add" near "Cart"`
- [x] Implement `after`/`before` modifiers
- [x] Implement `inside` modifier: `click "Submit" inside "Login Form"`
- [x] Implement `contains` modifier

---

## Phase 7: Integration and Testing

### 7.1 End-to-End Integration Tests

**Location**: `tests/integration/`

**Tasks**:
- [ ] Create integration test harness with real browser
- [ ] Test full flow: Intent command → Parser → Backend → Scanner → Response
- [ ] Test each backend (e, h, r) with same test suite
- [ ] Verify behavioral consistency across backends per SPEC-UNIFIED.md Section 6.1

### 7.2 Cross-Backend Consistency Tests

**Tasks**:
- [ ] Create test suite that runs identical commands on all three backends
- [ ] Verify observations are identical given same page state
- [ ] Verify actions produce identical results
- [ ] Document any unavoidable differences

### 7.3 Error Handling Tests

**Tasks**:
- [ ] Test all error codes from SPEC-SCANNER-PROTOCOL.md Section 2.3
- [ ] Verify error recovery hints are provided
- [ ] Test timeout behavior
- [ ] Test connection failure recovery

---

## Phase 8: Polish and Developer Experience

### 8.1 Unified CLI Experience

Per SPEC-UNIFIED.md Section 8.4:

**Tasks**:
- [ ] Create unified `lscope` binary that can invoke e/h/r modes
- [ ] Implement mode auto-detection based on environment
- [ ] Consistent argument handling across modes
- [ ] Help text and documentation

### 8.2 Logging and Debugging

**Tasks**:
- [ ] Add structured logging (tracing crate)
- [ ] Debug mode with verbose scanner output
- [ ] Request/response logging for troubleshooting
- [ ] Performance metrics collection

### 8.3 Configuration System

Per SPEC-INTENT-LANGUAGE.md Section 7:

**Tasks**:
- [ ] Implement default settings (timeout: 30s, verbosity: compact, etc.)
- [ ] Support configuration file
- [ ] Per-command option overrides
- [ ] Environment variable configuration

### 8.4 Documentation

**Tasks**:
- [ ] API documentation (rustdoc)
- [ ] User guide with examples
- [ ] Troubleshooting guide
- [ ] Architecture documentation for contributors

---

## Implementation Order Recommendation

Based on SPEC-UNIFIED.md Section 8 development priorities:

1. **Phase 1**: Core Infrastructure (Backend trait, Protocol types, Translation layer)
2. **Phase 2**: Remote Mode (lscope-r) - enables visual debugging during development
3. **Phase 5**: Scanner gaps - while testing remote mode
4. **Phase 6**: Parser gaps - intent commands require working backend
5. **Phase 3**: Headless Mode (lscope-h) - production backend
6. **Phase 4**: Embedded Mode (lscope-e) - specialized environment
7. **Phase 7**: Integration testing across all backends
8. **Phase 8**: Polish and documentation

---

## Dependencies to Add

### lscope-core
```toml
[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
```

### lscope-r
```toml
[dependencies]
lscope-core = { path = "../lscope-core" }
lscope-scanner = { path = "../lscope-scanner" }
tokio = { version = "1", features = ["full"] }
tokio-tungstenite = "0.21"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = "0.3"
```

### lscope-h
```toml
[dependencies]
lscope-core = { path = "../lscope-core" }
lscope-scanner = { path = "../lscope-scanner" }
chromiumoxide = "0.5"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = "0.3"
```

### lscope-e
```toml
[dependencies]
lscope-core = { path = "../lscope-core" }
lscope-scanner = { path = "../lscope-scanner" }
fantoccini = "0.19"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = "0.3"
```

---

*Document Version: 1.0*
*Generated: January 2025*
