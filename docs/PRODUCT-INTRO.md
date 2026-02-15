# Oryn: Product Introduction

## The Browser Designed for Agents

---

## The Vision

AI agents are transforming how we interact with software. They can reason, plan, and execute complex tasks. But when it comes to the web—the largest repository of information and services on earth—agents struggle.

Why? Because browsers weren't built for them.

Oryn changes that. It's the first browser designed from the ground up with AI agents as the primary user.

---

## The Problem We Solve

### Current Approaches Are Broken

When AI agents need to interact with websites, they're forced into workflows that don't match how they think:

**The Screenshot Trap**

Some systems send agents screenshots and ask them to "see" the page. This requires expensive vision models, produces unreliable results for text-heavy interfaces, and completely misses invisible state like whether a button is disabled or a field is required. Agents end up guessing at what they can click.

**The HTML Swamp**

Other systems dump raw HTML into agent context windows. Thousands of tokens of markup, most of it irrelevant, hiding the dozen interactive elements that actually matter. Agents waste context trying to parse what browsers already understand—and often get it wrong when JavaScript has modified the visible state.

**The Function Call Maze**

Tool-based approaches define rigid schemas: precise parameter names, exact types, complex nested structures. One wrong field and the call fails. Agents spend tokens on verbose tool definitions and careful formatting instead of accomplishing tasks.

### The Real Need

Agents need to:
1. Understand what's on a page (observations)
2. Express what they want to do (commands)
3. Get consistent results regardless of environment (reliability)

None of the current approaches deliver this cleanly.

---

## The Oryn Solution

### Intent Over Implementation

Oryn inverts the traditional model. Instead of exposing browser complexity, it provides a semantic layer that matches agent cognition.

**Observations are structured and meaningful:**

```
@ github.com/login "Sign in to GitHub"

[1] input/email "Username or email" {required}
[2] input/password "Password" {required}
[3] button/submit "Sign in" {primary}

# patterns
- login_form: [1,2] → [3]
```

The agent sees labeled elements, their types, their roles, their states. Not HTML. Not pixels. Just the interactive surface that matters.

**Commands are natural and forgiving:**

```
type email "user@example.com"
type password "secret123"
click "Sign in"
```

No CSS selectors. No XPath expressions. No rigid function schemas. Just intent.

### The Core Insight

A browser that provides agent-friendly intent language will always outperform systems that ask agents to understand images, parse markup, or construct function calls.

This isn't incremental improvement. It's a fundamental shift in abstraction level.

---

## How It Works

### Universal Scanner

At the heart of Oryn is the Universal Scanner—a JavaScript module that runs inside web pages and understands them the way agents need to.

The scanner:
- Identifies all interactive elements
- Labels them with simple numeric IDs
- Classifies their types (input, button, link, etc.)
- Infers their roles (email, password, search, submit)
- Reports their states (required, disabled, checked)
- Detects common patterns (login forms, search boxes, pagination)

This same scanner code runs in all environments—embedded devices, headless servers, and browser extensions—guaranteeing consistent behavior.

### Intent Language

The Oryn Intent Language (OIL) is designed for agent ergonomics:

**Token Efficient**: Minimal verbosity means more context for reasoning

**Forgiving Syntax**: Multiple ways to express the same intent all work

**Multi-Level Abstraction**: From `click 5` to `login "user" "pass"`

**Readable Responses**: Clear state representation, obvious error recovery

### Three Deployment Modes

Oryn adapts to your environment:

| Mode     | Binary | Best For                  |
| -------- | ------ | ------------------------- |
| Embedded | oryn-e | IoT, edge, containers     |
| Headless | oryn-h | Cloud, CI/CD, scraping    |
| Remote   | oryn-r | Assistance, auth sessions |

Same protocol. Same behavior. Different deployment targets.

---

## Key Capabilities

### Semantic Targeting

Agents can reference elements by meaning, not implementation:

```
type email "user@test.com"      # By role
click "Sign in"                  # By text
check "Remember me"              # By label
```

Oryn resolves these to concrete elements. The agent doesn't need to know the CSS selector.

### Pattern Detection

Common UI patterns are automatically recognized:

```
# patterns
- login_form: email=[1] password=[2] submit=[3]
- cookie_banner: accept=[7]
```

Agents can immediately understand page structure without element-by-element analysis.

### Change Tracking

After actions, Oryn reports what changed:

