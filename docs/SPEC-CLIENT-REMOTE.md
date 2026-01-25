# Oryn Client Libraries and Remote Extension Specification

## Version 1.1

---

## 1. Overview

This document specifies the architecture and protocols for Oryn client libraries and the remote mode browser extension. It establishes conventions for how agents written in any language communicate with Oryn engines across all three modes (Embedded, Headless, Remote).

### 1.1 Design Principles

**Thin Clients**

Client libraries handle transport, not abstraction. The Intent Language is the abstraction layer. Clients send command strings and receive response strings—nothing more.

**Subprocess Over FFI**

For Embedded and Headless modes, clients spawn the engine binary as a subprocess. This provides crash isolation, build simplicity, and independent versioning. The latency overhead (~0.1ms) is negligible compared to browser operations (100-2000ms).

**Client-Owned Server for Remote**

In Remote mode, the browser extension connects as a WebSocket client to an endpoint provided by the user's infrastructure. Oryn does not provide or require a relay server. What runs behind the WebSocket endpoint is entirely the client's concern.

**Engine in Extension**

The Intent Language parser, semantic resolver, and response formatter run as WASM inside the browser extension. This keeps any server-side component trivial (pure message forwarding) and enables fully client-side operation.

**Named Sessions**

Multiple isolated browser sessions can run simultaneously, each with independent state, cookies, and element maps.

### 1.2 Document Scope

This specification covers:

- Client library responsibilities and boundaries
- Transport mechanisms for each mode
- Remote extension architecture
- Wire protocols between components
- Binary distribution strategy
- Session management
- Output format modes (text and JSON)

This specification does not cover:

- Intent Language syntax (see SPEC-INTENT-LANGUAGE.md)
- Scanner protocol (see SPEC-SCANNER-PROTOCOL.md)
- Engine internals (see SPEC-UNIFIED.md)

---

## 2. Architecture Overview

### 2.1 Embedded and Headless Modes

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ Client Process                                                              │
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ Agent Code (Python/TypeScript/Any Language)                         │   │
│  │                                                                      │   │
│  │  ┌───────────────────────────────────────────────────────────────┐  │   │
│  │  │ Oryn Client Library                                           │  │   │
│  │  │ • Spawn subprocess                                            │  │   │
│  │  │ • Write commands to stdin                                     │  │   │
│  │  │ • Read responses from stdout                                  │  │   │
│  │  │ • Manage named sessions                                       │  │   │
│  │  └───────────────────────────────────────────────────────────────┘  │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                              │ stdin/stdout                                 │
│                              ↓                                              │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ oryn-e or oryn-h (subprocess)                                       │   │
│  │                                                                      │   │
│  │ • Intent Language Parser                                            │   │
│  │ • Semantic Resolver                                                 │   │
│  │ • Response Formatter                                                │   │
│  │ • Backend (WebDriver or CDP)                                        │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                              │ WebDriver HTTP / CDP WebSocket               │
└──────────────────────────────│──────────────────────────────────────────────┘
                               ↓
                    ┌─────────────────────┐
                    │ Browser Instance    │
                    │ (WPE WebKit or      │
                    │  Chromium)          │
                    └─────────────────────┘
