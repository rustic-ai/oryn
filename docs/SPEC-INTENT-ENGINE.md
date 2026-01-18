# Lemmascope Intent Engine Specification

## Version 1.0

---

## 1. Overview

The Intent Engine is the intelligence layer of Lemmascope that transforms high-level agent commands into executable atomic operations. It bridges the gap between what agents want to accomplish and the primitive operations the scanner understands.

### 1.1 Core Principle

**The scanner remains simple; the engine provides intelligence.**

The Universal Scanner knows only atomic commands: `scan`, `click`, `type`, `select`. It has no concept of "login" or "checkout" or "accept cookies." The Intent Engine holds this knowledge, expanding intent commands into sequences of atomic operations while handling errors, verifying outcomes, and learning from experience.

### 1.2 Design Goals

**Extensibility Without Recompilation**

New intents can be added through definition files without rebuilding binaries. The engine loads intent definitions at startup and can reload them at runtime.

**Progressive Intelligence**

The system starts with reliable built-in intents and grows smarter through loaded definitions, site-specific packs, and discovered patterns. Intelligence accumulates over time.

**Agent Empowerment**

Agents can define their own intents during sessions, teaching the engine new workflows. Successful patterns can be promoted to persistent definitions.

**Graceful Degradation**

When an intent cannot execute as defined, the engine falls back to heuristics, provides actionable errors, and suggests alternatives. Partial success is reported clearly.

### 1.3 Architecture Position

```
┌─────────────────────────────────────────────────────────────┐
│  Agent                                                      │
│  (Issues intent commands: login, checkout, search)          │
└─────────────────────────┬───────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────────┐
│  Intent Parser                                              │
│  (Recognizes intent syntax, extracts parameters)            │
└─────────────────────────┬───────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────────┐
│  INTENT ENGINE  ◄─── This Specification                     │
│                                                             │
│  • Intent Registry (built-in, loaded, discovered)           │
│  • Pattern Matcher (maps page patterns to available intents)│
│  • Intent Executor (expands and runs intent steps)          │
│  • Intent Learner (observes patterns, proposes intents)     │
│  • Pack Manager (loads site-specific intent packs)          │
└─────────────────────────┬───────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────────┐
│  Scanner Interface                                          │
│  (Sends atomic commands: scan, click, type, select)         │
└─────────────────────────┬───────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────────┐
│  Universal Scanner (JavaScript)                             │
│  (Executes atomics, returns results)                        │
└─────────────────────────────────────────────────────────────┘
```

---

## 2. Intent Tiers

Intents are organized into three tiers based on their origin, stability, and persistence.

### 2.1 Tier 1: Built-in Intents

**Characteristics**
- Compiled into the Rust binary
- Cannot be modified without recompilation
- Extensively tested and hardened
- Available immediately on startup
- Highest execution priority

**Built-in Intent List**

| Intent | Description |
|--------|-------------|
| `login` | Authenticate with username/password |
| `logout` | End authenticated session |
| `search` | Submit search query |
| `accept_cookies` | Dismiss cookie consent banner |
| `dismiss_popups` | Close modal dialogs and overlays |
| `scroll_to` | Scroll element into view |
| `fill_form` | Fill multiple form fields |
| `submit_form` | Submit the current form |

**Rationale**

These intents represent the most common, cross-site operations that agents perform. They are built-in because:
- They must work reliably across all deployments
- They benefit from Rust's performance and safety
- They serve as reference implementations for loaded intents
- They provide fallback when loaded intents fail

### 2.2 Tier 2: Loaded Intents

**Characteristics**
- Defined in YAML/JSON files
- Loaded at engine startup
- Can be reloaded at runtime
- Organized into site-specific packs
- Validated against schema before registration

**Sources**

| Source | Path | Description |
|--------|------|-------------|
| Core | `intents/core/*.yaml` | Common intents shipped with Lemmascope |
| Site Packs | `intents/packs/{domain}/*.yaml` | Site-specific intents |
| User | `~/.lemmascope/intents/*.yaml` | User-defined intents |
| Session | Runtime registration | Temporary intents |

**Loading Priority**

When multiple definitions exist for the same intent name:
1. User definitions override all others
2. Site pack definitions override core
3. Core definitions are the baseline

### 2.3 Tier 3: Discovered Intents

**Characteristics**
- Created by the Intent Learner during operation
- Session-scoped by default
- Require explicit promotion to persist
- Lower confidence than defined intents
- Include provenance metadata

**Lifecycle**

```
Observed Pattern → Proposed Intent → Refined Intent → Exported Definition
      │                  │                 │                  │
      ▼                  ▼                 ▼                  ▼
   Learner          Agent Review      Agent Testing      Tier 2 File
   detects          confirms or       validates          written to
   repetition       rejects           behavior           disk
```

---

## 3. Intent Definition Format

### 3.1 Schema Overview

Intent definitions use YAML format with the following structure:

```yaml
# Required metadata
intent: <name>
version: <semver>

# Optional metadata
description: <human-readable description>
author: <creator identifier>
tags: [<categorization tags>]

# Activation conditions
triggers:
  patterns: [<pattern names that enable this intent>]
  keywords: [<text that might invoke this intent>]
  urls: [<URL patterns where this intent applies>]

# Input specification
parameters:
  - name: <parameter name>
    type: <string|number|boolean|object|array>
    required: <boolean>
    default: <default value>
    description: <parameter description>

# Execution specification
steps:
  - <step definition>
  - <step definition>
  ...

# Outcome specification
success:
  conditions: [<success indicators>]
  extract: <data to return on success>

failure:
  conditions: [<failure indicators>]
  recovery: [<recovery steps>]

# Execution options
options:
  timeout: <duration>
  retry: <retry configuration>
  checkpoint: <boolean>
```

