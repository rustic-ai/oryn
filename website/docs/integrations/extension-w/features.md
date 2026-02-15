# Oryn-W Features and Functionality

This page documents the major capabilities shipped in `extension-w/`.

## Core Runtime

### Client-Side OIL Execution

- Runs fully in-browser as a Manifest V3 extension.
- Uses `oryn-core` compiled to WASM for command parsing and translation.
- Executes against live pages via `scanner.js` and content scripts.

### Shared Scanner Behavior

- Uses the same scanner source as other Oryn modes (`crates/oryn-scanner/src/scanner.js`).
- Supports page observation and action execution against discovered elements.
- Supports dynamic content and pattern detection via scanner output.

## User Interfaces

### Popup

- Fast command entry and immediate execution.
- Basic status checks for extension/runtime readiness.

### Sidepanel

- Rich command and result logs.
- Status widgets for WASM and LLM state.
- OIL mode and Agent mode switch.

## LLM and Agent Capabilities

### LLM Adapter Manager

Registered adapters:

- `chrome-ai`
- `webllm`
- `wllama`
- `openai`
- `claude`
- `gemini`

Behavior:

- Auto-detects adapter availability.
- Supports switching adapters/models.
- Persists configuration in extension storage.
- Defers initialization for dynamic-import adapters when running in service worker context.

### Ralph Agent Mode

- Task-driven iterative planner/executor in the extension.
- Uses retrieved trajectories as few-shot context.
- Includes retry logic and simple loop detection.
- Tracks command history and current iteration state.

### Trajectory Store

- Stores and retrieves trajectories for agent assistance.
- Supports stats/export/import/delete/clear operations through extension messages.
- Seeds default trajectories for common web flows.

## Hardware and Local Inference Support

- Hardware profile detection utilities for adapter guidance.
- Compatibility checks and recommendations for local/remote adapters.

## Build and Packaging Features

- `./scripts/build-extension-w.sh`:
  - scanner sync
  - WASM build
  - LLM vendor bundling
  - artifact sanity checks
- `./scripts/pack-extension-w.sh`:
  - creates distributable zip
  - writes checksum and package metadata in `dist/`

## Security and Permission Model

The extension currently uses:

- permissions: `activeTab`, `scripting`, `tabs`, `storage`, `sidePanel`, `offscreen`
- host permissions: `<all_urls>` plus provider endpoints for supported remote LLMs
- extension page CSP includes `'wasm-unsafe-eval'` for WASM initialization

## Current Constraints

- Best supported in Chromium-family browsers.
- Some pages do not allow content scripts (`chrome://`, Web Store, other restricted origins).
- Remote LLM adapters require API keys and network access.