```
ok click 3

# changes
~ url: /login → /dashboard
+ [1] nav "Dashboard"
- [3] button "Sign in"
```

Agents understand the impact of their actions without re-scanning.

### Error Recovery

When things go wrong, agents get actionable guidance:

```
error click 99: element not found

# hint
Available elements: 1-6. Run 'observe' to refresh.
```

Not just failure—a path forward.

---

## Use Cases

### Personal Assistant

An agent helps a user book travel:

- oryn-r connects to user's browser
- Agent has access to user's authenticated sessions
- User watches agent navigate booking sites
- Real browser fingerprint bypasses anti-bot measures

### Cloud Automation

A system monitors competitor pricing:

- oryn-h runs in cloud containers
- Headless Chrome handles complex SPAs
- Network interception captures API responses
- High-volume parallel execution

### Edge Intelligence

A retail kiosk provides voice-controlled browsing:

- oryn-e runs on embedded hardware
- 50MB footprint leaves room for voice processing
- Self-contained without Chrome dependency
- WebKit handles target site compatibility

### CI/CD Testing

An AI agent writes and executes tests:

- oryn-h provides consistent Chromium environment
- Screenshots document failures
- Network mocking isolates tests
- Same browser as production users

---

## Technical Foundation

### Architecture Principles

**Scanner as Source of Truth**

All HTML understanding happens in JavaScript, inside the browser. The Rust layer never parses HTML directly. This guarantees identical behavior across all three modes.

**Backend Abstraction**

A unified trait defines what all backends must implement. Swap embedded for headless without changing agent logic.

**Protocol Consistency**

The same JSON protocol flows between scanner and backend regardless of transport (WebDriver, CDP, WebSocket).

### Resource Requirements

| Mode   | RAM         | Notes                                |
| ------ | ----------- | ------------------------------------ |
| oryn-e | ~50MB       | WPE WebKit, minimal footprint        |
| oryn-h | ~300MB+     | Full Chromium, maximum compatibility |
| oryn-r | Zero server | Runs in user's browser               |

### Compatibility

| Mode   | Browser Engine | Web Compatibility |
| ------ | -------------- | ----------------- |
| oryn-e | WPE WebKit     | ~95%              |
| oryn-h | Chromium       | ~99%              |
| oryn-r | User's Browser | ~99%              |

---

## Competitive Positioning

### Versus Screenshot-Based Tools

Screenshot tools require vision models and produce unreliable results. Oryn provides structured observations with explicit state.

**Oryn advantage**: No vision overhead, precise state information, token efficiency

### Versus HTML Injection Tools

HTML injection consumes context and requires complex parsing logic. Oryn pre-processes HTML into semantic observations.

**Oryn advantage**: Smaller context, higher accuracy, automatic pattern detection

### Versus Function-Call Tools

Function tools require rigid schemas and verbose definitions. Oryn accepts natural, forgiving syntax.

**Oryn advantage**: Fewer failures, simpler prompts, multi-level abstraction

### Versus General Browser Automation

Playwright, Puppeteer, and Selenium are designed for developers writing scripts. Oryn is designed for AI agents expressing intent.

**Oryn advantage**: Agent-native interface, semantic targeting, built-in pattern recognition

---

## Future Directions

### Goal-Level Commands

Beyond intent commands, Oryn will support natural language goals:

```
goal: add "Blue T-Shirt Size M" to cart
goal: find the contact email on this page
goal: subscribe to the newsletter
```

The agent expresses what it wants to achieve; Oryn plans the execution.

### Learning Patterns

Site-specific pattern recognition will improve through observed interactions, automatically adapting to common site structures.

### Multi-Page Flows

Complex workflows spanning multiple pages will be expressible as single high-level commands, with Oryn managing state across navigation.

---

## Summary

Oryn represents a fundamental rethinking of how AI agents interact with the web.

**The Core Insight**: A browser that provides agent-friendly intent language will always outperform systems that ask agents to understand images, parse HTML, or construct function calls.

**The Product**: Three binaries (oryn-e, oryn-h, oryn-r) unified by a common protocol and universal scanner, deployable from IoT devices to cloud infrastructure to user browsers.

**The Result**: Agents that can navigate the web naturally, reliably, and efficiently.

---

*Oryn: The browser that speaks agent.*

---

*Document Version: 1.0*  
*Last Updated: January 2025*