### 3.2 Step Definitions

Each step in the `steps` array defines an atomic action or control flow operation.

**Action Steps**

```yaml
# Click an element
- action: click
  target: <target specification>
  options:
    button: left|right|middle
    count: <click count>
    force: <boolean>

# Type into an element
- action: type
  target: <target specification>
  text: <text or parameter reference>
  options:
    clear: <boolean>
    delay: <milliseconds>
    enter: <boolean>

# Select from dropdown
- action: select
  target: <target specification>
  value: <value, text, or index>

# Check/uncheck checkbox
- action: check|uncheck
  target: <target specification>

# Clear input field
- action: clear
  target: <target specification>

# Scroll viewport or element
- action: scroll
  target: <target specification or viewport>
  direction: up|down|left|right
  amount: <pixels or 'page'>

# Wait for condition
- action: wait
  condition: <condition specification>
  timeout: <duration>

# Fill multiple fields
- action: fill_form
  target: <form pattern or selector>
  data: <object or parameter reference>

# Execute sub-intent
- action: intent
  name: <intent name>
  params: <parameter mapping>

# Custom JavaScript (escape hatch)
- action: execute
  script: <javascript code>
  args: [<arguments>]
```

**Control Flow Steps**

```yaml
# Conditional branch
- branch:
    if: <condition>
    then: [<steps>]
    else: [<steps>]  # optional

# Loop over elements or data
- loop:
    over: <array or element selector>
    as: <variable name>
    steps: [<steps>]
    max: <maximum iterations>

# Try with fallback
- try:
    steps: [<steps>]
    catch: [<fallback steps>]

# Parallel execution (future)
- parallel:
    steps: [<steps>]
    wait: all|any|none
```

### 3.3 Target Specification

Targets identify which element(s) an action operates on.

```yaml
# By pattern reference
target:
  pattern: login_form.email

# By element role
target:
  role: email|password|submit|search|...

# By visible text
target:
  text: "Sign in"
  match: exact|contains|regex

# By selector
target:
  selector: "#email-input"
  
# By element ID from current scan
target:
  id: 5

# Combined with fallbacks
target:
  pattern: login_form.submit
  fallback:
    role: submit
    fallback:
      text_contains: [sign in, log in, submit]
```

### 3.4 Condition Specification

Conditions control wait steps and branching.

```yaml
# Pattern exists
condition:
  pattern_exists: login_form

# Element visible
condition:
  visible: <target specification>

# Element hidden
condition:
  hidden: <target specification>

# URL matches
condition:
  url_contains: [dashboard, home]
  # or
  url_matches: "^https://example.com/app/.*"

# Text appears
condition:
  text_contains: "Success"
  within: <target specification>  # optional scope

# Element count
condition:
  count: 
    selector: ".search-result"
    min: 1
    max: 100

# Custom expression
condition:
  expression: "$items.length > 0"

# Compound conditions
condition:
  all:
    - url_contains: dashboard
    - visible: { pattern: user_menu }
  # or
  any:
    - url_contains: success
    - url_contains: confirmation
```

### 3.5 Parameter References

Parameters are referenced using `$` prefix:

```yaml
parameters:
  - name: username
    type: string
    required: true
  - name: password
    type: string
    required: true
  - name: remember
    type: boolean
    default: false

steps:
  - action: type
    target: { role: email }
    text: $username           # Simple reference
    
  - action: type
    target: { role: password }
    text: $password
    
  - branch:
      if: $remember           # Boolean parameter in condition
      then:
        - action: check
          target: { text_contains: "remember" }
```

**Object Parameter Access**

```yaml
parameters:
  - name: shipping
    type: object

steps:
  - action: type
    target: { pattern: shipping_form.name }
    text: $shipping.full_name
    
  - action: type
    target: { pattern: shipping_form.address }
    text: $shipping.street_address
```

---

## 4. Built-in Intent Specifications

### 4.1 login

**Purpose**: Authenticate using username/email and password.

**Syntax**
```
login <username> <password> [--no-submit] [--wait <duration>]
```

**Parameters**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| username | string | yes | Username or email address |
| password | string | yes | Password |

**Options**

| Option | Default | Description |
|--------|---------|-------------|
| `--no-submit` | false | Fill fields but don't click submit |
| `--wait` | 10s | Time to wait for navigation after submit |

**Execution Steps**

1. Scan page for elements and patterns
2. Locate login form via:
   - `login_form` pattern (preferred)
   - Heuristic: email/username input + password input + submit button
3. Type username into identified field
4. Type password into identified field
5. Click submit button (unless `--no-submit`)
6. Wait for navigation or error state
7. Verify outcome:
   - Success: URL changed, no login form present
   - Failure: Login form still present, error pattern detected

**Success Response**
```
ok login

# actions
type [1] "user@example.com"
type [2] "••••••••"
click [3] "Sign in"
wait navigation

# changes
~ url: /login → /dashboard
- login_form
+ user_menu
```

**Failure Response**
```
error login: authentication failed

# actions
type [1] "user@example.com"
type [2] "••••••••"
click [3] "Sign in"

# result
Form error: "Incorrect username or password."

# hint
Verify credentials and retry.
```

