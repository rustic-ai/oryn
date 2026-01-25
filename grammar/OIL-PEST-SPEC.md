# OIL PEG Parser Specification (pest)

## Version 1.7.0

This document describes the pest PEG grammar implementation for OIL (Oryn Intent Language) and provides validation against the official specification.

---

## 1. Overview

The `oil.pest` grammar file implements a complete parser for the OIL language using the pest parser generator for Rust. This grammar parses **canonical OIL text** produced by Phase 1 normalization.

### 1.1 Two-Phase Processing Model

```
RAW INPUT → Phase 1 (Normalization) → CANONICAL TEXT → Phase 2 (pest Grammar) → AST
```

**Phase 1** (not in grammar, handled externally):
- Case folding (commands and options to lowercase)
- Alias resolution (`navigate` → `goto`, `scan` → `observe`, etc.)
- Quote normalization (single quotes → double quotes)
- Option normalization (`-hard` → `--hard`)
- Whitespace normalization

**Phase 2** (this grammar):
- Parses canonical text per ABNF specification
- Produces parse tree for AST construction

---

## 2. Grammar Structure

### 2.1 File Organization

```
oil.pest
├── Top-Level Structure (lines, comments)
├── Command Dispatch
├── Navigation Commands (goto, back, forward, refresh, url)
├── Observation Commands (observe, html, text, title, screenshot)
├── Action Commands (click, type, clear, press, select, check/uncheck, hover, focus, scroll, submit)
├── Wait Commands
├── Extraction Commands
├── Session Commands (cookies, storage)
├── Tab Commands
├── Intent Commands (login, search, dismiss, accept_cookies, scroll_until)
├── Pack Management Commands (packs, pack, intents, define, undefine, export, run)
├── Utility Commands (pdf, learn, exit, help)
├── Targets (IDs, roles, CSS/XPath, text, relational)
├── Primitives (strings, URLs, paths, identifiers, numbers, durations)
```

### 2.2 Key Design Decisions

#### Context-Sensitive `#` Handling

The grammar implements context-sensitive `#` handling as specified in SPEC-OIL-COMPLIANCE.md v1.7.0:

| Token Type | `#` Allowed? | Rationale |
|------------|--------------|-----------|
| `url_bare` | ✓ | URL fragments: `example.com#section` |
| `path_bare` | ✓ | Filenames: `/tmp/issue#123.txt` |
| `string_value` | ✓ | Inside quotes: `"Button#1"` |
| `target_id` | ✗ | IDs are `DIGIT+` only |
| `identifier` | ✗ | No `#` in names |
| `comment` | ✓ | After whitespace at line level |

#### Relational Target Associativity

The grammar parses relational chains flat; the AST builder applies right-to-left associativity:

```
Input: click "A" near "B" inside "C"
Parse: target_chain = "A" near "B" inside "C"  (flat)
AST:   A near (B inside C)                      (right-associative)
```

---

## 3. Token Character Sets

### 3.1 URL Bare (includes `#`)

```pest
url_char = {
    ASCII_ALPHA | ASCII_DIGIT |
    "-" | "_" | "." | "/" | ":" |
    "?" | "=" | "&" | "%" | "@" | "~" | "+" |
    "#"  // URL fragments allowed!
}
```

### 3.2 Path Bare (includes `#`)

```pest
path_char = {
    ASCII_ALPHA | ASCII_DIGIT |
    "-" | "_" | "." | "/" | "~" |
    ":" | "\\" |  // Windows paths
    "#"  // Allow # in paths
}
```

### 3.3 Target ID (excludes `#`)

```pest
target_id = @{ ASCII_DIGIT+ }
```

### 3.4 Identifier (excludes `#`)

```pest
identifier = @{ (ASCII_ALPHA | "_") ~ (ASCII_ALPHA | ASCII_DIGIT | "_" | "-")* }
```

---

## 4. Test Vector Compliance Matrix

### 4.1 URL Fragment Tests

| Vector ID | Input | Expected Result | Grammar Status |
|-----------|-------|-----------------|----------------|
| `url-with-fragment` | `goto example.com#section` | ✓ Valid | ✅ PASS |
| `url-fragment-and-comment` | `goto example.com#section #comment` | ✓ Valid | ✅ PASS |
| `url-no-fragment-with-comment` | `goto example.com #comment` | ✓ Valid | ✅ PASS |
| `url-multiple-hashes` | `goto example.com#a#b` | ✓ Valid | ✅ PASS |
| `tab-new-with-fragment` | `tab new docs.example.com#api` | ✓ Valid | ✅ PASS |

### 4.2 Target Comment Tests

| Vector ID | Input | Expected Result | Grammar Status |
|-----------|-------|-----------------|----------------|
| `click-with-comment` | `click 5 #comment` | ✓ Valid | ✅ PASS |
| `click-quoted-target-with-hash` | `click "Button#1"` | ✓ Valid | ✅ PASS |
| `click-quoted-target-and-comment` | `click "Button#1" #comment` | ✓ Valid | ✅ PASS |

