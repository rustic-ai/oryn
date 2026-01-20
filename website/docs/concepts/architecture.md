# Architecture

Oryn's architecture is designed around a core insight: **the scanner is the source of truth**. All HTML understanding happens in JavaScript inside the browser; the Rust layer never parses HTML directly.

## System Layers

```mermaid
graph TB
    subgraph "Agent Layer"
        A[AI Agent]
    end

    subgraph "Protocol Layer"
        B[Intent Parser]
        C[Intent Engine]
        D[Semantic Resolver]
    end

    subgraph "Backend Layer"
        E[Backend Trait]
        F[oryn-h CDP]
        G[oryn-e WebDriver]
        H[oryn-r WebSocket]
    end

    subgraph "Scanner Layer"
        I[Universal Scanner JS]
    end

    subgraph "Browser Layer"
        J[Chromium]
        K[WPE WebKit]
        L[User Browser]
    end

    A -->|Intent Commands| B
    B --> C
    C --> D
    D --> E
    E --> F
    E --> G
    E --> H
    F -->|CDP| J
    G -->|WebDriver| K
    H -->|Extension| L
    J --> I
    K --> I
    L --> I
```

## Agent Layer

AI agents communicate with Oryn using the Intent Language. They receive observations, make decisions, and issue commands. Agents never interact with raw browser APIs or HTML.

**Key Properties:**
- Token-efficient command syntax
- Semantic observations instead of raw HTML
- Multi-level abstraction (direct, semantic, intent)

## Protocol Layer

### Intent Parser

Interprets agent commands with forgiveness for variations:
- Case-insensitive commands
- Multiple quote styles
- Command aliases (e.g., `goto` = `navigate` = `go to`)
- Flexible option syntax

### Intent Engine

Transforms high-level intents into atomic operations:
- Built-in intents (login, search, accept_cookies, etc.)
- Loaded intents from YAML definitions
- Multi-step execution with error handling
- Success/failure verification

### Semantic Resolver

Translates targets to concrete elements:
- Text matching (`"Sign in"`)
- Role matching (`email`, `password`, `submit`)
- Pattern references (`login_form.email`)
- Fallback chains

## Backend Layer

### Backend Trait

A unified Rust trait that all backends implement:

```rust
pub trait Backend {
    async fn navigate(&mut self, url: &str) -> Result<()>;
    async fn execute_scanner(&mut self, cmd: ScannerCommand) -> Result<ScannerResponse>;
    async fn screenshot(&mut self) -> Result<Vec<u8>>;
    async fn close(&mut self) -> Result<()>;
}
```

This abstraction ensures agents can switch modes without changing their logic.

### oryn-h (Headless)

- **Technology**: Chrome DevTools Protocol (CDP)
- **Transport**: WebSocket
- **Library**: chromiumoxide
- **Best for**: Cloud automation, CI/CD, maximum compatibility

### oryn-e (Embedded)

- **Technology**: WebDriver protocol
- **Transport**: HTTP
- **Library**: fantoccini
- **Best for**: IoT, containers, low-memory environments

### oryn-r (Remote)

- **Technology**: Custom WebSocket protocol
- **Transport**: WebSocket
- **Client**: Browser extension
- **Best for**: User assistance, authenticated sessions, debugging

## Scanner Layer

The Universal Scanner is a JavaScript module injected into all browser contexts:

```javascript
// Simplified scanner interface
const scanner = {
  scan: () => { /* Returns all interactive elements */ },
  click: (id) => { /* Clicks element by ID */ },
  type: (id, text) => { /* Types into element */ },
  // ... more commands
};
```

**Key Properties:**
- Byte-for-byte identical across all backends
- JSON request/response protocol
- Maintains element map for efficient targeting
- Detects UI patterns automatically

## Data Flow

### Command Execution

```mermaid
sequenceDiagram
    participant Agent
    participant Parser
    participant Engine
    participant Backend
    participant Scanner
    participant Browser

    Agent->>Parser: click "Sign in"
    Parser->>Engine: ClickCommand(text="Sign in")
    Engine->>Backend: scan()
    Backend->>Scanner: {cmd: "scan"}
    Scanner->>Browser: Query DOM
    Browser-->>Scanner: Element data
    Scanner-->>Backend: {ok: true, elements: [...]}
    Backend-->>Engine: Elements list
    Engine->>Engine: Resolve "Sign in" → ID 5
    Engine->>Backend: click(5)
    Backend->>Scanner: {cmd: "click", id: 5}
    Scanner->>Browser: Click element
    Browser-->>Scanner: Success
    Scanner-->>Backend: {ok: true}
    Backend-->>Engine: Success
    Engine-->>Parser: ClickResult
    Parser-->>Agent: ok click [5]
```

### Observation Flow

```mermaid
sequenceDiagram
    participant Agent
    participant Oryn
    participant Scanner
    participant Browser

    Agent->>Oryn: observe
    Oryn->>Scanner: {cmd: "scan"}
    Scanner->>Browser: Query interactive elements
    Browser-->>Scanner: DOM elements
    Scanner->>Scanner: Classify types, roles
    Scanner->>Scanner: Detect patterns
    Scanner-->>Oryn: {elements: [...], patterns: {...}}
    Oryn->>Oryn: Format for agent
    Oryn-->>Agent: Structured observation
```

## Design Principles

### Scanner as Source of Truth

> **HTML parsing should NOT happen in Rust.**

If HTML were parsed differently per backend, behavior would diverge. The Universal Scanner eliminates this risk by ensuring all DOM understanding happens in JavaScript.

### Consistency Guarantees

Given the same page state and the same command:
- oryn-e, oryn-h, and oryn-r produce identical observations
- oryn-e, oryn-h, and oryn-r execute identical actions
- oryn-e, oryn-h, and oryn-r report identical results

### Separation of Concerns

| Layer | Responsibility |
|-------|----------------|
| Agent | Decision-making, goal pursuit |
| Protocol | Command parsing, intent execution |
| Backend | Browser communication |
| Scanner | DOM understanding, element classification |
| Browser | HTML rendering, JavaScript execution |

## Resource Comparison

| Mode | RAM | Binary Size | Notes |
|------|-----|-------------|-------|
| oryn-h | ~300MB+ | ~15MB | Chrome installed separately |
| oryn-e | ~50MB | ~15MB | WPE WebKit libraries |
| oryn-r | Zero server | ~15MB | Runs in user's browser |
