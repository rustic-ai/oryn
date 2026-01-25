# OIL Grammar Compliance Specification

## Version 1.8.1
**Date:** January 2026

This document defines the **cross-language conformance contract** for OIL (Oryn Intent Language) implementations.

**Normative artifacts (v1.8.1):**
- Grammar (Phase 2): `OIL.v1.8.1.canonical.fixed.abnf`
- Test vectors: `oil-test-vectors.v1.8.1.full.yaml`
- Summary (informational): `oil-test-vectors-summary.v1.8.1.full.json`

---

## 1. Processing model (normative)

OIL processing is a 3-step pipeline:

```
RAW INPUT
  → Phase 1: tokenize + normalize
  → CANONICAL TEXT (v1.8.1)
  → Phase 2: parse (ABNF)
  → AST
  → Phase 3: semantic validation + AST normalization
```

A compliant implementation MUST:
1. Produce canonical text from raw input per §2–§4.
2. Parse canonical text per the ABNF grammar.
3. Apply Phase 3 semantic policies per §5 (and as required by vectors).
4. Match the expected outcomes in the published vectors.

When this spec and a vector disagree, **the vector wins for that case** (and the spec should be updated).

---

## 2. Canonical text contract (Phase 1 output)

### 2.1 Canonical keywords
- Command verbs MUST be lowercase canonical keywords (e.g., `goto`, not `navigate`).
- Subcommands MUST be canonical keywords (e.g., `headers set`, `session new`, `tab switch`).
- Command aliases MAY be accepted in raw input, but MUST normalize to canonical verbs in canonical text.

### 2.2 Options
- Canonical options MUST use the `--kebab-case` form only (e.g., `--timeout`, `--respond-file`).
- Implementations MAY accept `-opt` or documented bare forms in raw input, but MUST normalize them to `--opt` in canonical text.

### 2.3 Strings
- Canonical strings MUST use double quotes: `"..."`.
- Phase 1 MAY accept single quotes in raw input, but MUST normalize them to double quotes.
- Required escape sequences in canonical strings:
  - `\"` (quote), `\\` (backslash), `\n`, `\r`, `\t`

### 2.4 Whitespace and newlines
- Raw input MAY contain multiple spaces/tabs; Phase 1 MUST normalize token separators to a single space in canonical text.
- Raw input MAY contain empty lines; canonical output MAY omit them or preserve them as empty lines (vectors define expectations).
- Inputs MAY be `LF` or `CRLF`.

---

## 3. Comment rule (context-sensitive `#`)

A `#` begins a comment only when it appears:
- at the start of a line (after optional leading whitespace), or
- **after at least one whitespace character** following a token separator.

Comments run to end-of-line.

Implications:
- `goto example.com#frag` treats `#frag` as **data** (URL fragment).
- `click 5#comment` is **not** a comment (parse error); write `click 5 #comment`.
- `#` inside quoted strings is always literal.

**Canonical representation:** canonical lines MAY retain trailing comments (`… ␠#comment`). The Phase 2 grammar MUST accept optional trailing comments.

---

## 4. JSON payloads (canonical requirement)

Some commands accept JSON payloads (e.g., `goto --headers`, `headers set`, `intercept --respond`).

**In canonical text, JSON payloads MUST be passed as quoted OIL strings**, for example:

- `goto api.example.com --headers "{\"Authorization\": \"Bearer token\"}"`
- `headers set "{\"Authorization\": \"Bearer token\"}"`
- `intercept "*/api/*" --respond "{\"name\":\"Test\"}"`

Phase 2 parsing treats JSON payloads as normal OIL strings.
Implementations MAY validate JSON in Phase 3, but JSON validation is **not required** for grammar conformance unless a vector requires it.

---

## 5. Semantic policies (Phase 3)

Vectors are authoritative for edge cases. The following policies apply unless vectors override them:

### 5.1 Duplicate options
- Boolean flags: repeated occurrences are allowed; presence=true.
- Value options: repeated occurrences are allowed; **last wins**.
- Mutually exclusive combinations: MUST produce a **semantic error**.

### 5.2 Target chain associativity
Target chains using relations (`near`, `inside`, `after`, `before`, `contains`) MUST associate **rightward**, e.g.:

`"Add" near "Product" inside "Modal"`
→ `Add near (Product inside Modal)`

---

## 6. Error model

At minimum, implementations MUST distinguish:
- **Parse errors**: canonical text does not match ABNF.
- **Semantic errors**: parses but violates semantic rules (conflicts, invalid combinations).

Vectors may further classify errors (normalization vs parse vs semantic); implementers SHOULD expose those distinctions where possible.

---

## 7. Compliance levels

### 7.1 Core (v1.7 baseline)
Implements the v1.7 command surface (navigation/observation/actions/wait/extract/session basic/tabs/intents/packs/utilities) and passes the corresponding vectors.