### 4.3 Target Error Tests

| Vector ID | Input | Expected Result | Grammar Status |
|-----------|-------|-----------------|----------------|
| `click-id-no-space-before-hash` | `click 5#comment` | ✗ Error | ✅ PASS (fails to parse) |
| `type-id-no-space-before-hash` | `type 3#x "hello"` | ✗ Error | ✅ PASS (fails to parse) |

### 4.4 Path Tests

| Vector ID | Input | Expected Result | Grammar Status |
|-----------|-------|-----------------|----------------|
| `path-with-hash` | `pdf /tmp/issue#123.pdf` | ✓ Valid | ✅ PASS |
| `path-with-hash-and-comment` | `pdf /tmp/file#1.pdf #note` | ✓ Valid | ✅ PASS |
| `screenshot-output-with-hash` | `screenshot --output /tmp/shot#1.png` | ✓ Valid | ✅ PASS |

### 4.5 Duplicate Option Tests (Syntactic)

| Vector ID | Input | Expected Result | Grammar Status |
|-----------|-------|-----------------|----------------|
| `boolean-repeated` | `refresh --hard --hard` | ✓ Valid (syntax) | ✅ PASS |
| `timeout-last-wins` | `click 5 --timeout 5s --timeout 10s` | ✓ Valid (syntax) | ✅ PASS |
| `verbosity-conflict` | `observe --full --minimal` | ✓ Valid (syntax) | ✅ PASS* |
| `storage-type-conflict` | `storage list --local --session` | ✓ Valid (syntax) | ✅ PASS* |

*Note: These parse syntactically; semantic validation rejects conflicting options.

### 4.6 Relational Target Tests

| Vector ID | Input | Expected Result | Grammar Status |
|-----------|-------|-----------------|----------------|
| `simple-near` | `click "Add" near "Product"` | ✓ Valid | ✅ PASS |
| `simple-inside` | `click "Submit" inside "Form"` | ✓ Valid | ✅ PASS |
| `simple-after` | `click "Next" after "Page 1"` | ✓ Valid | ✅ PASS |
| `simple-before` | `click "Back" before "Submit"` | ✓ Valid | ✅ PASS |
| `simple-contains` | `click "Row" contains "Delete"` | ✓ Valid | ✅ PASS |
| `chained` | `click "Add" near "Product" inside "Modal"` | ✓ Valid | ✅ PASS |
| `mixed-types` | `click 5 near "Label"` | ✓ Valid | ✅ PASS |

---

## 5. Command Coverage

### 5.1 Core Commands (Compliance Level: Core)

| Command | Grammar Rule | Status |
|---------|--------------|--------|
| `goto` | `goto_cmd` | ✅ |
| `back` | `back_cmd` | ✅ |
| `forward` | `forward_cmd` | ✅ |
| `refresh` | `refresh_cmd` | ✅ |
| `url` | `url_cmd` | ✅ |
| `observe` | `observe_cmd` | ✅ |
| `click` | `click_cmd` | ✅ |
| `type` | `type_cmd` | ✅ |
| `clear` | `clear_cmd` | ✅ |
| `press` | `press_cmd` | ✅ |
| `check` | `check_cmd` | ✅ |
| `uncheck` | `uncheck_cmd` | ✅ |
| `hover` | `hover_cmd` | ✅ |
| `focus` | `focus_cmd` | ✅ |
| `scroll` | `scroll_cmd` | ✅ |
| `submit` | `submit_cmd` | ✅ |

### 5.2 Extended Commands

| Command | Grammar Rule | Status |
|---------|--------------|--------|
| `wait` | `wait_cmd` | ✅ |
| `extract` | `extraction_cmd` | ✅ |
| `cookies` | `cookies_cmd` | ✅ |
| `storage` | `storage_cmd` | ✅ |
| `tabs` | `tabs_cmd` | ✅ |
| `tab` | `tab_action_cmd` | ✅ |
| `screenshot` | `screenshot_cmd` | ✅ |
| `text` | `text_cmd` | ✅ |
| `html` | `html_cmd` | ✅ |
| `title` | `title_cmd` | ✅ |

### 5.3 Full Commands (Level 3 + Pack Management + Utility)

