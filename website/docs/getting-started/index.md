# Getting Started

Welcome to Oryn! This section will help you get up and running with the browser designed for AI agents.

## What is Oryn?

Oryn (Open Runtime for Intentful Navigation) is a browser automation system designed specifically for AI agents. Traditional approaches force agents into unnatural workflows:

- **Screenshot/Vision**: Expensive inference, unreliable text extraction
- **HTML Parsing**: Thousands of tokens of irrelevant markup
- **Function Calls**: Rigid schemas, no tolerance for variation

Oryn provides a semantic intent language that matches how agents think about web interaction.

## Quick Overview

### The Unified CLI

Oryn provides a single binary that adapts to any environment:

```bash
# Headless Mode (Cloud/CI)
oryn headless

# Embedded Mode (IoT/Edge)
oryn embedded --driver-url http://localhost:8080

# Remote Mode (Debug/Assistance)
oryn remote --port 9001
```

### Semantic Observations

Instead of raw HTML, agents see structured, labeled elements:

```
@ github.com/login "Sign in to GitHub"
[1] input/email "Username or email" {required}
[2] input/password "Password" {required}
[3] button/submit "Sign in" {primary}
```

### Intent Commands

Agents express actions naturally:

```
type email "user@example.com"
click "Sign in"
```

## Next Steps

<div class="grid cards" markdown>

-   :material-download:{ .lg .middle } **Installation**

    ---

    Build Oryn from source and verify your installation.

    [:octicons-arrow-right-24: Installation Guide](installation.md)

-   :material-rocket-launch:{ .lg .middle } **Quick Start**

    ---

    Run your first web automation in minutes.

    [:octicons-arrow-right-24: Quick Start](quickstart.md)

-   :material-console:{ .lg .middle } **CLI Reference**

    ---

    Complete command-line documentation.

    [:octicons-arrow-right-24: CLI Reference](cli-reference.md)

</div>

## Choose Your Mode

| Mode | When to Use |
|------|-------------|
| **Headless** (`oryn headless`) | Cloud automation, CI/CD, web scraping |
| **Embedded** (`oryn embedded`) | IoT devices, containers, edge computing |
| **Remote** (`oryn remote`) | User assistance, debugging, authenticated sessions |

All modes share the same protocol and behavior—agents can switch modes without changing their logic.
