# Lemmascope

**The browser built for AI agents.**

---

## What is Lemmascope?

Lemmascope is a browser automation system designed specifically for AI agents. Instead of forcing agents to understand screenshots, parse HTML, or construct complex function calls, Lemmascope provides a semantic intent language that speaks naturally to how agents think about web interaction.

Traditional agent-browser interfaces fail because they expose the wrong abstraction. Lemmascope fixes this by providing:

- **Semantic observations** instead of raw HTML or pixels
- **Intent language** instead of rigid function schemas  
- **Consistent behavior** across all deployment environments

## The Three Binaries

Lemmascope ships as three specialized binaries for different environments:

| Binary | Name | Use Case |
|--------|------|----------|
| `lscope-e` | Embedded | IoT, containers, edge devices |
| `lscope-h` | Headless | Cloud automation, CI/CD, scraping |
| `lscope-r` | Remote | User assistance, authenticated sessions |

All three binaries use the same protocol and produce identical behavior for the same inputs.

## Quick Example

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

## Why Lemmascope?

### The Problem

Current approaches force agents into unnatural workflows:

**Screenshot/Vision**: Expensive inference, unreliable text extraction, no understanding of interactive state

**HTML Parsing**: Thousands of tokens of markup, complex reasoning about visibility and interactivity

**Function Calls**: Rigid schemas, verbose definitions, no tolerance for natural variation

### The Solution

Lemmascope presents the web as agents need to see it:

- Interactive elements are labeled and typed
- State is explicit (required, disabled, checked)
- Patterns are detected (login forms, search boxes)
- Commands are forgiving and semantic

## Mode Selection Guide

**Choose lscope-e (Embedded) when:**
- Running on Raspberry Pi or IoT hardware
- Deploying to resource-constrained containers
- RAM is precious (~50MB footprint)
- WebKit compatibility is sufficient

**Choose lscope-h (Headless) when:**
- Running cloud-based automation
- Maximum browser compatibility needed (~99%)
- Handling complex SPAs
- Network interception required

**Choose lscope-r (Remote) when:**
- Agent needs user's logged-in sessions
- Anti-bot bypass is critical (real browser fingerprint)
- User wants to watch agent actions
- Interactive assistance workflows

## Documentation

| Document | Description |
|----------|-------------|
| [SPEC-INTENT-LANGUAGE.md](docs/SPEC-INTENT-LANGUAGE.md) | The agent-facing command protocol |
| [SPEC-SCANNER-PROTOCOL.md](docs/SPEC-SCANNER-PROTOCOL.md) | Internal browser-scanner interface |
| [SPEC-UNIFIED.md](docs/SPEC-UNIFIED.md) | Architecture and mode comparison |
| [PRODUCT-INTRO.md](docs/PRODUCT-INTRO.md) | Product overview and vision |

## Project Structure

```
lemmascope/
├── crates/
│   ├── lscope-core/          # Shared protocol and types
│   ├── lscope-scanner/       # Universal JavaScript scanner
│   ├── lscope-e/             # Embedded mode binary
│   ├── lscope-h/             # Headless mode binary
│   └── lscope-r/             # Remote mode binary
├── extension/                 # Browser extension for remote mode
├── docs/
│   ├── SPEC-INTENT-LANGUAGE.md
│   ├── SPEC-SCANNER-PROTOCOL.md
│   └── SPEC-UNIFIED.md
└── README.md
```

## Core Concepts

### Universal Scanner

A single JavaScript implementation runs inside all browser contexts—WebKit, Chromium, and browser extensions. This guarantees behavioral consistency. The Rust layer never parses HTML directly; it only processes scanner JSON responses.

### Intent Language

Commands are designed for agent ergonomics:
- Case-insensitive, forgiving syntax
- Multiple targeting strategies (ID, text, role, selector)
- Multi-level abstraction (direct → semantic → intent)
- Token-efficient responses

### Backend Trait

All binaries implement the same interface. Agents can switch modes without changing their logic. The same scanner code, the same protocol, the same behavior.

## Requirements

**lscope-e (Embedded)**
- Linux with WPE WebKit / COG
- ~50MB RAM available

**lscope-h (Headless)**
- Chromium browser installed
- ~300MB+ RAM available

**lscope-r (Remote)**
- Browser extension installed
- WebSocket connection to server

## License

[License details to be determined]

## Contributing

[Contributing guidelines to be determined]

---

*Lemmascope: Intent, not implementation.*
