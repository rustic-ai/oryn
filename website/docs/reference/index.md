# Reference

Complete reference documentation for Oryn.

## Overview

This section contains detailed reference documentation for Oryn commands, protocols, and configuration, including current implementation status.

<div class="grid cards" markdown>

-   :material-text-box:{ .lg .middle } **Intent Commands**

    ---

    Current syntax and support notes for unified Intent Language commands.

    [:octicons-arrow-right-24: Intent Commands](intent-commands.md)

-   :material-code-json:{ .lg .middle } **Scanner Commands**

    ---

    Low-level scanner protocol reference.

    [:octicons-arrow-right-24: Scanner Commands](scanner-commands.md)

-   :material-table-large:{ .lg .middle } **Command Coverage**

    ---

    End-to-end implementation status across parser, translator, and executor stages.

    [:octicons-arrow-right-24: Command Coverage](command-coverage.md)

-   :material-shield-check:{ .lg .middle } **Truth & Trust**

    ---

    Verification workflow and source-of-truth mapping for keeping docs aligned with implementation.

    [:octicons-arrow-right-24: Truth & Trust](truth-and-trust.md)

-   :material-cog:{ .lg .middle } **Configuration**

    ---

    All configuration options and settings.

    [:octicons-arrow-right-24: Configuration](configuration.md)

-   :material-alert-circle:{ .lg .middle } **Error Codes**

    ---

    Error types, codes, and recovery strategies.

    [:octicons-arrow-right-24: Error Codes](error-codes.md)

-   :material-book-alphabet:{ .lg .middle } **Glossary**

    ---

    Terminology and definitions.

    [:octicons-arrow-right-24: Glossary](glossary.md)

</div>

## Quick Reference

### Most Used Commands

| Command | Description | Example |
|---------|-------------|---------|
| `goto <url>` | Navigate to URL | `goto google.com` |
| `observe` | List elements | `observe` |
| `click <target>` | Click element | `click "Login"` |
| `type <target> <text>` | Type text | `type email "user@test.com"` |
| `login <user> <pass>` | Login intent | `login "user" "pass"` |
| `search <query>` | Search intent | `search "topic"` |
| `wait <condition>` | Wait for condition | `wait visible "Success"` |

### Target Types

| Type | Syntax | Example |
|------|--------|---------|
| ID | Number | `click 5` |
| Text | Quoted string | `click "Sign in"` |
| Role | Role name | `type email "..."` |
| Selector | `css()` or `xpath()` | `click css(".btn")` |
| Relational | `near`, `inside` | `click "Edit" near "Item"` |

### Built-in Intents

| Intent | Syntax |
|--------|--------|
| `login` | `login <user> <pass>` |
| `search` | `search <query>` |
| `accept_cookies` | `accept_cookies` |
| `dismiss` | `dismiss popups` |
