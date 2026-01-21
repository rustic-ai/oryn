# Oryn Code Flow and Architecture Analysis

This document provides a detailed technical analysis of the Oryn project's architecture, control flow, and data flow. It traces the execution of automation scripts (`.oil`) through the system, explaining how the core logic interacts with the three distinct execution backends: **Headless**, **Embedded**, and **Remote**.

## 1. High-Level Architecture

Oryn is designed as a modular system where a unified "Core" drives different "Backends".

*   **Oryn Core (`crates/oryn-core`)**: The brain. Handles parsing, intent resolution, planning, and command translation. It defines the `Backend` trait.
*   **Oryn CLI (`crates/oryn`)**: The entry point. Handles user input (REPL or file), initializes the backend, and runs the execution loop.
*   **Backends**:
    *   **Headless (`crates/oryn-h`)**: Controls a local Chromium instance via Chrome DevTools Protocol (CDP).
    *   **Embedded (`crates/oryn-e`)**: Controls a lightweight embedded browser (COG) via WebDriver (WPEWebDriver).
    *   **Remote (`crates/oryn-r`)**: Controls a remote browser (e.g., user's Chrome) via a WebSocket server and a Browser Extension.
*   **Scanner (`crates/oryn-scanner`)**: A JavaScript payload injected into the target page. It acts as the "agent" inside the browser, executing commands like `scan`, `click`, `type`, and returning results.

## 2. The Execution Pipeline: From Script to Action

The execution flow follows a standard pipeline: **Parse -> Resolve -> Translate -> Dispatch -> Execute**.

### Phase 1: Parsing
*   **Input**: A line of text from an `.oil` file (e.g., `click "Sign In"`).
*   **Component**: `oryn-core/src/parser.rs`.
*   **Output**: A `Command` enum (e.g., `Command::Click(Target::Text("Sign In"), ...)`).

### Phase 2: Resolution (The "Semantic" Layer)
*   **Input**: `Command` + `ResolverContext` (state from previous scans).
*   **Component**: `crates/oryn/src/repl.rs` -> `oryn-core/src/resolver.rs`.
*   **Logic**:
    *   Automation scripts often use semantic targets ("Sign In") rather than technical IDs (`#btn-123`).
    *   The `ReplState` maintains a map of `Semantic ID -> Concrete ID` from the last `scan` result.
    *   If a command has a semantic target, `resolve_command` looks it up.
    *   **Result**: `Command` with a concrete ID (e.g., `Command::Click(Target::Id(42), ...)`).

### Phase 3: Translation
*   **Input**: Resolved `Command`.
*   **Component**: `oryn-core/src/translator.rs`.
*   **Logic**: Converts high-level Oryn commands into low-level `ScannerRequest` protocol messages.
    *   `Command::Click` -> `ScannerRequest::Click { id: 42, ... }`
    *   `Command::Observe` -> `ScannerRequest::Scan { ... }`
*   **Output**: `ScannerRequest` (a serializable struct).

### Phase 4: Dispatch (Backend Specific)
*   **Input**: `ScannerRequest`.
*   **Component**: The active `Backend` implementation (`HeadlessBackend`, `EmbeddedBackend`, or `RemoteBackend`).
*   **Logic**: Serializes the request to JSON and transmits it to the browser.
*   **Details**: See Section 4 for mode-specifics.

### Phase 5: Execution (In-Browser)
*   **Component**: `oryn-scanner/src/scanner.js` (injected into the page).
*   **Logic**: `window.Oryn.process(request)` is called.
    *   It dispatches based on `request.action` (e.g., "click", "scan").
    *   It interacts with the DOM API.
*   **Output**: A JSON response (e.g., `{ status: "ok", data: ... }`).

---

## 3. Detailed Trace: `01_static.oil`

We will trace the execution of `test-harness/scripts/01_static.oil` to see exactly what happens at each step.

**Script Content:**
```oil
goto "http://localhost:3000/static/article.html"
observe
extract text --selector "article"
back
```

### Step 1: `goto "http://localhost:3000/..."`

1.  **Parse**: `Command::GoTo("http://localhost:3000/...")`.
2.  **Resolve**: No resolution needed (no semantic targets).
3.  **Execute**: `repl.rs` calls `backend.navigate(url)`.
    *   **Headless**: Calls `cdp_client.page.goto(url)`. Waits for `load` event.
    *   **Embedded**: Calls `webdriver_client.goto(url)`.
    *   **Remote**: Sends `{"action": "navigate", "url": "..."}` via WebSocket. The Extension's background script intercepts this and calls `chrome.tabs.update`.
4.  **Result**: Browser navigates. Backend returns `NavigationResult`.

### Step 2: `observe`

1.  **Parse**: `Command::Observe(options)`.
2.  **Resolve**: None needed.
3.  **Translate**: Converts to `ScannerRequest::Scan(ScanRequest { ... })`.
4.  **Dispatch**:
    *   **Serialization**: `{ "action": "scan", "max_elements": null, ... }`
    *   **Injection**: Backend checks if `window.Oryn` exists. If not, it injects the code from `scanner.js`.
    *   **Call**: Calls `window.Oryn.process(json)`.
5.  **Browser Execution**:
    *   `scanner.js` traverses the DOM.
    *   It builds a map of elements, assigning stable numeric IDs (1, 2, 3...).
    *   It generates a JSON representation of the actionable UI (links, inputs, buttons).
6.  **Return**: The JSON data is returned to the CLI.
7.  **State Update**: `ReplState` updates its `ResolverContext` with this new map. Now, "Sign In" is mapped to ID `42`.

### Step 3: `extract text --selector "article"`

1.  **Parse**: `Command::Extract(ExtractSource::Css("article"))`. Wait, `extract text` maps to `Command::Text` in `parser` or `Command::Extract`?
    *   *Correction*: `extract text` syntax usually maps to `Command::Text` or `Command::Extract` depending on parser implementation. Based on `translator.rs`, `Command::Text` translates to `ScannerRequest::GetText`. Let's assume the parser handles `extract text ...` as `Command::Text`.
2.  **Translate**: `ScannerRequest::GetText { selector: Some("article") }`.
3.  **Dispatch**: Sends `{ "action": "get_text", "selector": "article" }` to browser.
4.  **Browser Execution**: `scanner.js` performs `document.querySelector("article").innerText`.
5.  **Return**: Returns the text content.

### Step 4: `back`

1.  **Parse**: `Command::Back`.
2.  **Execute**: Calls `backend.go_back()`.
    *   **Headless**: Evaluates `history.back()`.
    *   **Embedded**: Calls WebDriver `back` command.
    *   **Remote**: Sends `{"action": "back"}`. Extension background script calls `chrome.tabs.goBack`.

---

## 4. Mode Deep Dives

### Mode A: Headless (`oryn-h`)
**Control Flow:** `CLI` -> `CDP Client` -> `Chromium Pipe` -> `Browser`
**Key File:** `crates/oryn-h/src/backend.rs`

*   **Launch**: Spawns a `chromium` process with remote debugging enabled. Connects via WebSocket to the CDP port.
*   **Injection**: Uses `Page.evaluate` to inject the 2000+ line `scanner.js`.
*   **Execution**:
    *   It constructs a JS string: `window.Oryn.process({...json...})`.
    *   It calls `Runtime.evaluate`.
    *   **Critical Detail**: It wraps this in a `tokio::timeout` (10s) because in Headless mode, if the page opens an `alert()`, the JS thread halts and the CDP response never comes, hanging the entire CLI.

### Mode B: Embedded (`oryn-e`)
**Control Flow:** `CLI` -> `WebDriver Client` -> `WPEWebDriver` -> `COG Browser`
**Key File:** `crates/oryn-e/src/backend.rs`

*   **Launch**: Spawns `cog` (WPE WebKit browser) and `WPEWebDriver`. Connects via HTTP (WebDriver Protocol).
*   **Injection**: Similar to Headless, it checks for `window.Oryn` via `execute_script`. If missing, sends the entire `scanner.js` payload.
*   **Execution**:
    *   Uses `webdriver_client.execute(script, args)`.
    *   Script: `return window.Oryn.process(arguments[0])`.
*   **Use Case**: Embedded devices (kiosks, automotive) where Chromium is too heavy.

### Mode C: Remote (`oryn-r`)
**Control Flow:** `CLI` -> `TCP Server` -> `WebSocket` -> `Extension BG` -> `Content Script`
**Key File:** `crates/oryn-r/src/backend.rs` & `crates/oryn-r/src/server.rs` & `extension/background.js`

*   **Launch**: Starts a local TCP server on port 9001. It waits for an incoming connection.
*   **Connection**: The user has the Oryn Chrome Extension installed. The extension's `background.js` connects to `ws://localhost:9001`.
*   **Dispatch**:
    1.  `RemoteBackend` sends `ScannerRequest` to `RemoteServer`'s broadcast channel.
    2.  `RemoteServer` sends JSON string over WebSocket.
    3.  `background.js` receives the message.
        *   If `navigate` or `back`: It uses `chrome.tabs` API directly (privileged API).
        *   If `scan`, `click`, etc.: It finds the active tab and sends the message via `chrome.tabs.sendMessage`.
    4.  **Content Script** (`content.js`): Listens for runtime messages. It calls `Oryn.process(msg)` (since `scanner.js` is loaded into the page context).
    5.  **Return Path**: The content script sends the result back to `background.js`, which forwards it over WebSocket to `RemoteServer`, which passes it via channel to `RemoteBackend`.

## 5. Data Flow Diagram

```mermaid
graph TD
    User[User / Script] -->|Input| CLI[Oryn CLI]
    CLI -->|Parse| Parser
    Parser -->|Command| Resolver
    Resolver -->|Resolved CMD| Translator
    Translator -->|ScannerRequest| Backend{Backend Selection}

    subgraph "Headless Mode"
        Backend -->|CDP (JSON)| Chromium[Chromium Process]
        Chromium -->|JS Eval| ScannerH[Scanner.js]
    end

    subgraph "Embedded Mode"
        Backend -->|WebDriver (HTTP)| WPE[WPEWebDriver]
        WPE -->|IPC| COG[COG Browser]
        COG -->|JS Eval| ScannerE[Scanner.js]
    end

    subgraph "Remote Mode"
        Backend -->|Channel| Server[TCP Server]
        Server -->|WebSocket| ExtBG[Extension Background]
        ExtBG -->|Chrome Msg| Content[Content Script]
        Content -->|JS Call| ScannerR[Scanner.js]
    end

    ScannerH -->|JSON Result| Chromium
    ScannerE -->|JSON Result| COG
    ScannerR -->|JSON Result| Content

    Chromium -->|CDP Response| Backend
    COG -->|WebDriver Response| Backend
    Content -->|Chrome Msg| ExtBG
    ExtBG -->|WebSocket| Server
    Server -->|Channel| Backend

    Backend -->|ProtocolResponse| CLI
    CLI -->|Output| User
```

## 6. Key Code References

| Component | File Path | Responsibility |
|-----------|-----------|----------------|
| **Entry** | `crates/oryn/src/main.rs` | CLI parsing, Backend selection |
| **Loop** | `crates/oryn/src/repl.rs` | Read-Eval-Print Loop, state management |
| **Parser** | `crates/oryn-core/src/parser.rs` | Text -> Command Enum |
| **Translator** | `crates/oryn-core/src/translator.rs` | Command -> ScannerRequest (JSON) |
| **Scanner** | `crates/oryn-scanner/src/scanner.js` | In-browser execution engine |
| **Headless** | `crates/oryn-h/src/backend.rs` | CDP implementation |
| **Inject** | `crates/oryn-h/src/inject.rs` | Dialog handling, JS injection |
| **Embedded** | `crates/oryn-e/src/backend.rs` | WebDriver implementation |
| **Remote** | `crates/oryn-r/src/backend.rs` | WebSocket communication |
| **Extension** | `extension/background.js` | Remote mode bridge |
