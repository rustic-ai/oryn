# Lemmascope: Unified Tri-Modal Architecture

## Version 1.0

---

## 1. Executive Summary

Lemmascope is an agent-native browser designed from the ground up for AI agents. Rather than forcing agents to interpret screenshots, parse raw HTML, or construct complex function calls, Lemmascope provides a semantic intent language that speaks directly to how agents think about web interaction.

The system comprises three specialized binaries unified by a single Universal Scanner:

| Binary | Name | Environment |
|--------|------|-------------|
| **lscope-e** | Embedded | IoT, containers, edge devices |
| **lscope-h** | Headless | Cloud automation, CI/CD, scraping |
| **lscope-r** | Remote | User assistance, real browser sessions |

All three binaries share the same protocol, the same intent language, and the same scanner implementation—ensuring consistent behavior regardless of deployment environment.

---

## 2. The Core Problem

### 2.1 Why Agents Struggle with Browsers

Current approaches to browser automation for AI agents fall into predictable failure patterns:

**Screenshot/Vision Approaches**
Agents are asked to interpret visual screenshots of web pages. This requires:
- Expensive vision model inference
- Unreliable text extraction from rendered images
- No understanding of interactive affordances
- Inability to perceive hidden state (disabled buttons, required fields)

**HTML Parsing Approaches**
Agents receive raw HTML and must extract structure. This requires:
- Understanding the full complexity of HTML/CSS/JavaScript
- Reasoning about what is actually visible versus hidden
- Mapping DOM structure to interactive possibilities
- Maintaining context across thousands of tokens of markup

**Function Call Approaches**
Agents must construct precise function calls with typed parameters. This requires:
- Rigid schema adherence
- Complex parameter formatting
- No tolerance for natural variation
- Verbose tool definitions consuming context

### 2.2 The Lemmascope Solution

Lemmascope inverts the traditional model. Instead of exposing browser complexity to agents, it provides:

**Semantic Observations**
Structured descriptions of interactive elements with meaningful labels, types, roles, and states—not raw HTML or pixels.

**Intent Language**
Natural, forgiving syntax for expressing actions at the appropriate level of abstraction—not rigid function signatures.

**Consistent Behavior**
Identical semantics across embedded, headless, and remote modes—not per-environment quirks.

---

## 3. Architecture Overview

### 3.1 System Layers

**Agent Layer**
AI agents communicate with Lemmascope using the Intent Language. They receive observations, make decisions, and issue commands. Agents never interact with raw browser APIs or HTML.

**Protocol Layer**
The Intent Parser interprets agent commands with forgiveness for variations. The Semantic Resolver translates targets (text matches, roles) to concrete elements. The Change Tracker monitors DOM modifications.

**Backend Trait**
A unified interface that all three binaries implement. Commands are dispatched identically regardless of underlying browser engine.

**Scanner Layer**
JavaScript running inside the browser context that scans, labels, and interacts with elements. The same code runs in WebKit, Chromium, and browser extensions.

### 3.2 The Universal Scanner

The scanner is the architectural keystone. All three backends inject the identical JavaScript into their browser contexts:

**lscope-e**: Injects via WebDriver execute_script  
**lscope-h**: Injects via CDP Runtime.evaluate  
**lscope-r**: Runs as browser extension content script  

Because the same scanner code executes in all contexts, behavioral consistency is guaranteed. The Rust layer never parses HTML directly—it only processes the scanner's JSON responses.

This design principle is critical:

> **HTML parsing should NOT happen in Rust.**

If HTML were parsed differently per backend, behavior would diverge. The Universal Scanner eliminates this risk.

---

## 4. The Three Modes

### 4.1 Mode Comparison

| Aspect | lscope-e (Embedded) | lscope-h (Headless) | lscope-r (Remote) |
|--------|---------------------|---------------------|-------------------|
| **Browser Engine** | WPE WebKit | Chromium | User's Browser |
| **Protocol** | WebDriver (HTTP) | CDP (WebSocket) | Custom (WebSocket) |
| **Binary Size** | ~15MB + libs | ~15MB (Chrome separate) | ~15MB |
| **RAM Usage** | **~50MB** ⭐ | ~300MB+ | **Zero** (client-side) |
| **Compatibility** | ~95% (WebKit) | **~99%** ⭐ | **~99%** ⭐ |
| **Anti-Bot Bypass** | Weak | Medium | **Strong** ⭐ |

### 4.2 lscope-e: Embedded Mode

**Technology Stack**
- COG browser (WPE WebKit)
- WebDriver protocol via fantoccini
- Optimized for minimal resource consumption

**Best For**
- Raspberry Pi and IoT devices
- Resource-constrained containers
- Alpine Linux deployments
- Edge computing scenarios
- Environments where RAM is precious

**Characteristics**
- Smallest memory footprint (~50MB)
- WebKit rendering (excellent standards compliance)
- Some anti-bot detection vulnerability (recognizable fingerprint)
- Self-contained deployment possible

### 4.3 lscope-h: Headless Mode

**Technology Stack**
- Chromium browser
- Chrome DevTools Protocol via chromiumoxide
- Full browser capabilities without display

**Best For**
- Cloud-based automation
- CI/CD pipeline testing
- High-volume web scraping
- Complex SPA interaction
- Scenarios requiring maximum compatibility

**Characteristics**
- Full Chromium compatibility (~99%)
- Higher resource requirements (~300MB+)
- Network interception capabilities
- PDF generation and printing
- DevTools debugging integration

### 4.4 lscope-r: Remote Mode

**Technology Stack**
- Browser extension in user's own browser
- WebSocket communication
- Real user session and credentials

