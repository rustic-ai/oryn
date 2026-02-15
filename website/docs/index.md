# Oryn

<div class="hero" markdown>

## The Browser Designed for AI Agents

**Oryn** (Open Runtime for Intentful Navigation) is a browser automation system designed specifically for AI agents. Instead of forcing agents to understand screenshots, parse HTML, or construct complex function calls, Oryn provides a semantic intent language that speaks naturally to how agents think about web interaction.

<div class="quick-links" markdown>
<a href="getting-started/installation/" class="quick-link">Installation</a>
<a href="getting-started/quickstart/" class="quick-link">Quick Start</a>
<a href="reference/intent-commands/" class="quick-link">Command Reference</a>
<a href="https://github.com/dragonscale/oryn" class="quick-link">GitHub</a>
</div>

</div>

---

## Why Oryn?

Current approaches to browser automation for AI agents fall into predictable failure patterns:

| Approach | Problem |
|----------|---------|
| **Screenshot/Vision** | Expensive inference, unreliable text extraction, no understanding of interactive state |
| **HTML Parsing** | Thousands of tokens of markup, complex reasoning about visibility and interactivity |
| **Function Calls** | Rigid schemas, verbose definitions, no tolerance for natural variation |

**Oryn solves this by design:**

| Capability | Description |
|------------|-------------|
| **Semantic Observations** | Structured descriptions of interactive elements with meaningful labels, types, roles, and states |
| **Intent Language** | Natural, forgiving syntax for expressing actions at the appropriate level of abstraction |
| **Consistent Behavior** | Identical semantics across embedded, headless, and remote modes |
| **Token Efficient** | Minimal verbosity means more context for agent reasoning |

---

## Key Features

<div class="feature-grid" markdown>

<div class="feature-card" markdown>

### Three Deployment Modes
A single unified binary adapts to any environment: embedded IoT devices, headless cloud servers, or browser extensions for user assistance.

</div>

<div class="feature-card" markdown>

### Semantic Targeting
Agents can reference elements by meaning rather than implementation. Say "click login" instead of hunting for CSS selectors.

</div>

<div class="feature-card" markdown>

### Pattern Detection
Common UI patterns (login forms, search boxes, cookie banners) are automatically recognized and reported to agents.

</div>

<div class="feature-card" markdown>

### Intent Engine
High-level intents like `login`, `search`, and `accept_cookies` encapsulate common workflows, expandable via YAML definitions.

</div>

</div>

### Quick Example

Instead of parsing HTML or analyzing screenshots, agents interact naturally:

```
goto github.com/login
observe

@ github.com/login "Sign in to GitHub"
[1] input/email "Username or email" {required}
[2] input/password "Password" {required}
[3] button/submit "Sign in" {primary}

type 1 "myusername"
type 2 "mypassword"
click 3
```

The agent sees labeled interactive elements and issues simple commands. No CSS selectors, no XPath, no DOM traversal—just intent.

---

## Architecture Overview

Oryn's layered architecture separates concerns for maximum consistency and flexibility:

```
┌─────────────────────────────────────────────────────────────┐
│                      AI Agent                                │
│  (Issues intent commands: login, search, click)             │
├─────────────────────────────────────────────────────────────┤
│                    Oryn CLI / Protocol                       │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
│  │ Intent      │  │   Intent    │  │   Scanner           │  │
│  │ Parser      │→ │   Engine    │→ │   Interface         │  │
│  └─────────────┘  └─────────────┘  └──────────┬──────────┘  │
└─────────────────────────────────────────────────┼────────────┘
                                                  │
                    ┌─────────────────────────────┴─────────────────────────────┐
                    │                    Browser Backend                         │
                    │     ┌──────────┐    ┌──────────┐    ┌──────────┐          │
                    │     │ oryn-e   │    │ oryn-h   │    │ oryn-r   │          │
                    │     │ Embedded │    │ Headless │    │ Remote   │          │
                    │     └──────────┘    └──────────┘    └──────────┘          │
                    └───────────────────────────────────────────────────────────┘
```

**[Learn more about the architecture →](concepts/architecture.md)**

---

## The Three Modes

| Mode | Binary | Engine | Best For |
|------|--------|--------|----------|
| **Embedded** | oryn-e | WPE WebKit | IoT, containers, edge (~50MB RAM) |
| **Headless** | oryn-h | Chromium | Cloud automation, CI/CD (~99% compatibility) |
| **Remote** | oryn-r | User's Browser | User assistance, authenticated sessions |