**Fallback Behavior**

If no `login_form` pattern is detected:

```
1. Find element with role=email OR role=username
   OR input with name/id containing 'email', 'user', 'login'
   
2. Find element with role=password
   OR input[type=password]
   
3. Find element with role=submit
   OR button with text containing 'sign in', 'log in', 'submit'
   OR input[type=submit] within same form
   
4. If all three found, proceed with login
5. If any missing, return error with specific guidance
```

### 4.2 search

**Purpose**: Submit a search query.

**Syntax**
```
search <query> [--submit] [--wait <duration>]
```

**Parameters**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| query | string | yes | Search terms |

**Options**

| Option | Default | Description |
|--------|---------|-------------|
| `--submit` | auto | How to submit: `enter`, `click`, or `auto` |
| `--wait` | 5s | Time to wait for results |

**Execution Steps**

1. Scan for search form pattern or search input
2. Clear existing content if any
3. Type search query
4. Submit via:
   - Press Enter (most common)
   - Click search button if present
5. Wait for results (URL change or content update)

**Success Response**
```
ok search "rust programming"

# actions
clear [1]
type [1] "rust programming"
press Enter

# changes
~ url: / → /search?q=rust+programming
+ search_results [5-15]
```

### 4.3 accept_cookies

**Purpose**: Dismiss cookie consent banners.

**Syntax**
```
accept_cookies [--reject] [--wait <duration>]
```

**Options**

| Option | Default | Description |
|--------|---------|-------------|
| `--reject` | false | Click reject instead of accept |
| `--wait` | 2s | Time to wait for banner to appear |

**Execution Steps**

1. Scan for `cookie_banner` pattern
2. If not found, search for common cookie consent indicators:
   - Elements with 'cookie', 'consent', 'gdpr' in class/id
   - Buttons with 'accept', 'agree', 'ok', 'got it' text
3. Click appropriate button (accept or reject based on option)
4. Verify banner dismissal

**Success Response**
```
ok accept_cookies

# actions
click [7] "Accept All"

# changes
- cookie_banner
- [7] button "Accept All"
- [8] button "Manage Preferences"
```

**No Banner Response**
```
ok accept_cookies

# result
No cookie banner detected.
```

### 4.4 dismiss_popups

**Purpose**: Close modal dialogs, overlays, and interruptions.

**Syntax**
```
dismiss_popups [--all] [--type <type>]
```

**Options**

| Option | Default | Description |
|--------|---------|-------------|
| `--all` | true | Dismiss all detected popups |
| `--type` | any | Filter: `modal`, `overlay`, `toast`, `banner` |

**Execution Steps**

1. Scan for popup patterns:
   - `modal_dialog`
   - `overlay`
   - `cookie_banner`
   - `notification_toast`
   - `promo_popup`
2. For each detected popup:
   - Find close/dismiss button
   - Click to dismiss
   - Verify removal
3. Re-scan to check for newly revealed popups
4. Repeat until no popups remain (max 5 iterations)

**Success Response**
```
ok dismiss_popups

# dismissed
[1] modal "Subscribe to newsletter" → clicked "✕"
[2] cookie_banner → clicked "Accept"

# changes
- modal_dialog
- cookie_banner
```

### 4.5 fill_form

**Purpose**: Fill multiple form fields from a data object.

**Syntax**
```
fill_form <data> [--pattern <pattern>] [--partial]
```

**Parameters**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| data | object | yes | Field name/value pairs |

**Options**

| Option | Default | Description |
|--------|---------|-------------|
| `--pattern` | auto | Form pattern to target |
| `--partial` | false | Allow filling only some fields |

**Execution Steps**

1. Identify target form:
   - Use specified pattern
   - Or find form containing focused element
   - Or find first form on page
2. For each key in data object:
   - Find matching field by: pattern reference, name attribute, id, label text
   - Set appropriate value based on field type:
     - Text inputs: type value
     - Selects: select by value or text
     - Checkboxes: check/uncheck based on boolean
     - Radio buttons: select matching value
3. Report fields filled and any that couldn't be matched

**Success Response**
```
ok fill_form

# filled
[2] input "First name" ← "John"
[3] input "Last name" ← "Smith"
[4] input "Email" ← "john@example.com"
[5] select "Country" ← "United States"
[6] checkbox "Subscribe" ← checked

# skipped
"middle_name": no matching field found
```

### 4.6 submit_form

**Purpose**: Submit the current or specified form.

**Syntax**
```
submit_form [--pattern <pattern>] [--wait <duration>]
```

**Options**

| Option | Default | Description |
|--------|---------|-------------|
| `--pattern` | auto | Form pattern to target |
| `--wait` | 10s | Time to wait for response |

**Execution Steps**

1. Identify target form
2. Find submit mechanism:
   - Submit button within form
   - Input[type=submit]
   - Button with submit role
3. Click submit
4. Wait for:
   - Navigation
   - Form disappearance
   - Success/error message
5. Report outcome

---

## 5. Intent Execution Model

### 5.1 Execution Pipeline

