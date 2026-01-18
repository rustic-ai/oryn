# Oryn Parser Compliance Kit Specification

## Version 1.0

---

## 1. Overview

The Parser Compliance Kit (PCK) defines the authoritative test suite and verification procedures for validating that a parser implementation correctly interprets the Oryn Intent Language as specified in SPEC-INTENT-LANGUAGE.md.

### 1.1 Purpose

Parser implementations may vary in their internal architecture, programming language, and optimization strategies. The PCK ensures that despite these differences, all compliant parsers produce semantically equivalent results for the same input. This guarantees that agents can rely on consistent behavior regardless of which parser implementation they interact with.

### 1.2 Scope

The PCK covers:

- Lexical analysis (tokenization)
- Syntactic parsing (command structure)
- Target resolution (ID, text, role, selector)
- Option handling (flags and values)
- Error detection and reporting
- Forgiveness behaviors (case, aliases, variations)

The PCK does not cover:

- Command execution (handled by backends)
- Scanner protocol compliance (separate specification)
- Performance characteristics (implementation-specific)
- Memory usage or resource consumption

### 1.3 Conformance Language

This specification uses the following terms:

| Term | Meaning |
|------|---------|
| **MUST** | Absolute requirement for compliance |
| **MUST NOT** | Absolute prohibition for compliance |
| **SHOULD** | Recommended but not required |
| **SHOULD NOT** | Discouraged but not prohibited |
| **MAY** | Optional behavior |

---

## 2. Compliance Levels

The PCK defines three compliance levels. Each level builds upon the previous, adding additional requirements.

### 2.1 Level 1: Core

Core compliance represents the minimum viable parser. A Level 1 compliant parser can handle basic agent interactions but lacks advanced features.

**Requirements:**

- MUST parse all navigation commands (goto, back, forward, refresh, url)
- MUST parse observation commands (observe, html, text, title, screenshot)
- MUST parse primary action commands (click, type, clear, press, select, check, uncheck)
- MUST support ID targeting (numeric element references)
- MUST support quoted string arguments (double quotes)
- MUST return structured errors for unparseable input
- MUST be case-insensitive for command names

**Excluded from Level 1:**

- Text and role targeting
- Selector targeting
- Command aliases
- Single-quote strings
- Option parsing
- Intent commands
- Wait commands

### 2.2 Level 2: Standard

Standard compliance represents a production-ready parser suitable for most agent deployments.

**Requirements (in addition to Level 1):**

