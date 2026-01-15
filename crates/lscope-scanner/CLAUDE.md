# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**lscope-scanner** is the Universal Scanner module for Lemmascope, a browser automation system for AI agents. The scanner is injected into browser contexts (WebKit, Chromium, browser extensions) to discover interactive elements and execute user actions via a JSON protocol.

This crate is primarily JavaScript with a thin Rust wrapper that embeds the scanner for injection.

## Build and Test Commands

```bash
# Run JavaScript tests (Jest + Puppeteer)
npm test

# Install dependencies
npm install

# Run Rust tests (from workspace root or crate directory)
cargo test
```

## Architecture

### Two-Layer Design

1. **Rust Layer** (`src/lib.rs`) - Minimal wrapper that embeds `scanner.js` as a string constant (`SCANNER_JS`) for injection into browser contexts

2. **JavaScript Layer** (`src/scanner.js`) - Core implementation with these modules:
   - **STATE**: Global element tracking (`elementMap`, `inverseMap`)
   - **Scanner**: Page scanning via `scan()` - discovers and serializes interactive elements
   - **Executor**: User actions (`click`, `type`, `clear`, `check/uncheck`, `select`, `scroll`, `focus`, `hover`, `submit`, `wait_for`)
   - **Extractor**: Data retrieval (`get_text`, `get_value`, `exists`, `execute`)

### Command Protocol

All commands flow through `window.Lemmascope.process(message)` where `message.cmd` determines the action. The scanner returns JSON responses with `{ok: true/false, data?: ..., error?: ...}`.

### Element Map Lifecycle

- `scan` clears the previous element map and assigns sequential numeric IDs to interactive elements
- Action commands reference elements by these IDs
- The map becomes stale on navigation or DOM changes; agents must re-scan

## Test Setup

- **Framework**: Jest 29.7 with Puppeteer 23
- **Test file**: `tests/scanner.test.js`
- **Test harness**: `tests/harness.html` - HTML fixture with forms, buttons, visibility edge cases

Tests launch a real browser via Puppeteer and verify scanner behavior end-to-end.

## Key Specifications

Read these docs in the parent workspace for protocol details:
- `docs/SPEC-SCANNER-PROTOCOL.md` - Scanner JSON protocol (Version 1.0)
- `docs/SPEC-INTENT-LANGUAGE.md` - Agent-facing command language

## Cross-Platform Considerations

The same JavaScript must work identically across WebKit (lscope-e), Chromium (lscope-h), and browser extensions (lscope-r). Browser compatibility is critical—avoid browser-specific APIs.
