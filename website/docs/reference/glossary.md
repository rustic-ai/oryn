# Glossary

Terminology and definitions used in Oryn documentation.

## A

### Agent
An AI system that uses Oryn to interact with web pages. Agents receive observations, make decisions, and issue intent commands.

## B

### Backend
The component that communicates with the browser. Oryn has three backends:
- **oryn-e**: Embedded (WebDriver/WebKit)
- **oryn-h**: Headless (CDP/Chromium)
- **oryn-r**: Remote (WebSocket/Extension)

### Backend Trait
The unified Rust interface that all backends implement, ensuring consistent behavior across modes.

## C

### CDP (Chrome DevTools Protocol)
The protocol used by oryn-h to communicate with Chromium browsers.

### COG
A WPE WebKit browser used by oryn-e in embedded mode.

### Checkpoint
An engine/design concept for resumable multi-step intents. Not currently exposed as a stable unified CLI command feature.

## D

### Direct Command
A command that operates on element IDs directly (e.g., `click 5`).

## E

### Element
An interactive item on a web page (input, button, link, etc.) that Oryn can observe and interact with.

### Element ID
A numeric identifier assigned to each interactive element during a scan. IDs are session-scoped and change after navigation.

### Element Map
The internal mapping of element IDs to DOM elements maintained by the scanner.

### Element Type
Classification of an element: input, button, link, select, textarea, checkbox, radio, generic.

## F

### Flow
A multi-page automation concept. In unified CLI today, the practical form is `.oil` scripts rather than declarative flow definitions.

## I

### Intent
A high-level command that encapsulates a workflow (e.g., `login`, `search`, `accept_cookies`).

### Intent Command
A Level 3 command in the abstraction hierarchy that executes multiple atomic operations.

### Intent Engine
The component that transforms high-level intent commands into sequences of atomic scanner operations.

### Intent Language (OIL)
The Oryn Intent Language - the token-efficient, human-readable protocol for agent-browser communication.

### Intent Pack
A collection of site-specific intents and patterns organized for a particular domain.

## M

### Modifier
A state flag on an element: `required`, `disabled`, `readonly`, `hidden`, `primary`, `checked`, `focused`.

## O

### Observation
The structured output from scanning a page, including elements, patterns, and page metadata.

### OIL (Oryn Intent Language)
See Intent Language.

### Oryn
Open Runtime for Intentful Navigation - the browser automation system designed for AI agents.

## P

### Pack
See Intent Pack.

### Pattern
A recognized UI structure on a page (e.g., login_form, search_form, cookie_banner).

### Pattern Detection
Automatic recognition of common UI patterns during page scanning.

### Protocol Layer
The layer that parses commands, resolves targets, and formats responses.

## R

### REPL
Read-Eval-Print Loop - the interactive command interface after starting Oryn.

### Role
Semantic classification of an element's purpose: email, password, search, submit, username, tel, url.

## S

### Scanner
The JavaScript module that runs inside browser contexts to observe and interact with web pages.

### Scanner Protocol
The JSON-based protocol between backends and the Universal Scanner.

### Scan
The process of analyzing a page to identify interactive elements and patterns.

### Semantic Command
A command that targets elements by text or role rather than ID (e.g., `click "Sign in"`).

### Semantic Targeting
Referencing elements by meaning (text, role) rather than implementation (ID, selector).

### Session Intent
An intended runtime concept for `define`-based temporary intents. In unified CLI today, `define`/session intent management is not fully wired end-to-end.

### Stale Element
An element reference that is no longer valid because the DOM changed.

## T

### Target
The element specification in a command. Can be ID, text, role, selector, or relational.

### Target Resolution
The process of converting a target specification to a concrete element ID.

### Tier
Classification of intents by origin:
- **Tier 1**: Built-in (compiled into binary)
- **Tier 2**: Loaded (from YAML files)
- **Tier 3**: Discovered (learned during session)

## U

### Universal Scanner
The single JavaScript implementation that runs identically in all browser contexts, ensuring consistent behavior.

## V

### Viewport
The visible area of the browser window.

## W

### WebDriver
The W3C standard protocol for browser automation, used by oryn-e.

### WPE WebKit
A WebKit port for embedded systems, used by oryn-e.

## Symbols

### @
Page header indicator in observation output (e.g., `@ example.com "Title"`).

### [ ]
Element notation (e.g., `[5]` for element ID 5).

### { }
Modifier notation (e.g., `{required}`, `{disabled}`).

### #
Comment or section header in output.

### +
Element appeared (in change notation).

### -
Element disappeared (in change notation).

### ~
Element/URL changed (in change notation).