| Command | Grammar Rule | Status |
|---------|--------------|--------|
| `login` | `login_cmd` | ✅ |
| `search` | `search_cmd` | ✅ |
| `dismiss` | `dismiss_cmd` | ✅ |
| `accept cookies` | `accept_cookies_cmd` | ✅ |
| `scroll until` | `scroll_until_cmd` | ✅ |
| `packs` | `packs_cmd` | ✅ |
| `pack` | `pack_action_cmd` | ✅ |
| `intents` | `intents_cmd` | ✅ |
| `define` | `define_cmd` | ✅ |
| `undefine` | `undefine_cmd` | ✅ |
| `export` | `export_cmd` | ✅ |
| `run` | `run_cmd` | ✅ |
| `pdf` | `pdf_cmd` | ✅ |
| `learn` | `learn_cmd` | ✅ |
| `exit` | `exit_cmd` | ✅ |
| `help` | `help_cmd` | ✅ |

---

## 6. Usage

### 6.1 Rust Integration

Add to `Cargo.toml`:

```toml
[dependencies]
pest = "2.7"
pest_derive = "2.7"
```

Create parser:

```rust
use pest::Parser;
use pest_derive::Parser;

#[derive(Parser)]
#[grammar = "oil.pest"]
pub struct OilParser;

fn parse_command(input: &str) -> Result<...> {
    OilParser::parse(Rule::line, input)
}
```

### 6.2 Example Parse Trees

**Simple command:**
```
Input: "click 5"
Parse Tree:
  line
    command
      action_cmd
        click_cmd
          target
            target_chain
              target_atomic
                target_id: "5"
```

**With comment:**
```
Input: "click 5 #submit"
Parse Tree:
  line
    command
      action_cmd
        click_cmd
          target
            target_chain
              target_atomic
                target_id: "5"
    comment: "#submit"
```

**URL with fragment:**
```
Input: "goto example.com#section"
Parse Tree:
  line
    command
      navigation_cmd
        goto_cmd
          url_value
            url_bare: "example.com#section"
```

---

## 7. Validation Summary

### Test Statistics

| Category | Vectors | Passing | Status |
|----------|---------|---------|--------|
| Navigation | 20+ | 20+ | ✅ |
| Observation | 15+ | 15+ | ✅ |
| Actions | 40+ | 40+ | ✅ |
| Wait | 10+ | 10+ | ✅ |
| Extraction | 10+ | 10+ | ✅ |
| Session | 15+ | 15+ | ✅ |
| Tabs | 8+ | 8+ | ✅ |
| Intents | 15+ | 15+ | ✅ |
| Pack Mgmt | 12+ | 12+ | ✅ |
| Utility | 12+ | 12+ | ✅ |
| Relational | 10+ | 10+ | ✅ |
| Comments | 8+ | 8+ | ✅ |
| Errors | 10+ | 10+ | ✅ |
| Edge Cases | 20+ | 20+ | ✅ |

### Compliance Declaration

This grammar implementation is **COMPLIANT** with:
- OIL.abnf v1.7.0
- SPEC-OIL-COMPLIANCE.md v1.7.0
- oil-test-vectors.yaml v1.0.0

---

## 8. Implementation Notes

### 8.1 Silent Rules

The following rules are silent (underscore prefix) and don't appear in the parse tree:

- `WSP` - Whitespace is consumed but not captured

### 8.2 Atomic Rules

The following rules are atomic (`@` prefix) and capture as a single token:

- `target_id` - Captures complete numeric ID
- `url_bare` - Captures complete URL including fragments
- `path_bare` - Captures complete path including `#`
- `identifier` - Captures complete identifier
- `number` - Captures complete number
- `duration` - Captures duration with unit
- `key_combo` - Captures key combination
- `comment` - Captures complete comment text
- `string_inner` - Captures string contents

### 8.3 Compound Atomic Rules

The following rules are compound atomic (`$` prefix):

- `string_value` - Ensures quotes are included with content

---

## 9. Future Considerations

### 9.1 Error Recovery

The current grammar does not include error recovery rules. Future versions may add:
- Synchronization points for better error messages
- Recovery rules for common mistakes

### 9.2 Performance Optimization

Consider:
- Memoization hints for frequently-matched rules
- Rule ordering optimization for common command paths

---

## Appendix A: Grammar File Location

The grammar file should be placed at:
```
src/oil.pest
```

## Appendix B: ABNF to pest Translation Notes

| ABNF | pest | Notes |
|------|------|-------|
| `rule = ...` | `rule = { ... }` | Standard rule |
| `=` | `=` | Definition |
| `/` | `|` | Alternation |
| `*rule` | `rule*` | Zero or more |
| `1*rule` | `rule+` | One or more |
| `[rule]` | `rule?` | Optional |
| `"literal"` | `"literal"` | Same |
| `%x20-7E` | `'\x20'..'\x7E'` | Range |
| `ALPHA` | `ASCII_ALPHA` | Built-in |
| `DIGIT` | `ASCII_DIGIT` | Built-in |
| `WSP` | `" " | "\t"` | Space/tab |
| `CRLF / LF` | `NEWLINE` | Built-in |

---

*Document Version: 1.7.0*
*Generated: January 2026*
*Grammar Spec: oil.pest v1.7.0*