- MUST support text targeting (string matches to element text)
- MUST support role targeting (email, password, search, submit, etc.)
- MUST support selector targeting (css(), xpath())
- MUST parse all wait commands with all condition types
- MUST parse options in --flag and --key=value formats
- MUST accept single-quote strings interchangeably with double quotes
- MUST support standard escape sequences in strings (\", \', \\, \n, \t)
- MUST recognize command aliases (navigate→goto, go to→goto)
- MUST provide suggestions in error responses for unknown commands
- MUST handle extraction commands (extract, cookies, storage)
- MUST handle tab commands (tabs, tab)

### 2.3 Level 3: Full

Full compliance represents a parser that implements every feature of the Intent Language specification, including advanced forgiveness behaviors.

**Requirements (in addition to Level 2):**

- MUST parse intent commands (login, search, dismiss, accept)
- MUST accept options in all variant formats (--opt, -opt, opt after command)
- MUST handle unquoted single-word arguments where unambiguous
- MUST normalize whitespace (multiple spaces, leading/trailing)
- MUST support Unicode in all string contexts
- MUST provide position information in error responses
- MUST detect and report ambiguous inputs with resolution hints
- SHOULD support future goal commands (extensibility)

---

## 3. Test Categories

The PCK organizes tests into categories that map to parser responsibilities.

### 3.1 Lexical Tests

Lexical tests verify correct tokenization of input strings.

**3.1.1 Token Recognition**

Tests that the parser correctly identifies:

- Command keywords
- Numeric literals (element IDs, amounts)
- String literals (quoted content)
- Option flags (--prefixed tokens)
- Special tokens (reserved words, modifiers)

**3.1.2 String Handling**

Tests for string literal processing:

- Double-quoted strings
- Single-quoted strings
- Empty strings
- Strings containing spaces
- Strings containing quotes (escaped)
- Strings containing escape sequences
- Strings containing Unicode characters
- Strings containing newlines (escaped)
- Unterminated strings (error case)

**3.1.3 Whitespace Handling**

Tests for whitespace normalization:

- Single spaces between tokens
- Multiple spaces between tokens
- Leading whitespace
- Trailing whitespace
- Tab characters
- Mixed whitespace

### 3.2 Syntactic Tests

Syntactic tests verify correct parsing of command structure.

**3.2.1 Command Structure**

Tests that commands are parsed into correct components:

- Command name extraction
- Target identification
- Argument association
- Option attachment
- Argument ordering

**3.2.2 Command-Specific Syntax**

Each command category has specific syntactic requirements:

| Category | Syntax Pattern |
|----------|----------------|
| Navigation | `command [url]` |
| Observation | `command [--options]` |
| Action | `command target [arguments] [--options]` |
| Wait | `command condition [target] [--options]` |
| Extraction | `command subcommand [arguments]` |
| Intent | `command [arguments]` |

**3.2.3 Option Parsing**

Tests for option handling:

- Boolean flags (--force)
- Value options (--timeout 10s)
- Assignment syntax (--timeout=10s)
- Multiple options
- Option ordering (before/after arguments)
- Unknown options (error or ignore based on strictness)

### 3.3 Target Resolution Tests

Target resolution tests verify that element references are correctly classified.

**3.3.1 ID Targeting**

- Positive integers (click 5)
- Zero (click 0) — implementation-defined validity
- Large numbers (click 999)
- Negative numbers (error case)
- Non-integers (error case)

**3.3.2 Text Targeting**

- Exact quoted strings (click "Sign in")
- Strings with spaces (click "Log out now")
- Empty strings (implementation-defined)
- Special characters in text

**3.3.3 Role Targeting**

- Reserved role words (email, password, search, submit, username, phone, url)
- Role words are unquoted
- Disambiguation from text targeting

**3.3.4 Selector Targeting**

- CSS selectors: css(".class"), css("#id"), css("[attr=value]")
- XPath selectors: xpath("//div"), xpath("//button[@type='submit']")
- Malformed selectors (parser accepts syntax; validation is backend responsibility)

**3.3.5 Ambiguity Resolution**

When input could match multiple target types:

- Numeric tokens → ID targeting
- Quoted strings → Text targeting
- Reserved words (unquoted) → Role targeting
- css(...) or xpath(...) → Selector targeting
- Unquoted non-reserved words → Implementation-defined (Level 3 requires text inference)

### 3.4 Forgiveness Tests

Forgiveness tests verify that the parser accepts reasonable variations.

**3.4.1 Case Insensitivity**

- Lowercase commands (click)
- Uppercase commands (CLICK)
- Mixed case commands (CliCk)
- Case preservation in arguments (type 1 "Hello" keeps "Hello" as-is)

**3.4.2 Command Aliases**

| Canonical | Accepted Aliases |
|-----------|------------------|
| goto | navigate, go to, nav, open |
| click | tap, press (when unambiguous) |
| type | enter, input |
| observe | scan, look, see |
| screenshot | capture, snap |

**3.4.3 Quote Flexibility**

- Double quotes: "hello"
- Single quotes: 'hello'
- Mixed (error): "hello' or 'hello"
- Nested quotes: "say 'hi'" or 'say "hi"'

**3.4.4 Option Flexibility**

- Double dash: --force
- Single dash: -force
- No dash (after command context): force
- Assignment: --timeout=10s
- Space-separated: --timeout 10s

### 3.5 Error Handling Tests

Error handling tests verify that the parser produces useful error information.

**3.5.1 Error Detection**

The parser MUST detect:

- Unknown commands
- Malformed syntax
- Missing required arguments
- Invalid argument types
- Unterminated strings
- Unbalanced parentheses (for selectors)

**3.5.2 Error Response Structure**

Error responses MUST include:

- Error category/code
- Human-readable message
- Input that caused the error

Error responses SHOULD include:

- Position in input (character offset or line:column)
- Suggestions for correction
- Expected syntax pattern

**3.5.3 Recovery Behavior**

After an error, the parser:

- MUST NOT produce partial results
- MUST NOT crash or hang
- SHOULD be ready to parse the next input

---

## 4. Test Case Specification

### 4.1 Test Case Format

Each test case is defined using a structured text format:

```
@test "<unique-test-id>"
@category "<category-name>"
@level <1|2|3>
@description "<human-readable description>"

INPUT:
<raw command string, may span multiple lines if escaped>

EXPECT:
<expected parse result in canonical JSON>

---
```

For error cases:

```
@test "<unique-test-id>"
@category "<category-name>"
@level <1|2|3>
@description "<human-readable description>"

INPUT:
<raw command string>

EXPECT_ERROR:
@code "<error-code>"
@message_contains "<substring that must appear>"
@suggests ["<suggestion1>", "<suggestion2>"]  # optional

---
```

### 4.2 Canonical Output Format

Parse results are expressed in a canonical JSON format for comparison:

```json
{
  "command": "<canonical-command-name>",
  "target": {
    "type": "<id|text|role|css|xpath>",
    "value": "<target-value>"
  },
  "arguments": {
    "<arg-name>": "<arg-value>"
  },
  "options": {
    "<option-name>": "<option-value|true>"
  }
}
```

**Canonicalization Rules:**

- Command names are lowercase
- Target types use canonical names (id, text, role, css, xpath)
- Numeric values are JSON numbers, not strings
- Boolean options are JSON booleans
- Absent optional fields are omitted (not null)
- Field ordering is alphabetical for deterministic comparison

### 4.3 Test Metadata

**Test ID Format:** `<category>-<subcategory>-<sequence>`

Examples:
- `lexical-string-001`
- `syntax-click-015`
- `target-role-003`
- `forgive-case-007`
- `error-unknown-002`

**Category Names:**

| Category | Description |
|----------|-------------|
| lexical | Tokenization tests |
| syntax | Command structure tests |
| target | Target resolution tests |
| forgive | Forgiveness behavior tests |
| error | Error handling tests |
| command | Command-specific tests |
| intent | Intent command tests |

---

## 5. Test Suite Organization

### 5.1 Directory Structure

```
compliance-kit/
├── spec/
│   └── COMPLIANCE-KIT.md          # This document
├── tests/
│   ├── level-1/
│   │   ├── lexical/
│   │   │   ├── tokens.cases
│   │   │   └── strings.cases
│   │   ├── syntax/
│   │   │   ├── navigation.cases
│   │   │   ├── observation.cases
│   │   │   └── actions.cases
│   │   ├── target/
│   │   │   └── id-targeting.cases
│   │   └── error/
│   │       └── basic-errors.cases
│   ├── level-2/
│   │   ├── target/
│   │   │   ├── text-targeting.cases
│   │   │   ├── role-targeting.cases
│   │   │   └── selector-targeting.cases
│   │   ├── syntax/
│   │   │   ├── wait-commands.cases
│   │   │   ├── options.cases
│   │   │   └── extraction.cases
│   │   ├── forgive/
│   │   │   ├── aliases.cases
│   │   │   └── quotes.cases
│   │   └── error/
│   │       └── suggestions.cases
│   └── level-3/
│       ├── intent/
│       │   └── intent-commands.cases
│       ├── forgive/
│       │   ├── option-variants.cases
│       │   ├── whitespace.cases
│       │   └── unquoted-args.cases
│       └── edge/
│           ├── unicode.cases
│           └── ambiguity.cases
├── schemas/
│   ├── test-case.schema.json      # JSON Schema for test case format
│   └── parse-result.schema.json   # JSON Schema for canonical output
└── tools/
    └── validator/                  # Reference validator specification
```

### 5.2 Test Counts by Level

| Level | Minimum Test Count | Coverage Requirement |
|-------|-------------------|----------------------|
| 1 | 150 tests | All Level 1 commands and basic targeting |
| 2 | 300 tests | All Level 2 features, aliases, error suggestions |
| 3 | 200 tests | Intent commands, edge cases, full forgiveness |
| **Total** | **650+ tests** | Complete specification coverage |

### 5.3 Test Distribution

Approximate distribution across categories:

| Category | Percentage | Focus |
|----------|------------|-------|
| Command-specific | 40% | Each command's syntax variations |
| Target resolution | 20% | All targeting strategies |
| Forgiveness | 15% | Case, aliases, quotes, options |
| Error handling | 15% | Detection and reporting |
| Edge cases | 10% | Unicode, ambiguity, limits |

---

## 6. Verification Procedures

### 6.1 Test Execution

**6.1.1 Input Delivery**

The test harness MUST:

- Deliver input strings exactly as specified (byte-for-byte)
- Not normalize or preprocess input before passing to parser
- Support multi-byte UTF-8 input
- Handle empty input as a valid test case

**6.1.2 Output Capture**

The test harness MUST:

- Capture the complete parse result or error
- Not modify the parser's output
- Handle both success and error responses
- Enforce reasonable timeout (suggested: 100ms per test)

**6.1.3 Comparison**

Results are compared using semantic equivalence:

- JSON structural equality (not string equality)
- Numeric values compared as numbers
- String values compared as Unicode sequences
- Field ordering is ignored
- Extra fields in parser output are ignored (forward compatibility)
- Missing required fields cause test failure

### 6.2 Pass/Fail Criteria

**6.2.1 Individual Test**

A test passes if:

- For success cases: Parser output matches expected output (semantic equivalence)
- For error cases: Parser returns error with correct code and required content

A test fails if:

- Parser produces different output than expected
- Parser produces success when error expected (or vice versa)
- Parser crashes, hangs, or times out
- Parser produces malformed output

**6.2.2 Level Certification**

A parser achieves Level N certification if:

- 100% of Level N tests pass
- 100% of all lower level tests pass
- No crashes or hangs on any test (including higher levels)

**6.2.3 Partial Compliance**

If a parser passes some but not all tests at a level:

- Report percentage passed per category
- Identify specific failing tests
- Do not certify at that level

### 6.3 Regression Testing

When the specification or test suite is updated:

- Previously certified parsers SHOULD be re-tested
- New tests SHOULD be clearly marked with version introduced
- Removed tests SHOULD be archived with removal rationale
- Changed tests SHOULD maintain the same test ID with version annotation

---

## 7. Reporting Requirements

### 7.1 Compliance Report Structure

A compliance report MUST include:

**7.1.1 Summary Section**

- Parser identification (name, version)
- Test suite version
- Execution timestamp
- Overall compliance level achieved
- Pass/fail counts by level

**7.1.2 Detailed Results**

For each test:

- Test ID
- Pass/Fail status
- For failures: expected vs. actual output

**7.1.3 Category Breakdown**

- Pass rate per category
- List of failing tests per category

### 7.2 Report Format

Reports SHOULD be available in:

- Human-readable format (Markdown or HTML)
- Machine-readable format (JSON)

**JSON Report Schema:**

```json
{
  "parser": {
    "name": "<parser-name>",
    "version": "<parser-version>"
  },
  "suite": {
    "version": "<test-suite-version>",
    "executed_at": "<ISO-8601-timestamp>"
  },
  "summary": {
    "compliance_level": <0|1|2|3>,
    "total_tests": <number>,
    "passed": <number>,
    "failed": <number>,
    "skipped": <number>
  },
  "levels": {
    "1": { "total": <n>, "passed": <n>, "failed": <n> },
    "2": { "total": <n>, "passed": <n>, "failed": <n> },
    "3": { "total": <n>, "passed": <n>, "failed": <n> }
  },
  "categories": {
    "<category>": { "total": <n>, "passed": <n>, "failed": <n> }
  },
  "failures": [
    {
      "test_id": "<test-id>",
      "category": "<category>",
      "level": <level>,
      "input": "<input-string>",
      "expected": <expected-output>,
      "actual": <actual-output>,
      "error": "<error-message-if-applicable>"
    }
  ]
}
```

### 7.3 Certification Badge

Parsers achieving compliance MAY display a badge indicating:

- Compliance level (1, 2, or 3)
- Test suite version
- Certification date

Badge text format: `Oryn Parser Compliant: Level N (vX.Y)`

---

## 8. Reference Test Cases

This section provides representative test cases for each category. The complete test suite contains hundreds of cases; these examples illustrate the format and coverage expectations.

### 8.1 Lexical Tests

```
@test "lexical-string-001"
@category "lexical"
@level 1
@description "Double-quoted string with spaces"

INPUT:
type 1 "hello world"

EXPECT:
{
  "command": "type",
  "target": { "type": "id", "value": 1 },
  "arguments": { "text": "hello world" }
}

---

@test "lexical-string-002"
@category "lexical"
@level 2
@description "Single-quoted string"

INPUT:
type 1 'hello world'

EXPECT:
{
  "command": "type",
  "target": { "type": "id", "value": 1 },
  "arguments": { "text": "hello world" }
}

---

@test "lexical-string-003"
@category "lexical"
@level 2
@description "Escaped quote in string"

INPUT:
type 1 "say \"hello\""

EXPECT:
{
  "command": "type",
  "target": { "type": "id", "value": 1 },
  "arguments": { "text": "say \"hello\"" }
}

---

@test "lexical-string-004"
@category "lexical"
@level 3
@description "Unicode string content"

INPUT:
type 1 "こんにちは"

EXPECT:
{
  "command": "type",
  "target": { "type": "id", "value": 1 },
  "arguments": { "text": "こんにちは" }
}

---

@test "lexical-whitespace-001"
@category "lexical"
@level 3
@description "Multiple spaces between tokens"

INPUT:
click    5

EXPECT:
{
  "command": "click",
  "target": { "type": "id", "value": 5 }
}

---
```

### 8.2 Target Resolution Tests

```
@test "target-id-001"
@category "target"
@level 1
@description "Numeric ID targeting"

INPUT:
click 42

EXPECT:
{
  "command": "click",
  "target": { "type": "id", "value": 42 }
}

---

@test "target-text-001"
@category "target"
@level 2
@description "Text targeting with quoted string"

INPUT:
click "Sign in"

EXPECT:
{
  "command": "click",
  "target": { "type": "text", "value": "Sign in" }
}

---

@test "target-role-001"
@category "target"
@level 2
@description "Role targeting - email"

INPUT:
type email "user@example.com"

EXPECT:
{
  "command": "type",
  "target": { "type": "role", "value": "email" },
  "arguments": { "text": "user@example.com" }
}

---

@test "target-role-002"
@category "target"
@level 2
@description "Role targeting - password"

INPUT:
type password "secret123"

EXPECT:
{
  "command": "type",
  "target": { "type": "role", "value": "password" },
  "arguments": { "text": "secret123" }
}

---

@test "target-css-001"
@category "target"
@level 2
@description "CSS selector targeting"

INPUT:
click css(".btn-primary")

EXPECT:
{
  "command": "click",
  "target": { "type": "css", "value": ".btn-primary" }
}

---

@test "target-xpath-001"
@category "target"
@level 2
@description "XPath selector targeting"

INPUT:
click xpath("//button[@type='submit']")

EXPECT:
{
  "command": "click",
  "target": { "type": "xpath", "value": "//button[@type='submit']" }
}

---
```

### 8.3 Command Syntax Tests

```
@test "command-goto-001"
@category "command"
@level 1
@description "Basic goto with URL"

INPUT:
goto https://example.com

EXPECT:
{
  "command": "goto",
  "arguments": { "url": "https://example.com" }
}

---

@test "command-goto-002"
@category "command"
@level 1
@description "Goto with domain only"

INPUT:
goto example.com

EXPECT:
{
  "command": "goto",
  "arguments": { "url": "example.com" }
}

---

@test "command-type-001"
@category "command"
@level 1
@description "Type with ID target"

INPUT:
type 3 "hello"

EXPECT:
{
  "command": "type",
  "target": { "type": "id", "value": 3 },
  "arguments": { "text": "hello" }
}

---

@test "command-type-002"
@category "command"
@level 2
@description "Type with append option"

INPUT:
type 3 "more" --append

EXPECT:
{
  "command": "type",
  "target": { "type": "id", "value": 3 },
  "arguments": { "text": "more" },
  "options": { "append": true }
}

---

@test "command-wait-001"
@category "command"
@level 2
@description "Wait for element visible"

INPUT:
wait visible 5

EXPECT:
{
  "command": "wait",
  "arguments": { "condition": "visible" },
  "target": { "type": "id", "value": 5 }
}

---

@test "command-wait-002"
@category "command"
@level 2
@description "Wait with timeout option"

INPUT:
wait visible 5 --timeout 10s

EXPECT:
{
  "command": "wait",
  "arguments": { "condition": "visible" },
  "target": { "type": "id", "value": 5 },
  "options": { "timeout": "10s" }
}

---

@test "command-select-001"
@category "command"
@level 1
@description "Select by value"

INPUT:
select 5 "option1"

EXPECT:
{
  "command": "select",
  "target": { "type": "id", "value": 5 },
  "arguments": { "value": "option1" }
}

---

@test "command-press-001"
@category "command"
@level 1
@description "Press single key"

INPUT:
press Enter

EXPECT:
{
  "command": "press",
  "arguments": { "key": "Enter" }
}

---

@test "command-press-002"
@category "command"
@level 1
@description "Press key combination"

INPUT:
press Control+A

EXPECT:
{
  "command": "press",
  "arguments": { "key": "Control+A" }
}

---
```

### 8.4 Forgiveness Tests

```
@test "forgive-case-001"
@category "forgive"
@level 1
@description "Uppercase command"

INPUT:
CLICK 5

EXPECT:
{
  "command": "click",
  "target": { "type": "id", "value": 5 }
}

---

@test "forgive-case-002"
@category "forgive"
@level 1
@description "Mixed case command"

INPUT:
ObSeRvE

EXPECT:
{
  "command": "observe"
}

---

@test "forgive-alias-001"
@category "forgive"
@level 2
@description "Navigate alias for goto"

INPUT:
navigate example.com

EXPECT:
{
  "command": "goto",
  "arguments": { "url": "example.com" }
}

---

@test "forgive-alias-002"
@category "forgive"
@level 2
@description "Two-word alias 'go to'"

INPUT:
go to example.com

EXPECT:
{
  "command": "goto",
  "arguments": { "url": "example.com" }
}

---

@test "forgive-option-001"
@category "forgive"
@level 3
@description "Single-dash option"

INPUT:
click 5 -force

EXPECT:
{
  "command": "click",
  "target": { "type": "id", "value": 5 },
  "options": { "force": true }
}

---
```

### 8.5 Intent Command Tests

```
@test "intent-login-001"
@category "intent"
@level 3
@description "Login intent command"

INPUT:
login "user@example.com" "password123"

EXPECT:
{
  "command": "login",
  "arguments": {
    "username": "user@example.com",
    "password": "password123"
  }
}

---

@test "intent-search-001"
@category "intent"
@level 3
@description "Search intent command"

INPUT:
search "rust programming"

EXPECT:
{
  "command": "search",
  "arguments": { "query": "rust programming" }
}

---

@test "intent-dismiss-001"
@category "intent"
@level 3
@description "Dismiss popups intent"

INPUT:
dismiss popups

EXPECT:
{
  "command": "dismiss",
  "arguments": { "target": "popups" }
}

---

@test "intent-accept-001"
@category "intent"
@level 3
@description "Accept cookies intent"

INPUT:
accept cookies

EXPECT:
{
  "command": "accept",
  "arguments": { "target": "cookies" }
}

---
```

### 8.6 Error Handling Tests

```
@test "error-unknown-001"
@category "error"
@level 1
@description "Unknown command"

INPUT:
clik 5

EXPECT_ERROR:
@code "UNKNOWN_COMMAND"
@message_contains "clik"
@suggests ["click"]

---

@test "error-string-001"
@category "error"
@level 1
@description "Unterminated string"

INPUT:
type 1 "hello

EXPECT_ERROR:
@code "UNTERMINATED_STRING"
@message_contains "unterminated"

---

@test "error-missing-001"
@category "error"
@level 1
@description "Missing required argument"

INPUT:
type 1

EXPECT_ERROR:
@code "MISSING_ARGUMENT"
@message_contains "text"

---

@test "error-target-001"
@category "error"
@level 1
@description "Invalid target (negative ID)"

INPUT:
click -5

EXPECT_ERROR:
@code "INVALID_TARGET"
@message_contains "target"

---
```

---

## 9. Extensibility

### 9.1 Adding New Commands

When the Intent Language specification adds new commands:

1. Define test cases at the appropriate level
2. Assign new test IDs following the naming convention
3. Document the suite version where tests were added
4. Existing certified parsers retain certification until re-tested

### 9.2 Deprecating Commands

When commands are deprecated:

1. Tests remain in the suite but are marked deprecated
2. Parsers SHOULD still pass deprecated tests
3. Deprecation warnings in parser output are acceptable
4. Tests may be removed after two major versions

### 9.3 Custom Extensions

Parser implementations MAY support custom extensions:

- Extension commands MUST NOT conflict with standard commands
- Extension tests are not part of the official compliance kit
- Extension support does not affect compliance certification
- Extensions SHOULD be documented separately

---

## 10. Versioning

### 10.1 Suite Versioning

The compliance kit uses semantic versioning: `MAJOR.MINOR.PATCH`

- **MAJOR**: Breaking changes to test format or compliance criteria
- **MINOR**: New tests added, categories expanded
- **PATCH**: Test corrections, clarifications, typo fixes

### 10.2 Compatibility

- Parsers certified against version X.Y are presumed compatible with X.Z where Z > Y
- Major version changes require re-certification
- Test suite maintains backwards compatibility within major versions

### 10.3 Specification Alignment

The compliance kit version SHOULD align with the Intent Language specification version:

| Intent Language Spec | Compliance Kit |
|---------------------|----------------|
| 1.0 | 1.x |
| 2.0 | 2.x |

---

## Appendix A: Error Codes

| Code | Description | Level |
|------|-------------|-------|
| `UNKNOWN_COMMAND` | Command name not recognized | 1 |
| `INVALID_SYNTAX` | General syntax error | 1 |
| `UNTERMINATED_STRING` | String literal not closed | 1 |
| `MISSING_ARGUMENT` | Required argument not provided | 1 |
| `INVALID_TARGET` | Target cannot be resolved | 1 |
| `INVALID_OPTION` | Option name or value invalid | 2 |
| `AMBIGUOUS_TARGET` | Multiple resolution strategies match | 3 |
| `INVALID_SELECTOR` | CSS/XPath syntax error | 2 |
| `UNEXPECTED_TOKEN` | Token appears in wrong position | 1 |

---

## Appendix B: Reserved Words

These words have special meaning and affect parsing behavior:

**Role Words** (Level 2+):
email, password, search, submit, username, phone, url

**Condition Words** (Level 2+):
visible, hidden, exists, gone, idle, load

**Direction Words**:
up, down, left, right, top, bottom

**Modifier Words**:
near, after, before, inside, contains

**Key Names**:
Enter, Tab, Escape, Space, Backspace, Delete, ArrowUp, ArrowDown, ArrowLeft, ArrowRight, Home, End, PageUp, PageDown, Control, Shift, Alt, Meta, F1-F12

---

## Appendix C: Glossary

| Term | Definition |
|------|------------|
| **Canonical** | The standard, normalized form of a parse result |
| **Compliance Level** | Tier of specification conformance (1, 2, or 3) |
| **Forgiveness** | Parser acceptance of input variations |
| **Intent Command** | High-level command that encapsulates multiple actions |
| **Parse Result** | Structured output from successful parsing |
| **Semantic Equivalence** | Logical equality ignoring superficial differences |
| **Target** | Element reference in a command (ID, text, role, selector) |
| **Test Case** | Single input/expected-output pair |
| **Test Suite** | Complete collection of test cases |

---

*Document Version: 1.0*
*Last Updated: January 2025*