```
┌─────────────────────────────────────────────────────────────┐
│  1. PARSE                                                   │
│     • Extract intent name and parameters                    │
│     • Validate parameter types                              │
│     • Apply defaults                                        │
└─────────────────────────┬───────────────────────────────────┘
                          ▼
┌─────────────────────────────────────────────────────────────┐
│  2. RESOLVE                                                 │
│     • Look up intent definition (Tier 1 → 2 → 3)           │
│     • Check if intent is available (triggers satisfied)     │
│     • Bind parameters to step templates                     │
└─────────────────────────┬───────────────────────────────────┘
                          ▼
┌─────────────────────────────────────────────────────────────┐
│  3. PLAN                                                    │
│     • Scan page for current state                           │
│     • Resolve targets to element IDs                        │
│     • Evaluate conditions for branches                      │
│     • Generate concrete step sequence                       │
└─────────────────────────┬───────────────────────────────────┘
                          ▼
┌─────────────────────────────────────────────────────────────┐
│  4. EXECUTE                                                 │
│     • Run each step sequentially                            │
│     • Send atomic commands to scanner                       │
│     • Collect results and track changes                     │
│     • Handle step failures per retry policy                 │
└─────────────────────────┬───────────────────────────────────┘
                          ▼
┌─────────────────────────────────────────────────────────────┐
│  5. VERIFY                                                  │
│     • Check success conditions                              │
│     • Check failure conditions                              │
│     • Extract result data if specified                      │
│     • Determine final outcome                               │
└─────────────────────────┬───────────────────────────────────┘
                          ▼
┌─────────────────────────────────────────────────────────────┐
│  6. RESPOND                                                 │
│     • Format response for agent                             │
│     • Include action log                                    │
│     • Include changes detected                              │
│     • Include hints on failure                              │
└─────────────────────────────────────────────────────────────┘
```

### 5.2 Target Resolution

When executing a step, targets must be resolved to concrete element IDs.

**Resolution Order**

1. **Pattern reference**: Look up element ID from detected patterns
2. **Role match**: Find element with matching role
3. **Text match**: Find element with matching text
4. **Selector match**: Query DOM with CSS selector
5. **Fallback chain**: Try each fallback in order

**Resolution Algorithm**

```
function resolve_target(target, elements, patterns):
    if target.pattern:
        path = parse_pattern_path(target.pattern)  # e.g., "login_form.email"
        if patterns[path.pattern] and patterns[path.pattern][path.field]:
            return patterns[path.pattern][path.field]
    
    if target.role:
        matches = elements.filter(e => e.role == target.role)
        if matches.length == 1:
            return matches[0].id
        if matches.length > 1:
            # Apply disambiguation: prefer visible, enabled, primary
            return disambiguate(matches)
    
    if target.text:
        match_fn = get_match_function(target.match || 'contains')
        matches = elements.filter(e => match_fn(e.text, target.text))
        if matches.length >= 1:
            return disambiguate(matches)
    
    if target.selector:
        # Fall back to scanner query
        result = scanner.exists(target.selector)
        if result.exists:
            return result.element_id
    
    if target.fallback:
        return resolve_target(target.fallback, elements, patterns)
    
    return TargetNotFound(target)
```

### 5.3 Error Handling

**Step-Level Errors**

| Error | Default Behavior | Configurable |
|-------|------------------|--------------|
| Target not found | Fail intent | Retry after re-scan |
| Element not visible | Scroll into view, retry | Skip or fail |
| Element disabled | Wait for enabled | Timeout or fail |
| Click intercepted | Use force click | Fail |
| Type failed | Clear and retry | Fail |
| Timeout | Fail | Extend or skip |

**Intent-Level Error Handling**

```yaml
# In intent definition
options:
  retry:
    max_attempts: 3
    delay: 1s
    on: [target_not_found, element_stale]
  
  on_error:
    target_not_found: 
      action: rescan_and_retry
    element_disabled:
      action: wait
      timeout: 5s
    default:
      action: fail
```

### 5.4 Checkpointing

Long intents can define checkpoints for recovery:

```yaml
intent: checkout
steps:
  - action: click
    target: { text: "Checkout" }
    
  - checkpoint: shipping_started
  
  - action: fill_form
    target: { pattern: shipping_form }
    data: $shipping
    
  - action: click
    target: { role: submit }
    
  - checkpoint: payment_started
  
  - action: fill_form
    target: { pattern: payment_form }
    data: $payment
```

On failure, the engine reports which checkpoint was reached:

```
error checkout: payment form submission failed

# checkpoint
payment_started

# hint
Shipping completed successfully. Retry from payment step:
  checkout --resume payment_started --payment {...}
```

---

## 6. Pattern-Intent Mapping

### 6.1 Pattern Detection Triggers

When the scanner detects patterns, the engine evaluates which intents become available:

```
┌─────────────────────────────────────────────────────────────┐
│  Scanner Patterns                                           │
│                                                             │
│  login_form: {email: 1, password: 2, submit: 3}            │
│  search_form: {input: 5, submit: 6}                         │
│  cookie_banner: {accept: 8, reject: 9}                      │
└─────────────────────────┬───────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────────┐
│  Intent Availability Evaluation                             │
│                                                             │
│  login:          ✓ ready (login_form detected)             │
│  search:         ✓ ready (search_form detected)            │
│  accept_cookies: ✓ ready (cookie_banner detected)          │
│  checkout:       ✗ unavailable (no cart_summary)           │
│  subscribe:      ✗ unavailable (no subscription_form)      │
└─────────────────────────────────────────────────────────────┘
```

### 6.2 Availability Response

Observations include available intents:

```
observe

@ example.com/login "Sign In"

[1] input/email "Email" {required}
[2] input/password "Password" {required}
[3] button/submit "Sign In" {primary}
[4] checkbox "Remember me"
[5] link "Forgot password?"

# patterns
- login_form: email=[1] password=[2] submit=[3] remember=[4]

# available intents
- login <username> <password>: ready
- fill_form <data>: ready
```