```

### 2.2 Remote Mode

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ Client Infrastructure (User Provided)                                       │
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ Agent Code + WebSocket Server                                        │   │
│  │                                                                      │   │
│  │ The client provides a WebSocket endpoint. Implementation is          │   │
│  │ entirely their concern—could be Python, Node, Go, Rust, or any      │   │
│  │ language or framework.                                               │   │
│  │                                                                      │   │
│  │ The endpoint receives Intent Language commands and returns           │   │
│  │ Intent Language responses.                                           │   │
│  │                                                                      │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                      ↑                                      │
└──────────────────────────────────────│──────────────────────────────────────┘
                                       │ WebSocket (Extension connects out)
                                       │
┌──────────────────────────────────────│──────────────────────────────────────┐
│ User's Browser                       │                                      │
│                                      ↓                                      │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ Oryn Extension                                                       │   │
│  │                                                                      │   │
│  │  ┌───────────────────────────────────────────────────────────────┐  │   │
│  │  │ Service Worker                                                 │  │   │
│  │  │                                                                │  │   │
│  │  │  ┌─────────────────────────────────────────────────────────┐  │  │   │
│  │  │  │ Engine (WASM)                                            │  │  │   │
│  │  │  │ • Intent Language Parser                                 │  │  │   │
│  │  │  │ • Semantic Resolver                                      │  │  │   │
│  │  │  │ • Response Formatter                                     │  │  │   │
│  │  │  └─────────────────────────────────────────────────────────┘  │  │   │
│  │  │                                                                │  │   │
│  │  │  WebSocket Client (connects to configured endpoint)            │  │   │
│  │  │  Tab Manager (routes commands to appropriate tabs)             │  │   │
│  │  └───────────────────────────────────────────────────────────────┘  │   │
│  │                            ↕ chrome.tabs / chrome.scripting          │   │
│  │  ┌───────────────────────────────────────────────────────────────┐  │   │
│  │  │ Content Script: Scanner.js (injected per tab)                  │  │   │
│  │  └───────────────────────────────────────────────────────────────┘  │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                            ↕ DOM Access                                     │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │ Web Pages (user's actual browsing session)                          │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Session Management

### 3.1 Named Sessions

Oryn supports running multiple isolated browser sessions simultaneously. Each session has:
- Independent browser process (or context)
- Separate element ID maps
- Isolated cookies and storage
- Independent navigation history

### 3.2 Session Activation

**Command-line flag:**
```bash
oryn --session <name> <mode>
oryn --session agent1 headless
oryn --session agent2 headless
```

**Environment variable:**
```bash
ORYN_SESSION=agent1 oryn headless
```

**Default session:**
When no session name is specified, the session is named `default`.

### 3.3 Session Architecture (Embedded/Headless)

Each named session is an independent OS process:

```
┌─────────────────────────────────────────────────┐
│ Parent Orchestrator (optional)                   │
│                                                  │
│  Session Registry: ~/.oryn/sessions/             │
│  ├── default.pid                                 │
│  ├── agent1.pid                                  │
│  └── agent2.pid                                  │
└─────────────────────────────────────────────────┘
         │              │              │
         ↓              ↓              ↓
    ┌─────────┐   ┌─────────┐   ┌─────────┐
    │ default │   │ agent1  │   │ agent2  │
    │ process │   │ process │   │ process │
    │         │   │         │   │         │
    │ Browser │   │ Browser │   │ Browser │
    └─────────┘   └─────────┘   └─────────┘
```

### 3.4 Session Commands

**List sessions:**
```
sessions
```

Response:
```
ok sessions

# active sessions
- default (current)
- agent1
- agent2
```

**Show current session:**
```
session
```

Response:
```
ok session

# session
name: agent1
mode: headless
started: 2026-01-24T10:30:00Z
pages: 3
url: https://example.com
```

**Create new session:**
```
session new agent3
session new agent3 --mode headless
```

**Close session:**
```
session close agent3
```

### 3.5 Cross-Session Commands (Advanced)

From a controller process, commands can target specific sessions:

```
# Target specific session
@agent1 goto example.com
@agent2 observe