All three modes share the same protocol, intent language, and Universal Scanner—ensuring consistent behavior regardless of deployment environment.

---

## Quick Start

Get up and running with Oryn:

```bash
# Clone and build
git clone https://github.com/dragonscale/oryn.git
cd oryn
cargo build --release -p oryn

# Run in headless mode
./target/release/oryn headless

# In the REPL, navigate and observe
> goto example.com
> observe
> click "More information..."
```

**[Complete Quick Start Guide →](getting-started/quickstart.md)**

---

## Documentation

### Getting Started
- **[Installation](getting-started/installation.md)** — Build from source, prerequisites, verification
- **[Quick Start](getting-started/quickstart.md)** — Your first web automation in minutes
- **[CLI Reference](getting-started/cli-reference.md)** — Complete command-line documentation

### Core Concepts
- **[Architecture](concepts/architecture.md)** — System design and component overview
- **[Intent Language](concepts/intent-language.md)** — Agent-facing command protocol
- **[Scanner Protocol](concepts/scanner-protocol.md)** — Internal browser-scanner interface
- **[Intent Engine](concepts/intent-engine.md)** — High-level intent execution
- **[Backend Modes](concepts/backend-modes.md)** — Embedded, headless, and remote modes

### Developer Guides
- **[Basic Navigation](guides/basic-navigation.md)** — Navigating and observing pages
- **[Form Interactions](guides/form-interactions.md)** — Filling and submitting forms
- **[Custom Intents](guides/custom-intents.md)** — Defining your own intent commands
- **[Multi-Page Flows](guides/multi-page-flows.md)** — Workflows spanning multiple pages
- **[Troubleshooting](guides/troubleshooting.md)** — Common issues and solutions

### Integrations
- **[Google ADK](integrations/google-adk.md)** — Using Oryn with Google ADK agents
- **[IntentGym](integrations/intentgym.md)** — Benchmark harness for evaluating Oryn-based web agents
- **[Python SDK](integrations/python-sdk.md)** — Sync/async Python client for OIL command execution
- **[Remote Extension](integrations/remote-extension.md)** — Connect `oryn remote` to the browser extension
- **[WASM Extension](integrations/wasm-extension.md)** — Building and running the standalone extension-w workflow

### Reference
- **[Intent Commands](reference/intent-commands.md)** — Complete command syntax reference
- **[Command Coverage](reference/command-coverage.md)** — Parser/translator/executor implementation status
- **[Truth & Trust](reference/truth-and-trust.md)** — Verification workflow and source-of-truth map
- **[Scanner Commands](reference/scanner-commands.md)** — Low-level scanner protocol
- **[Configuration](reference/configuration.md)** — All configuration options
- **[Error Codes](reference/error-codes.md)** — Error types and recovery
- **[Glossary](reference/glossary.md)** — Terminology reference

---

## Project Status

Oryn is under active development. Current status:

| Feature | Status |
|---------|--------|
| Intent Language Parser | <span class="badge badge-new">Stable</span> |
| Universal Scanner Runtime | <span class="badge badge-new">Stable</span> |
| Headless Mode (`oryn-h`) | <span class="badge badge-new">Stable</span> |
| Embedded Mode (`oryn-e`) | <span class="badge badge-experimental">Partial</span> |
| Remote Mode (`oryn-r`) | <span class="badge badge-experimental">Partial</span> |
| Unified Command End-to-End Coverage | <span class="badge badge-experimental">Partial</span> |
| Built-in Intent Commands in Unified CLI (`login`, `search`, `dismiss`, `accept_cookies`) | <span class="badge badge-new">Stable</span> |
| Declarative Intent/Pack Management Commands (`intents`, `define`, `run`, ...) | <span class="badge badge-experimental">Stubbed</span> |
| Multi-step Automation via `.oil` Scripts | <span class="badge badge-new">Stable</span> |

---

## Contributing

We welcome contributions! See the Contributing Guide for details.

```bash
# Run tests
./scripts/run-tests.sh

# Run E2E tests
./scripts/run-e2e-tests.sh

# Check formatting and lints
cargo fmt --check && cargo clippy --workspace
```

---

## License

Oryn is open source under the Apache 2.0 License.

---

<div class="footer-nav">

**Ready to dive in?** Start with the **[Installation Guide](getting-started/installation.md)** or jump straight to the **[Quick Start](getting-started/quickstart.md)**.

</div>