### 6.3 Intent Suggestions

The engine can suggest intents based on context:

```
@ amazon.com/dp/B08N5WRWNW "Product Page"

[1] heading "Wireless Mouse"
[2] text "$29.99"
[3] select "Quantity" [1,2,3,4,5]
[4] button/primary "Add to Cart"
[5] button "Buy Now"
[6] button "Add to Wishlist"

# patterns
- product_page: title=[1] price=[2] quantity=[3] add_cart=[4] buy_now=[5]
- wishlist_button: [6]

# available intents
- add_to_cart [--quantity <n>]: ready
- buy_now: ready
- add_to_wishlist: ready

# suggested
Based on product_page pattern, you can:
  add_to_cart              # Add current product to cart
  add_to_cart --quantity 2 # Add multiple
  buy_now                  # Proceed directly to checkout
```

---

## 7. Agent-Defined Intents

### 7.1 Define Command

Agents can define intents during a session:

**Syntax**
```
define <name>:
  [description: "<description>"]
  [parameters: <parameter list>]
  steps:
    <step list>
  [success: <conditions>]
```

**Example**
```
define add_to_wishlist:
  description: "Add current product to wishlist"
  steps:
    - click "Add to Wishlist" or click "♡" or click "Save for later"
    - wait visible { text_contains: [added, saved, wishlist] }
  success:
    - text_contains: [added, saved, wishlist]
```

**Response**
```
ok define add_to_wishlist

# registered
Intent 'add_to_wishlist' available for this session.

# usage
add_to_wishlist
```

### 7.2 Step Syntax Shortcuts

Agent-defined intents support a simplified step syntax:

```
# Multiple target fallbacks
click "Add to Wishlist" or click "♡" or click "Save"
  → try: [{click "Add to Wishlist"}, {click "♡"}, {click "Save"}]

# Implicit wait
wait visible toast
  → wait condition: {visible: {text: "toast"}}

# Type shorthand  
type email "user@test.com"
  → action: type, target: {role: email}, text: "user@test.com"

# Combined fill
fill name="John" email="john@test.com"
  → action: fill_form, data: {name: "John", email: "john@test.com"}
```

### 7.3 Parameterized Definitions

```
define review_product:
  parameters:
    - rating: number, required
    - text: string, required
  steps:
    - click "Write a review"
    - wait visible review_form
    - click star[$rating]
    - type review_text $text
    - click "Submit"
  success:
    - text_contains: "review submitted"
```

**Usage**
```
review_product --rating 5 --text "Excellent product!"
```

### 7.4 Session Management

```
# List session intents
intents --session

# Session intents:
# - add_to_wishlist (defined 5 mins ago)
# - review_product (defined 2 mins ago)

# Remove session intent
undefine add_to_wishlist

# Clear all session intents
intents --clear-session
```

### 7.5 Promotion to Persistent

```
# Export to file
export add_to_wishlist --out ~/.lemmascope/intents/add_to_wishlist.yaml

ok export add_to_wishlist

# written
~/.lemmascope/intents/add_to_wishlist.yaml

# The intent will now load automatically on startup.
```

---

## 8. Intent Learning

### 8.1 Observation Mode

The Intent Learner watches agent command sequences and identifies patterns:

```
┌─────────────────────────────────────────────────────────────┐
│  Learner observes command sequences across sessions:        │
│                                                             │
│  Session 1:                                                 │
│    click "Write a review"                                   │
│    wait 2s                                                  │
│    click [star_5]                                           │
│    type [14] "Great product, highly recommend!"             │
│    click "Submit Review"                                    │
│                                                             │
│  Session 2:                                                 │
│    click "Write a review"                                   │
│    click [star_4]                                           │
│    type [14] "Good quality, fast shipping"                  │
│    click "Submit"                                           │
│                                                             │
│  Session 3:                                                 │
│    click "Leave a Review"     # text variation              │
│    click [star_5]                                           │
│    type [review_textarea] "Amazing!"                        │
│    click "Post Review"        # text variation              │
└─────────────────────────────────────────────────────────────┘
```

### 8.2 Pattern Recognition

The learner identifies:

**Structural Similarity**
- Same sequence of action types (click → click → type → click)
- Similar target patterns (text containing "review", star elements, textarea, submit)

**Parameterizable Elements**
- Star rating varies (4, 5) → parameter `rating`
- Review text varies → parameter `text`

**Text Variations**
- "Write a review" / "Leave a Review" → same intent trigger
- "Submit Review" / "Submit" / "Post Review" → same submit action

### 8.3 Intent Proposal

When confidence threshold is met:

```
# discovered intent proposal

Intent Learner has identified a repeated pattern:

  intent: write_review
  confidence: 0.89
  observations: 3
  
  inferred parameters:
    - rating: number (values seen: 4, 5)
    - text: string (variable content)
  
  inferred steps:
    1. click { text_contains: [review, write] }
    2. click { class_contains: star, index: $rating }
    3. type { role: textarea } $text
    4. click { text_contains: [submit, post], role: submit }
  
  Accept? [yes / no / refine]
```

### 8.4 Refinement Interface