# Or via explicit command
session agent1 goto example.com
```

### 3.6 Session Isolation

Sessions are fully isolated:

| Resource | Isolation |
|----------|-----------|
| Cookies | Separate cookie jar per session |
| localStorage | Separate per session |
| sessionStorage | Separate per session |
| Element IDs | Independent maps |
| Navigation history | Independent |
| Network state | Independent |

---

## 4. Client Library Specification

### 4.1 Responsibilities

Client libraries are responsible for:

| Responsibility | Description |
|----------------|-------------|
| Transport management | Spawn subprocess or open WebSocket |
| Connection lifecycle | Connect, disconnect, reconnect |
| Command transmission | Send Intent Language strings |
| Response reception | Receive Intent Language strings |
| Binary discovery | Locate engine binary in PATH |
| Session management | Track and manage named sessions |

Client libraries are explicitly NOT responsible for:

| Non-Responsibility | Reason |
|--------------------|--------|
| Response parsing | Agent reasons about raw text |
| Retry logic | Infrastructure concern |
| Workflow abstraction | Agent's job |
| State management | Agent's job |
| Logging | Client's choice of framework |

### 4.2 Public Interface

Client libraries expose a minimal interface:

**Constructor**

Accepts mode selection (embedded, headless, remote), mode-specific configuration (port for remote mode), and optional session name.

**connect(session_name?)**

Establishes connection to engine. For embedded/headless, spawns subprocess. For remote, starts WebSocket server and waits for extension connection. Optional session name for named sessions.

**send(command: string) → string**

Sends an Intent Language command and returns the response. This is the core method. All interaction flows through this single function.

**close()**

Terminates connection. For subprocess modes, sends quit command and terminates process. For remote, closes WebSocket server.

**Convenience Methods (Optional)**

Typed methods that delegate to send() for improved developer experience:

- goto(url) → send("goto {url}")
- observe() → send("observe")  
- click(target) → send("click {target}")
- type(target, text) → send("type {target} \"{text}\"")

These are syntactic sugar. Agents can use send() directly with raw Intent Language.

**Session Methods**

- list_sessions() → send("sessions")
- get_session() → send("session")
- new_session(name, mode?) → send("session new {name}")
- close_session(name) → send("session close {name}")

### 4.3 Subprocess Transport

For Embedded and Headless modes, the client spawns the engine binary as a subprocess.

**Process Lifecycle**

1. Client locates binary (oryn-e or oryn-h) in PATH
2. Client spawns process with stdin/stdout pipes, including --session flag if specified
3. Client reads ready signal from stdout
4. Client enters command-response loop
5. On close, client sends "quit" command and terminates process

**Message Framing**

Commands are newline-terminated:

```
{command}\n
```

Responses are delimiter-terminated:

```
{response lines}
---
```

The delimiter "---" on its own line marks the end of a response. This allows multi-line responses without length prefixing.

**Binary Discovery**

Clients search for binaries in this order:

1. Explicit path provided by user
2. System PATH (via which/where)
3. Common installation locations:
   - ~/.local/bin/
   - /usr/local/bin/
   - ~/.cargo/bin/
4. If not found, raise descriptive error with installation instructions

### 4.4 WebSocket Server Transport

For Remote mode, the client runs a WebSocket server that the extension connects to.

**Connection Lifecycle**

1. Client starts WebSocket server on configured port
2. Client waits for extension to connect
3. Extension connects and sends registration message
4. Client enters command-response loop
5. On close, client terminates server

**Message Framing**

Commands include a request ID for correlation:

```
{id}:{command}
```

Responses include the same ID:

```
{id}:{response}
```

The ID is a monotonically increasing integer. This allows the client to correlate responses with requests, supporting potential future concurrent command execution.

### 4.5 Supported Languages

Official client libraries are provided for:

| Language | Package | Transport |
|----------|---------|-----------|
| Python | oryn | asyncio subprocess / websockets |
| TypeScript | oryn | child_process / ws |

The protocol is simple enough that clients in other languages can be implemented easily. The specification is:

1. Subprocess: spawn binary, write to stdin, read from stdout, parse "---" delimiter
2. Remote: run WebSocket server, frame messages as "id:content"

---

## 5. Remote Extension Specification

### 5.1 Architecture

The remote extension consists of three components:

**Service Worker (Background Script)**

- Manages WebSocket connection to client's server
- Hosts the WASM engine
- Routes commands to appropriate tabs
- Handles browser API calls (navigation, screenshots, tabs)

**WASM Engine**

- Compiled from oryn-core Rust crate
- Parses Intent Language commands
- Resolves semantic targets
- Formats responses
- Identical logic to oryn-e and oryn-h engines

**Content Script (Scanner.js)**

- Injected into web pages
- Scans DOM for interactive elements
- Executes actions (click, type, etc.)
- Returns structured observations
- Identical to scanner in other modes

### 5.2 Extension Configuration

Users configure the extension with:

| Setting | Description | Default |
|---------|-------------|---------|
| WebSocket Endpoint | URL to connect to | ws://localhost:8080 |
| Auto-Connect | Connect on browser start | false |
| Tab Scope | Which tabs to allow control | Active tab only |

Configuration is accessible via extension popup and synced via chrome.storage.

### 5.3 Connection Behavior

**Initial Connection**

1. User enters WebSocket endpoint in extension settings
2. User clicks "Connect" or enables auto-connect
3. Extension opens WebSocket connection to endpoint
4. Extension sends registration message
5. Extension displays "Connected" status

**Reconnection**

1. On connection loss, extension waits 2 seconds
2. Extension attempts to reconnect to same endpoint
3. Exponential backoff up to 30 seconds
4. Extension displays connection status throughout

**Disconnection**

1. User clicks "Disconnect" or closes browser
2. Extension closes WebSocket gracefully
3. Extension clears connection state

### 5.4 Command Processing Flow

1. Client sends command: "42:click 3"
2. Extension receives message, extracts ID (42) and command ("click 3")
3. WASM engine parses Intent Language command
4. Engine determines command type (action command targeting element 3)
5. Engine translates to Scanner JSON: {"cmd": "click", "id": 3}
6. Service worker sends to content script via chrome.tabs.sendMessage
7. Content script (Scanner.js) executes click on element 3
8. Content script returns result JSON
9. WASM engine formats as Intent Language response
10. Extension sends response: "42:ok click 3\n\n# changes\n..."

### 5.5 Tab Management

The extension manages multiple tabs:

**Tab Identification**

Each tab has a numeric ID assigned by Chrome. Commands can optionally specify a target tab.

**Default Targeting**

If no tab specified, commands target the active tab in the current window.

**Tab Commands**

The extension handles tab-related commands directly (without Scanner.js):

- tabs: Lists all tabs with IDs, URLs, and titles
- tab new {url}: Opens new tab
- tab switch {id}: Activates specified tab
- tab close {id}: Closes specified tab

**Navigation Commands**

Navigation commands (goto, back, forward, refresh) use chrome.tabs API directly rather than Scanner.js, as they affect the page at the browser level.

### 5.6 Browser API Commands

Certain commands are handled by the service worker using browser APIs:

| Command | Browser API |
|---------|-------------|
| goto | chrome.tabs.update |
| back | chrome.tabs.goBack |
| forward | chrome.tabs.goForward |
| refresh | chrome.tabs.reload |
| screenshot | chrome.tabs.captureVisibleTab |
| tabs | chrome.tabs.query |
| tab new | chrome.tabs.create |
| tab switch | chrome.tabs.update |
| tab close | chrome.tabs.remove |

All other commands (observe, click, type, etc.) are routed to Scanner.js in the content script.

### 5.6.1 Cross-Mode Behavior Consistency

Oryn promises consistent behavior across modes, but some commands have inherent platform differences:

**Screenshot Semantics**

| Mode | Implementation | Captures |
|------|----------------|----------|
| oryn-e | WebDriver screenshot | Full page (configurable) |
| oryn-h | CDP Page.captureScreenshot | Full page or viewport |
| oryn-r | chrome.tabs.captureVisibleTab | Visible viewport only |

To maintain consistency:
- Default `screenshot` captures visible viewport in all modes
- `screenshot --fullpage` captures full scrollable page (not available in remote mode)
- Remote mode returns `error: fullpage not supported` for `screenshot --fullpage`

**Tab Commands**

Tab commands (tabs, tab new, tab switch, tab close) are only meaningful in remote mode. In embedded and headless modes:
- `tabs` returns the single controlled tab
- `tab new` opens a new page in the same context
- `tab close` is not supported (closes the session)

**Network Interception**

Network interception is available in headless mode (via CDP) but not in embedded or remote modes. Commands like `intercept` return `error: not supported in this mode`.

Agents should query capabilities via registration or handle `not supported` errors gracefully.

### 5.7 Security Considerations

The extension grants significant capabilities to connected servers: DOM observation, screenshot capture, form filling, and navigation control. The security model must reflect this trust level.

**Endpoint Allowlist**

The extension maintains an allowlist of permitted WebSocket endpoints. Default allowlist:

- ws://localhost:*
- ws://127.0.0.1:*
- wss://localhost:*
- wss://127.0.0.1:*

Users can add additional endpoints via extension settings. Non-localhost endpoints MUST use wss:// (TLS). The extension refuses to connect to non-localhost ws:// endpoints.

**Connection Authentication**

To prevent malicious local processes from hijacking the connection, the extension implements pairing:

1. When adding a new endpoint, extension generates a one-time pairing code (6 alphanumeric characters)
2. Extension displays pairing code in popup UI
3. On first connection, server must send pairing code in registration message
4. Extension validates code and stores server identity (endpoint + optional name)
5. Subsequent connections from same endpoint skip pairing

Pairing codes expire after 5 minutes. Users can revoke paired endpoints at any time.

**Transport Security**

| Endpoint Type | Requirement |
|---------------|-------------|
| localhost/127.0.0.1 | ws:// or wss:// allowed |
| LAN addresses (192.168.*, 10.*, etc.) | wss:// required |
| Public endpoints | wss:// required |

The extension warns users when connecting to non-localhost endpoints that the server will have access to page contents.

**Capability Model**

Connected servers have access to:

| Capability | Risk Level | Notes |
|------------|------------|-------|
| observe/extract | High | Can read all visible page content |
| screenshot | High | Can capture visible viewport |
| click/type/select | High | Can perform actions as user |
| goto/navigation | Medium | Can navigate to any URL |
| tabs | Medium | Can see all open tab URLs/titles |
| tab close | Medium | Can close user's tabs |

Future versions may implement granular capability controls (e.g., "allow observe but not screenshot"). For v1, connection grants all capabilities.

**Domain Allowlist (Future)**

A future enhancement may add per-domain controls:

- Allowlist of domains the extension can control
- Prompt on first interaction with new domain
- Block sensitive domains (banking, email) by default

**Tab Scope**

By default, the extension only controls the active tab. Users can expand scope to all tabs in settings. Even with expanded scope, the extension cannot control:

- chrome:// pages
- Chrome Web Store pages
- Other extension pages
- about: pages

Commands targeting restricted pages return an error.

**No Credential Storage**

The extension never stores or transmits user credentials. Authentication to websites happens through the user's existing browser sessions. The extension does not have access to cookies or stored passwords—only to the rendered DOM.

### 5.8 Service Worker Lifecycle (MV3)

Chrome Manifest V3 extensions use service workers that can be suspended aggressively by the browser. The extension must handle this gracefully.

**Suspension Behavior**

The browser may suspend the service worker when:

- No active WebSocket connection
- No pending chrome API calls
- Idle timeout exceeded (typically 30 seconds)

An active WebSocket connection generally prevents suspension, but edge cases exist.

**Restart Recovery**

When the service worker restarts:

1. All in-memory state is lost (pending requests, element maps)
2. WebSocket connection is closed
3. Service worker re-initializes WASM engine
4. Service worker attempts to reconnect to last configured endpoint
5. On reconnection, extension sends fresh registration with `reconnected: true` flag
6. Client MUST treat reconnection as state reset—any in-flight commands are lost

**Client Responsibilities on Reconnection**

When client receives a registration message with `reconnected: true`:

1. Abandon any pending requests (they will never complete)
2. Assume element maps are stale
3. Re-issue `observe` before continuing automation
4. Log the reconnection for debugging

**Content Script Injection Timing**

Content scripts may not be ready immediately after navigation:

- Navigation via `goto` triggers content script injection
- There's a race between "navigation complete" and "scanner ready"
- Commands sent before scanner ready return `error: scanner not ready`

The extension mitigates this by:

1. Waiting for chrome.tabs.onUpdated status:"complete" after navigation
2. Sending a ping to content script before returning navigation success
3. Retrying ping up to 3 times with 100ms delay

Clients should still handle `scanner not ready` errors by waiting and retrying.

**Command Ordering Guarantees**

Commands are processed with the following guarantees:

| Guarantee | Scope | Description |
|-----------|-------|-------------|
| Serial execution | Per tab | Commands to a single tab execute in order |
| No interleaving | Per tab | A command completes before the next begins |
| No ordering | Cross-tab | Commands to different tabs may execute concurrently |
| Request ID correlation | Connection | Responses may arrive out of order for cross-tab commands |

For single-tab automation (the common case), commands are strictly serialized. Clients sending commands to multiple tabs concurrently must use request IDs to correlate responses.

**Keepalive Strategy**

To prevent unwanted suspension, the service worker:

1. Maintains WebSocket connection (primary keepalive)
2. Sends WebSocket ping every 25 seconds if idle
3. Uses chrome.alarms as backup wakeup (fires every 30 seconds when connected)

Despite these measures, clients should be resilient to reconnection events.

---

## 6. Wire Protocol

### 6.1 Client ↔ Engine (Subprocess)

**Direction: Client → Engine**

```
{intent_language_command}\n
```

Examples:
```
goto github.com\n
observe\n
click 3\n
type 1 "hello world"\n
```

**Direction: Engine → Client**

Responses are terminated by a line containing exactly `---`:

```
{intent_language_response}
---
```

**Delimiter Escaping**

Since `---` can appear in page content (markdown horizontal rules, code blocks, etc.), the engine escapes any line that is exactly `---` in the response payload:

| Location | Content | Wire Format |
|----------|---------|-------------|
| In payload | `---` | `\---` |
| End of response | `---` | `---` (unescaped, terminates) |

Escaping rules:
- Engine: Before sending, replace any line that is exactly `---` with `\---`
- Client: After receiving (and splitting on terminator), replace any line that is exactly `\---` with `---`

Example with markdown content:
```
Page contains:
  Line 1
  ---
  Line 2

