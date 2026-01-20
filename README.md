# Oryn

**The browser built for AI agents.**

---

## What is Oryn?

Oryn (Open Runtime for Intentful Navigation) is a browser automation system designed specifically for AI agents. Instead of forcing agents to understand screenshots, parse HTML, or construct complex function calls, Oryn provides a semantic intent language that speaks naturally to how agents think about web interaction.

Traditional agent-browser interfaces fail because they expose the wrong abstraction. Oryn fixes this by providing:

- **Semantic observations** instead of raw HTML or pixels
- **Intent language** instead of rigid function schemas  
- **Consistent behavior** across all deployment environments

## The Unified CLI
Oryn provides a single unified binary `oryn` that adapts to any environment:

```bash
# Headless Mode (Cloud/CI)
oryn headless

# Embedded Mode (IoT/Edge)
oryn embedded --driver-url http://localhost:8080

# Remote Mode (Debug/Assistance)
oryn remote --port 9001
```

See the [User Guide](docs/USER_GUIDE.md) for full usage instructions.

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

## Why Oryn?

### The Problem

Current approaches force agents into unnatural workflows:

**Screenshot/Vision**: Expensive inference, unreliable text extraction, no understanding of interactive state

**HTML Parsing**: Thousands of tokens of markup, complex reasoning about visibility and interactivity

**Function Calls**: Rigid schemas, verbose definitions, no tolerance for natural variation

### The Solution

Oryn presents the web as agents need to see it:

- Interactive elements are labeled and typed
- State is explicit (required, disabled, checked)
- Patterns are detected (login forms, search boxes)
- Commands are forgiving and semantic

## Mode Selection Guide

**Choose oryn-e (Embedded) when:**
- Running on Raspberry Pi or IoT hardware
- Deploying to resource-constrained containers
- RAM is precious (~50MB footprint)
- WebKit compatibility is sufficient

**Choose oryn-h (Headless) when:**
- Running cloud-based automation
- Maximum browser compatibility needed (~99%)
- Handling complex SPAs
- Network interception required

**Choose oryn-r (Remote) when:**
- Agent needs user's logged-in sessions
- Anti-bot bypass is critical (real browser fingerprint)
- User wants to watch agent actions
- Interactive assistance workflows

## Documentation

| Document                                                  | Description                        |
| --------------------------------------------------------- | ---------------------------------- |
| [USER_GUIDE.md](docs/USER_GUIDE.md)                       | Full installation and usage guide  |
| [GOOGLE_ADK_TUTORIAL.md](docs/GOOGLE_ADK_TUTORIAL.md)     | Integration with Google ADK Agents |
| [SPEC-INTENT-LANGUAGE.md](docs/SPEC-INTENT-LANGUAGE.md)   | The agent-facing command protocol  |
| [SPEC-SCANNER-PROTOCOL.md](docs/SPEC-SCANNER-PROTOCOL.md) | Internal browser-scanner interface |
| [SPEC-UNIFIED.md](docs/SPEC-UNIFIED.md)                   | Architecture and mode comparison   |
| [PRODUCT-INTRO.md](docs/PRODUCT-INTRO.md)                 | Product overview and vision        |

## Project Structure

```
oryn/
├── crates/
│   ├── oryn-core/          # Shared protocol and types
│   ├── oryn-scanner/       # Universal JavaScript scanner
│   ├── oryn-e/             # Embedded mode binary
│   ├── oryn-h/             # Headless mode binary
│   └── oryn-r/             # Remote mode binary
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

**oryn-e (Embedded)**
- Linux with WPE WebKit / COG
- ~50MB RAM available

**oryn-h (Headless)**
- Chromium browser installed
- ~300MB+ RAM available

**oryn-r (Remote)**
- Browser extension installed
- WebSocket connection to server

## License
[MIT](https://choosealicense.com/licenses/mit/)

## Contributing
Pull requests are welcome. For major changes, please open an issue first to discuss what you would like to change.

Please make sure to update tests as appropriate.

---

*Oryn: Intent, not implementation.*