```
refine write_review

# Current definition:
intent: write_review
parameters:
  - rating: number
  - text: string
steps:
  - click { text_contains: [review, write] }
  - click { class_contains: star, index: $rating }
  - type { role: textarea } $text
  - click { text_contains: [submit, post] }

# Commands:
#   add param <name>:<type>     Add parameter
#   remove param <name>         Remove parameter
#   edit step <n>               Modify step
#   add step <position>         Insert step
#   remove step <n>             Delete step
#   test                        Test current definition
#   save                        Accept and save
#   cancel                      Discard changes

> add param title:string --optional

# Updated:
parameters:
  - rating: number
  - text: string
  - title: string (optional)

> edit step 3

# Step 3: type { role: textarea } $text
# New definition:

> type { pattern: review_form.body } $text

# Updated step 3: type { pattern: review_form.body } $text

> test --rating 5 --text "Test review"

# Testing write_review...
✓ Step 1: click "Write a review"
✓ Step 2: click [star_5]
✓ Step 3: type [14] "Test review"
✓ Step 4: click "Submit Review"
✓ Success: text "review submitted" found

> save

ok save write_review

# Intent 'write_review' saved to discovered intents.
# Use 'export write_review' to persist to file.
```

### 8.5 Learning Configuration

```yaml
# ~/.lemmascope/config.yaml

intent_learner:
  enabled: true
  
  # Minimum observations before proposing
  min_observations: 3
  
  # Minimum confidence to propose
  min_confidence: 0.75
  
  # How to handle proposals
  auto_accept: false      # Require explicit approval
  
  # What to learn from
  learn_from:
    - direct_commands     # click, type, etc.
    - failed_intents      # Learn from what agents try
  
  # Scope
  persistence: session    # session | user | global
  
  # Privacy
  exclude_fields:
    - password
    - credit_card
    - ssn
```

---

## 9. Site-Specific Intent Packs

### 9.1 Pack Structure

Intent packs bundle site-specific patterns and intents:

```
intent-packs/
├── github.com/
│   ├── pack.yaml           # Pack metadata
│   ├── patterns.yaml       # Site-specific patterns
│   └── intents/
│       ├── star_repo.yaml
│       ├── fork_repo.yaml
│       ├── create_issue.yaml
│       └── create_pr.yaml
├── amazon.com/
│   ├── pack.yaml
│   ├── patterns.yaml
│   └── intents/
│       ├── add_to_cart.yaml
│       ├── checkout.yaml
│       └── track_order.yaml
└── google.com/
    ├── pack.yaml
    ├── patterns.yaml
    └── intents/
        ├── search.yaml
        └── advanced_search.yaml
```

### 9.2 Pack Metadata

```yaml
# intent-packs/github.com/pack.yaml

pack: github.com
version: 1.2.0
description: "Intent pack for GitHub"
author: lemmascope-community
license: MIT

domains:
  - github.com
  - www.github.com
  - gist.github.com

requires:
  lemmascope: ">=1.0.0"

patterns:
  - patterns.yaml

intents:
  - intents/*.yaml

# Auto-load when visiting these URLs
auto_load:
  - "https://github.com/*"
  - "https://gist.github.com/*"
```

### 9.3 Site-Specific Patterns

```yaml
# intent-packs/github.com/patterns.yaml

patterns:
  repo_header:
    description: "Repository action buttons"
    detection:
      container: ".pagehead-actions"
    elements:
      star:
        selector: "button[data-ga-click*='star']"
        text_contains: ["Star", "Unstar"]
      fork:
        selector: "a[data-ga-click*='fork']"
        text_contains: "Fork"
      watch:
        selector: "button[data-ga-click*='watch']"
    
  issue_form:
    description: "New issue creation form"
    detection:
      url_contains: "/issues/new"
    elements:
      title:
        selector: "#issue_title"
        role: text
      body:
        selector: "#issue_body"
        role: textarea
      labels:
        selector: "#labels-select-menu"
        role: select
      submit:
        selector: "button[type='submit']"
        text_contains: "Submit"

  pr_form:
    description: "Pull request creation form"
    detection:
      url_contains: "/compare/"
    elements:
      title:
        selector: "#pull_request_title"
      body:
        selector: "#pull_request_body"
      submit:
        selector: "button.btn-primary"
        text_contains: ["Create pull request", "Create PR"]
```

### 9.4 Site-Specific Intents

```yaml
# intent-packs/github.com/intents/create_issue.yaml

intent: create_issue
version: 1
description: "Create a new GitHub issue"
pack: github.com

triggers:
  patterns:
    - issue_form
  urls:
    - "*/issues/new*"

parameters:
  - name: title
    type: string
    required: true
    description: "Issue title"
  - name: body
    type: string
    required: true
    description: "Issue body/description"
  - name: labels
    type: array
    required: false
    description: "Labels to apply"
  - name: assignees
    type: array
    required: false
    description: "Users to assign"

steps:
  - action: type
    target: { pattern: issue_form.title }
    text: $title
    
  - action: type
    target: { pattern: issue_form.body }
    text: $body
    
  - branch:
      if: $labels
      then:
        - action: click
          target: { pattern: issue_form.labels }
        - loop:
            over: $labels
            as: label
            steps:
              - action: click
                target: { text: $label }
        - action: press
          key: Escape
          
  - branch:
      if: $assignees
      then:
        - action: click
          target: { text: "Assignees" }
        - loop:
            over: $assignees
            as: assignee
            steps:
              - action: type
                target: { role: search }
                text: $assignee
              - action: click
                target: { text: $assignee }
        - action: press
          key: Escape
    
  - action: click
    target: { pattern: issue_form.submit }
    
  - action: wait
    condition:
      url_matches: ".*/issues/\\d+$"
    timeout: 10s

success:
  conditions:
    - url_matches: ".*/issues/\\d+$"
  extract:
    issue_number:
      from: url
      pattern: "/issues/(\\d+)$"
    issue_url:
      from: url

failure:
  conditions:
    - pattern_exists: form_error
  recovery:
    - action: observe
```