Wire format:
  ok observe
  [1] text "Line 1"
  [2] text "\---"
  [3] text "Line 2"
  ---
```

Examples:
```
ready oryn-h v1.0.0
---
```

```
ok goto github.com

@ github.com "GitHub"
---
```

```
error click 99: element not found

# hint
Available elements: 1-6. Run 'observe' to refresh.
---
```

### 6.2 Client ↔ Extension (WebSocket)

**Direction: Client → Extension**

```
{request_id}:{intent_language_command}
```

Examples:
```
1:goto github.com
2:observe
3:click 3
```

**Direction: Extension → Client**

```
{request_id}:{intent_language_response}
```

Examples:
```
1:ok goto github.com

@ github.com "GitHub"
```

```
2:@ github.com "GitHub"

[1] input/search "Search" {focused}
[2] button/submit "Search"
...
```

```
3:ok click 3

# changes
~ url: /search → /results
```

### 6.3 Extension Registration

On connection, the extension sends a registration message. Request ID 0 is reserved for extension-initiated messages.

**Initial Registration**

```
0:register protocol=1 engine=1.0.0 extension=1.0.0 browser=Chrome/120.0
```

**Registration with Pairing Code**

When connecting to a newly-added endpoint that requires pairing:

```
0:register protocol=1 engine=1.0.0 extension=1.0.0 browser=Chrome/120.0 pairing=A7X9K2
```

**Reconnection Registration**

When service worker restarts and reconnects:

```
0:register protocol=1 engine=1.0.0 extension=1.0.0 browser=Chrome/120.0 reconnected=true
```

**Registration Fields**

| Field | Required | Description |
|-------|----------|-------------|
| protocol | Yes | Protocol version number (integer) |
| engine | Yes | WASM engine semver |
| extension | Yes | Extension semver |
| browser | Yes | Browser name and version |
| pairing | No | One-time pairing code for new endpoints |
| reconnected | No | Present and true if this is a reconnection |

**Client Response**

Client SHOULD respond to registration:

```
0:ok
```

Or reject incompatible protocol version:

```
0:error unsupported protocol version 1, require 2+
```

### 6.4 Protocol Versioning

**Protocol Version**

The protocol version is a single integer that increments when breaking changes occur:

| Version | Description |
|---------|-------------|
| 1 | Initial release |
| 2 | Added session management |

Breaking changes that increment protocol version:
- Changing registration format
- Changing message framing
- Removing commands
- Changing command semantics incompatibly

Non-breaking changes that do NOT increment protocol version:
- Adding new commands
- Adding optional fields to responses
- Adding new error codes

**Version Compatibility**

| Client Protocol | Extension Protocol | Compatibility |
|-----------------|-------------------|---------------|
| N | N | ✓ Full compatibility |
| N | N+1 | ✓ Extension is newer, should work |
| N+1 | N | ✗ Client requires features extension lacks |

Clients SHOULD reject connections from extensions with lower protocol versions than required.

**Capabilities (Future)**

Future protocol versions may include a capabilities field in registration:

```
0:register protocol=2 ... capabilities=screenshots,tabs,network,sessions
```

This allows clients to detect optional features without version checks.

### 6.5 Error Responses

Errors follow Intent Language format:

```
{id}:error {command}: {message}