### 7.2 Enhanced (v1.8.1)
Adds all v1.8.1 command families and enhancements, and passes the v1.8.1 vectors, including:
- Session management: `sessions`, `session new/close`, `state save/load`, `headers set/clear`
- Network: `intercept`, `requests`
- Console/Errors: `console`, `errors`
- Frames: `frames`, `frame`
- Dialogs: `dialog …`
- Viewport/Device/Media: `viewport`, `device`, `devices`, `media`
- Recording: `trace`, `record`, `highlight`
- Action enhancements: `keydown`, `keyup`, `keys`, `box`
- Wait enhancements: `wait ready/until/items`
- Observation enhancement: `observe --positions`
- Navigation enhancement: `goto --headers`

---

## 8. Using the vectors (runner requirements)

A conformance runner MUST, for each vector:
1. Run Phase 1 normalization on `raw`.
2. Compare produced canonical text with the vector’s `canonical`.
3. Parse canonical text (Phase 2) and compare `ast` when provided.
4. Apply semantics (Phase 3) and compare expected semantic results / errors when provided.

---

## 9. Command reference (v1.8.1 canonical surface)

This section lists the canonical command surface that Phase 2 MUST parse.

### Navigation
- `goto <url> [--headers <json-string>] [--timeout <duration>]`
- `back` / `forward` / `refresh [--hard]` / `url`

### Observation
- `observe [--full|--minimal|--viewport|--hidden|--positions] [--near "<text>"] [--timeout <duration>]`
- `html [--selector "<selector>"]`
- `text [--selector "<selector>"] [<target>]`
- `title`
- `screenshot [--output <path>] [--format png|jpeg|webp] [--fullpage] [<target>]`
- `box <target>`

### Actions
- `click <target> [--double|--right|--middle|--force|--ctrl|--shift|--alt] [--timeout <duration>]`
- `type <target> "<text>" [--append|--enter|--clear] [--delay <ms>] [--timeout <duration>]`
- `clear <target>`
- `press <key-combo>`
- `keydown <key>` / `keyup <key|all>` / `keys`
- `select <target> (<string>|<number>)`
- `check <target>` / `uncheck <target>`
- `hover <target>` / `focus <target>`
- `scroll [up|down|left|right] [--amount <n>] [--page] [--timeout <duration>] [<target>]`
- `submit [<target>]`

### Wait
- `wait load|idle|navigation|ready`
- `wait visible|hidden <target>`
- `wait exists|gone|url|until "<expr>"`
- `wait items "<selector>" <count>`
- `wait … [--timeout <duration>]`

### Extract
- `extract links|images|tables|meta|text`
- `extract css("<selector>")`
- `extract … [--selector "<selector>"] [--format json|csv|text]`

### Session / State / Headers
- `cookies list|get|set|delete|clear …`
- `storage list|get|set|delete|clear … [--local|--session]`
- `sessions`
- `session [<name>] | session new <name> [--mode embedded|headless|remote] | session close <name>`
- `state save|load <path> [--cookies-only] [--domain "<domain>"] [--include-session] [--merge]`
- `headers [<domain>] | headers set [<domain>] "<json-string>" | headers clear [<domain>]`

### Tabs
- `tabs`
- `tab new <url>` / `tab switch <n>` / `tab close [<n>]`

### Intents
- `login "<user>" "<pass>" [--no-submit] [--wait <duration>] [--timeout <duration>]`
- `search "<query>" [--submit enter|click|auto] [--wait <duration>] [--timeout <duration>]`
- `dismiss popups|modals|modal|banner|<identifier>|"<text>"`
- `accept_cookies`
- `scroll until <target> [--amount <n>] [--page] [--timeout <duration>]`

### Packs
- `packs` / `pack load|unload <name>`
- `intents [--session]`
- `define <name>:` / `undefine <name>`
- `export <name> [--out <path>]`
- `run <name> [--param <value>]…`

### Network
- `intercept clear [<pattern>]`
- `intercept "<pattern>" [--block] [--respond "<json-string>"] [--respond-file <path>] [--status <n>]`
- `requests [--filter "<text>"] [--method GET|POST|…] [--last <n>]`

### Console / Errors
- `console [--level log|warn|error|info] [--filter "<text>"] [--last <n>]`
- `console clear`
- `errors [--last <n>]`
- `errors clear`

### Frames
- `frames`
- `frame main|parent|<target>`

### Dialogs
- `dialog accept ["<text>"]`
- `dialog dismiss`
- `dialog auto accept|dismiss|off`

### Viewport / Device / Media
- `viewport <w> <h>`
- `device "<name>" | device reset`
- `devices`
- `media reset | media color-scheme dark|light | media reduced-motion reduce|no-preference`

### Recording
- `trace start [<path>]` / `trace stop [<path>]`
- `record start <path> [--quality low|medium|high]` / `record stop`
- `highlight <target> [--duration <duration>] [--color "<name>"]` / `highlight clear`

### Utility
- `pdf <path> [--format A4|Letter|Legal|Tabloid] [--landscape] [--margin <n|"..."]`
- `learn status|save|discard|show …`
- `help [<topic>]`
- `exit`