**Best For**
- User assistance workflows ("book me a flight")
- Demonstrations and teaching
- Tasks requiring authenticated sessions
- Anti-bot bypass (real browser fingerprint)
- Interactive agent assistance

**Characteristics**
- Zero server-side memory (runs in user's browser)
- Maximum anti-bot effectiveness (real browser instance)
- Access to user's logged-in sessions
- Visual feedback for users watching agent work
- Requires user to install extension and maintain connection

---

## 5. Backend Interface

### 5.1 Unified Contract

All three binaries implement identical capabilities:

**Lifecycle Management**
- Launch browser context
- Close browser context
- Check readiness state

**Navigation**
- Navigate to URL
- Handle page load completion

**Scanner Operations**
- Execute scanner commands
- Receive scanner responses

**Capture**
- Screenshot current state

### 5.2 Scanner Command Flow

1. Agent issues Intent Language command
2. Protocol layer parses and resolves command
3. Command is translated to Scanner JSON
4. Backend sends JSON to browser context (method varies by mode)
5. Scanner executes and returns JSON response
6. Backend receives and deserializes response
7. Protocol layer formats response for agent

The JSON format is identical regardless of transport. Only the communication mechanism differs.

---

## 6. Protocol Guarantees

### 6.1 Consistency

Given the same page state and the same command:
- lscope-e, lscope-h, and lscope-r produce identical observations
- lscope-e, lscope-h, and lscope-r execute identical actions
- lscope-e, lscope-h, and lscope-r report identical results

### 6.2 Limitations

Each mode has inherent limitations from its underlying technology:

**lscope-e Limitations**
- WebKit may render some pages differently than Chromium
- Some cutting-edge web features may lag Chromium support
- WebDriver protocol has performance overhead versus CDP

**lscope-h Limitations**
- Headless Chrome has detectable fingerprint
- Higher resource requirements
- Chrome installation required (not bundled)

**lscope-r Limitations**
- Requires user to install and connect extension
- User's browser must remain open
- Network dependent (WebSocket connection)
- Cannot operate unattended without user presence

---

## 7. Deployment Scenarios

### 7.1 Scenario: IoT Automation Hub

**Use Case**: Smart home controller running automation agents

**Recommended**: lscope-e (Embedded)

**Why**: Raspberry Pi has limited RAM. The ~50MB footprint of WPE WebKit leaves resources for the agent itself. COG provides reliable rendering without Chromium overhead.

### 7.2 Scenario: Cloud Scraping Service

**Use Case**: High-volume data extraction from multiple sites

**Recommended**: lscope-h (Headless)

**Why**: Maximum compatibility ensures complex SPAs render correctly. DevTools integration enables network interception for API discovery. Cloud resources handle RAM requirements.

### 7.3 Scenario: Personal Assistant

**Use Case**: Agent helps user book travel, manage accounts

**Recommended**: lscope-r (Remote)

**Why**: Agent needs access to user's authenticated sessions. Real browser fingerprint bypasses anti-bot. User can watch and verify agent actions. No credentials passed to server.

### 7.4 Scenario: CI/CD Testing

**Use Case**: Automated testing of web application

**Recommended**: lscope-h (Headless)

**Why**: Chromium matches production browser. Headless operation fits CI environment. Screenshot capture documents failures. Network interception enables API mocking.

### 7.5 Scenario: Edge Retail Kiosk

**Use Case**: In-store kiosk with voice-controlled web agent

**Recommended**: lscope-e (Embedded)

**Why**: Self-contained deployment without Chrome dependency. Low resource requirements for embedded hardware. WebKit provides adequate compatibility for specific target sites.

---

## 8. Development Priorities

### 8.1 Phase 1: Core + Remote Mode

**Rationale**: Remote mode provides immediate visual feedback during development. The browser extension runs in a standard browser where issues are easily debugged. This phase establishes the scanner and protocol foundations.

**Deliverables**:
- Universal Scanner implementation
- Backend trait definition
- Remote backend (WebSocket server)
- Browser extension (Chrome)
- Intent Language parser
- Semantic resolution layer

### 8.2 Phase 2: Headless Mode

**Rationale**: Same scanner, different transport. CDP integration via chromiumoxide builds on Phase 1 foundations with a production-ready backend.

**Deliverables**:
- Headless backend (CDP client)
- Chrome-specific features (network interception, etc.)
- Headless testing infrastructure

### 8.3 Phase 3: Embedded Mode

**Rationale**: Most environment setup required. WebDriver integration via fantoccini completes the tri-modal architecture.

**Deliverables**:
- Embedded backend (WebDriver client)
- COG/WPE integration
- Alpine container configuration
- Low-memory environment testing

### 8.4 Phase 4: Polish

**Deliverables**:
- Mode auto-detection
- Unified CLI experience
- Comprehensive documentation
- Cross-backend test suite

---

## 9. Summary

| Component | Implementation |
|-----------|----------------|
| **Universal Scanner** | JavaScript injected into all browser contexts |
| **Backend Trait** | Unified Rust interface for all modes |
| **lscope-e** | WebDriver → COG → WPE WebKit |
| **lscope-h** | CDP → Chromium |
| **lscope-r** | WebSocket → Browser Extension |
| **Intent Language** | Agent-facing protocol |
| **Scanner Protocol** | Internal JSON protocol |

**The Key Insight**

The scanner is the source of truth. Rust never parses HTML. All backends inject the same JavaScript, guaranteeing consistent behavior across embedded devices, cloud servers, and user browsers.

Agents get a clean, semantic interface to the web—not pixels, not HTML, not rigid function calls. Just intent.

---

*Document Version: 1.0*  
*Last Updated: January 2025*