# hint
{recovery_suggestion}
```

Extension-specific errors:

| Error | Cause |
|-------|-------|
| error: no active tab | No tab available to target |
| error: tab not found | Specified tab ID doesn't exist |
| error: cannot access page | Page is restricted (chrome://, etc.) |
| error: scanner not ready | Content script not yet injected |
| error: pairing required | Endpoint requires pairing code |
| error: pairing failed | Pairing code invalid or expired |
| error: session not found | Named session doesn't exist |
| error: not supported in this mode | Feature unavailable in current mode |

---

## 7. Distribution

### 7.1 Client Libraries

| Language | Registry | Package Name |
|----------|----------|--------------|
| Python | PyPI | oryn |
| TypeScript | npm | oryn |

Client libraries are pure Python/TypeScript with no native dependencies. They require the engine binary to be installed separately for embedded/headless modes.

### 7.2 Engine Binaries

| Channel | Command |
|---------|---------|
| Cargo | cargo install oryn |
| Homebrew | brew install oryn |
| GitHub Releases | Download from releases page |
| Linux Packages | apt/dnf repositories (future) |

Binary package provides: oryn-e, oryn-h

### 7.3 Browser Extension

| Browser | Distribution |
|---------|--------------|
| Chrome | Chrome Web Store |
| Firefox | Firefox Add-ons (future) |
| Edge | Edge Add-ons (future) |

Extension includes WASM engine and Scanner.js bundled.

### 7.4 Version Compatibility

| Component | Versioning | Compatibility |
|-----------|------------|---------------|
| Client Library | semver | Compatible with same major version of engine |
| Engine Binary | semver | Protocol version in ready message |
| Extension | semver | Protocol version in registration |
| Protocol | integer | Breaking changes increment; see §7.4 |

Clients and extensions negotiate protocol version during connection. See section 7.4 for detailed compatibility rules.

---

## 8. Implementation Notes

### 8.1 Why Subprocess Over FFI

The decision to use subprocess rather than language bindings (FFI) is deliberate:

**Latency is Irrelevant**

Subprocess IPC adds ~0.1-1ms latency. Browser operations take 100-2000ms. The overhead is noise.

**Build Simplicity**

Subprocess: One Rust binary, compiles on all platforms.
FFI: Per-language bindings, per-platform builds, complex CI.

**Deployment Simplicity**

Subprocess: Pure Python/TS package + binary in PATH.
FFI: Native extensions bundled in package, platform-specific wheels.

**Crash Isolation**

Subprocess: Engine crash returns error, agent continues.
FFI: Engine crash kills entire agent process.

**Independent Updates**

Subprocess: Update binary or client independently.
FFI: Tight coupling, must update together.

### 8.2 Why Extension Connects Out

The extension acts as WebSocket client (connecting to the user's server) rather than WebSocket server (accepting connections) because:

**Technical Feasibility**

Browser extensions cannot bind to network ports. They can only make outbound connections.

**Firewall Compatibility**

Outbound connections typically allowed. Inbound connections often blocked.

**Simpler Discovery**

Extension connects to known endpoint. No need to discover where extension is listening.

**User Control**

User explicitly configures where their extension connects. No implicit network exposure.

### 8.3 Why WASM Engine in Extension

The Intent Language engine runs as WASM in the extension rather than server-side because:

**Minimal Server Requirements**

If the server must process Intent Language, it must run Oryn. With WASM in extension, the server just forwards strings.

**Consistent Behavior**

Same WASM engine used regardless of what language/framework the client uses.

**Offline Capability**

Extension could potentially work with local-only WebSocket server (localhost), no internet required.

**Simplified Protocol**

Client sends Intent Language, receives Intent Language. No need for intermediate representation.

---

## 9. Future Considerations

### 9.1 Connection Pooling

For high-throughput scenarios, clients may want multiple concurrent connections. The protocol's request ID mechanism supports this—multiple commands can be in flight, responses correlated by ID.

### 9.2 Streaming Observations

For real-time DOM monitoring, a streaming mode could be added:

```
observe --stream
```

The extension would send incremental updates as the DOM changes, rather than requiring repeated observe commands.

### 9.3 Multi-Extension Support

A single client server could accept connections from multiple extension instances (different browsers, different machines). The registration message includes enough information to distinguish instances.

### 9.4 Binary Bundling

For simplified deployment, optional packages could bundle the engine binary:

- oryn-bin (Python): Contains platform-specific binary
- @oryn/bin (npm): Contains platform-specific binary

These would be separate packages to keep the core client library pure and lightweight.

---

## 10. Summary

### 10.1 Client Library

- Thin transport layer, not abstraction layer
- Subprocess for embedded/headless modes
- WebSocket server for remote mode
- Single core method: send(command) → response
- Optional typed convenience methods
- Handles delimiter escaping for `---` in content
- Session management via --session flag or commands

### 10.2 Remote Extension

- Extension connects as WebSocket client to user's endpoint
- WASM engine (parser, resolver, formatter) runs in extension
- Scanner.js handles DOM interaction
- Service worker manages connection and tab routing
- User controls where extension connects
- Pairing authentication for new endpoints
- Handles MV3 service worker lifecycle (suspension, restart)

### 10.3 Key Design Decisions

- Subprocess over FFI: simplicity, isolation, negligible latency cost
- Extension as client: technical necessity, better security model
- WASM engine in extension: minimal server requirements, consistent behavior
- Thin client libraries: Intent Language is the abstraction
- Protocol versioning: explicit version negotiation, capability detection
- Security: endpoint allowlist, pairing codes, wss for non-localhost
- Named sessions: full isolation for parallel agent workflows

---

*Document Version: 1.1*
*Last Updated: January 2026*