### 9.5 Pack Loading

```
# List available packs
packs

# Available intent packs:
# ✓ github.com (v1.2.0) - loaded
# ✓ amazon.com (v1.1.0) - loaded
#   twitter.com (v1.0.0) - available
#   linkedin.com (v0.9.0) - available

# Load a pack manually
pack load twitter.com

# Unload a pack
pack unload github.com

# Update packs
pack update

# Install community pack
pack install stripe.com --source community
```

### 9.6 Auto-Loading

When navigating to a URL, the engine checks for matching packs:

```
goto github.com/anthropics/claude

# auto-loading
Loading intent pack: github.com (v1.2.0)

@ github.com/anthropics/claude "anthropics/claude"
...

# available intents (from pack)
- star_repo: ready
- fork_repo: ready
- create_issue: navigate to /issues/new
- create_pr: navigate to /compare
```

---

## 10. Response Format

### 10.1 Success Response

```
ok <intent> [<summary>]

# actions
<action log>

# changes
<change notation>

# result (if extract defined)
<extracted data>
```

**Example**
```
ok login "user@example.com"

# actions
scan
type [1] "user@example.com"
type [2] "••••••••"
click [3] "Sign in"
wait navigation (2.3s)
scan

# changes
~ url: github.com/login → github.com
~ title: "Sign in" → "GitHub"
- [1] input/email
- [2] input/password
- [3] button/submit
- login_form
+ [1] nav "Dashboard"
+ user_menu

# result
authenticated: true
redirect: github.com
```

### 10.2 Failure Response

```
error <intent>: <message>

# actions
<action log up to failure>

# checkpoint (if applicable)
<last successful checkpoint>

# context
<relevant page state>

# hint
<recovery suggestions>
```

**Example**
```
error checkout: payment form validation failed

# actions
click [5] "Proceed to checkout"
wait shipping_form (1.2s)
fill_form shipping [success]
click [12] "Continue"
wait payment_form (1.8s)
fill_form payment
click [18] "Place order"

# checkpoint
payment_started

# context
Form errors detected:
  - "Card number is invalid"
  - "Expiration date is required"

# hint
Payment form has validation errors. Check:
  - card_number: format should be 16 digits
  - expiration: format should be MM/YY
Retry: checkout --resume payment_started --payment {...}
```

### 10.3 Partial Success Response

```
partial <intent>: <summary>

# completed
<successful steps>

# failed
<failed steps>

# result
<partial data extracted>

# hint
<how to complete remaining steps>
```

---

## 11. Configuration

### 11.1 Engine Configuration

```yaml
# ~/.lemmascope/config.yaml

intent_engine:
  # Intent resolution
  resolution:
    tier_priority: [user, pack, core, builtin]
    allow_discovered: true
    strict_mode: false  # Fail on ambiguous targets vs. best-effort
  
  # Execution
  execution:
    default_timeout: 30s
    step_timeout: 10s
    max_retries: 3
    retry_delay: 1s
    parallel_steps: false  # Future feature
  
  # Verification  
  verification:
    verify_success: true
    verify_failure: true
    rescan_after_action: auto  # auto | always | never
  
  # Logging
  logging:
    log_actions: true
    log_changes: true
    redact_sensitive: true  # Hide passwords in logs
    
  # Learning
  learning:
    enabled: true
    min_observations: 3
    auto_propose: true
    
  # Packs
  packs:
    auto_load: true
    pack_paths:
      - ./intent-packs
      - ~/.lemmascope/packs
    community_repo: https://packs.lemmascope.dev
```

### 11.2 Per-Intent Options

Override engine defaults for specific intents:

```
login "user" "pass" --timeout 60s --no-verify

checkout --retry 5 --checkpoint-resume shipping_complete
```

### 11.3 Runtime Configuration

```
# View current config
config show intent_engine

# Modify at runtime
config set intent_engine.execution.default_timeout 60s
config set intent_engine.learning.enabled false

# Reset to defaults
config reset intent_engine.execution
```

---

## 12. Extensibility

### 12.1 Custom Step Actions

Packs can define custom actions:

```yaml
# In pack definition
custom_actions:
  github_api:
    description: "Make authenticated GitHub API call"
    parameters:
      - endpoint: string
      - method: string
      - body: object
    implementation:
      type: javascript
      code: |
        async function(params, context) {
          const token = await context.getCookie('github_token');
          const response = await fetch(`https://api.github.com${params.endpoint}`, {
            method: params.method,
            headers: { 'Authorization': `token ${token}` },
            body: JSON.stringify(params.body)
          });
          return response.json();
        }
```

### 12.2 Custom Conditions

```yaml
custom_conditions:
  github_authenticated:
    description: "Check if logged into GitHub"
    implementation:
      type: pattern_check
      patterns:
        - user_menu
      # OR
      type: javascript
      code: |
        function(context) {
          return context.patterns.includes('user_menu');
        }
```

### 12.3 Hooks

```yaml
# Pack-level hooks
hooks:
  before_intent:
    - action: dismiss_popups
      condition: { pattern_exists: modal_dialog }
      
  after_navigate:
    - action: wait
      condition: idle
      timeout: 2s
      
  on_error:
    - action: screenshot
      output: "./errors/{timestamp}.png"
```

---

## 13. Security Considerations

### 13.1 Sensitive Data Handling

**Password Masking**
```
# In responses, passwords are masked
type [2] "••••••••"  # Never show actual password
```

**Sensitive Fields**

The engine recognizes sensitive field types and:
- Never logs their values
- Masks them in responses
- Excludes them from learning

```yaml
sensitive_fields:
  - password
  - credit_card
  - card_number
  - cvv
  - ssn
  - social_security
  - secret
  - token
  - api_key
```

### 13.2 Intent Validation

Loaded intents are validated for:
- Schema compliance
- No arbitrary code execution (unless explicitly enabled)
- Bounded loops (max iterations)
- Reasonable timeouts

### 13.3 Pack Trust

```yaml
# Trust levels for packs
pack_trust:
  builtin: full          # Can do anything
  official: verified     # Signed by Lemmascope
  community: sandboxed   # Limited capabilities
  local: configurable    # User decides
```

**Sandboxed Capabilities**
- No `execute` steps (arbitrary JavaScript)
- No custom actions with JavaScript
- No file system access
- Network limited to current domain

---

## 14. Future Directions

### 14.1 Goal-Level Commands

Natural language goals that the engine plans automatically:

```
goal: "Purchase the cheapest flight to NYC next Friday"

# planning
Analyzing goal...
Required intents: search, filter, select, checkout
Estimated steps: 12-15

# plan
1. search "flights to NYC"
2. filter by date "next Friday"
3. sort by price
4. select cheapest option
5. checkout with saved payment method

Execute? [yes / modify / cancel]
```

### 14.2 Multi-Page Flows

Intents that span multiple page navigations:

```yaml
intent: complete_purchase
type: flow

pages:
  - name: cart
    url_pattern: "*/cart*"
    intents: [verify_cart, apply_coupon]
    next: checkout
    
  - name: checkout
    url_pattern: "*/checkout*"
    intents: [fill_shipping, fill_payment]
    next: confirmation
    
  - name: confirmation
    url_pattern: "*/confirmation*"
    extract: [order_number, total]
```

### 14.3 Collaborative Learning

Share learned intents across users:

```
# Contribute a refined intent
publish write_review --to community

# Discover community intents
search intents "newsletter subscribe"

# Install from community
install intent subscribe_newsletter --author trusted_user
```

### 14.4 Intent Composition

Build complex intents from simpler ones:

```yaml
intent: setup_new_account
compose:
  - intent: signup
    params: { email: $email, password: $password }
  - intent: verify_email
    params: { email: $email }
  - intent: complete_profile
    params: { name: $name, bio: $bio }
  - intent: configure_notifications
    params: { preferences: $notification_prefs }
```

---

## Appendix A: Built-in Intent Quick Reference

| Intent | Syntax | Description |
|--------|--------|-------------|
| `login` | `login <user> <pass>` | Authenticate |
| `logout` | `logout` | End session |
| `search` | `search <query>` | Submit search |
| `accept_cookies` | `accept_cookies` | Dismiss cookie banner |
| `dismiss_popups` | `dismiss_popups` | Close modals |
| `fill_form` | `fill_form <data>` | Fill form fields |
| `submit_form` | `submit_form` | Submit current form |
| `scroll_to` | `scroll_to <target>` | Scroll element into view |

---

## Appendix B: Intent Definition Schema

```yaml
# JSON Schema for intent definitions
$schema: "https://lemmascope.dev/schemas/intent-v1.json"

type: object
required: [intent, version, steps]
properties:
  intent:
    type: string
    pattern: "^[a-z][a-z0-9_]*$"
  version:
    type: string
    pattern: "^\\d+(\\.\\d+)*$"
  description:
    type: string
  author:
    type: string
  tags:
    type: array
    items: { type: string }
  triggers:
    type: object
    properties:
      patterns: { type: array, items: { type: string } }
      keywords: { type: array, items: { type: string } }
      urls: { type: array, items: { type: string } }
  parameters:
    type: array
    items:
      type: object
      required: [name, type]
      properties:
        name: { type: string }
        type: { enum: [string, number, boolean, object, array] }
        required: { type: boolean }
        default: {}
        description: { type: string }
  steps:
    type: array
    items: { $ref: "#/definitions/step" }
  success:
    type: object
  failure:
    type: object
  options:
    type: object
```

---

## Appendix C: Error Codes

| Code | Description |
|------|-------------|
| `INTENT_NOT_FOUND` | Intent name not recognized |
| `INTENT_UNAVAILABLE` | Intent triggers not satisfied |
| `PARAMETER_MISSING` | Required parameter not provided |
| `PARAMETER_INVALID` | Parameter type mismatch |
| `TARGET_NOT_FOUND` | Could not resolve target to element |
| `TARGET_AMBIGUOUS` | Multiple elements match target |
| `STEP_FAILED` | Individual step execution failed |
| `TIMEOUT` | Intent or step exceeded timeout |
| `VERIFICATION_FAILED` | Success conditions not met |
| `CHECKPOINT_INVALID` | Cannot resume from specified checkpoint |
| `PACK_LOAD_FAILED` | Could not load intent pack |
| `DEFINITION_INVALID` | Intent definition schema violation |

---

*Document Version: 1.0*  
*Last Updated: January 2025*
