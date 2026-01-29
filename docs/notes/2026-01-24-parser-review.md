# Parser and resolver review notes (2026-01-24)

Consolidated from:
- RESOLVER_GAPS.md
- REVIEW_ISSUES.md

Note: References to the original filenames are historical; the originals were merged into this document.

---

## RESOLVER_GAPS.md

# E2E OIL Resolver Gaps

## Findings
- HIGH: Action commands with CSS selector targets can't resolve because selectors are passed through and `extract_id` rejects non-ID targets, so these script lines fail before any scanner request: `test-harness/scripts/06_edge_cases.oil:14`, `test-harness/scripts/07_intents_builtin.oil:23`, `test-harness/scripts/07_intents_builtin.oil:79`, `test-harness/scripts/07_intents_builtin.oil:139`, `test-harness/scripts/07_intents_builtin.oil:140`, `test-harness/scripts/09_target_resolution.oil:58`. Root cause in `crates/oryn-parser/src/resolver.rs:110` and `crates/oryn-parser/src/translator.rs:33`.
- HIGH: `wait visible` with text targets gets resolved to numeric IDs, but `WaitRequest` only carries `target`/`text` and the scanner treats `target` as a CSS selector, so waits like these will time out: `test-harness/scripts/03_ecommerce.oil:7`, `test-harness/scripts/03_ecommerce.oil:12`, `test-harness/scripts/05_dynamic.oil:10`, `test-harness/scripts/05_dynamic.oil:13`, `test-harness/scripts/09_target_resolution.oil:79`. Root cause in `crates/oryn-parser/src/lib.rs:72`, `crates/oryn-parser/src/translator.rs:171`, `crates/oryn-common/src/protocol.rs:169`, `crates/oryn-scanner/src/scanner.js:1451`.
- HIGH: `dismiss` is stub-parsed and never translated, so the commands in these scripts will always error: `test-harness/scripts/02_forms.oil:8`, `test-harness/scripts/07_intents_builtin.oil:43`. Root cause in `crates/oryn-parser/src/parser.rs:759` and `crates/oryn-parser/src/translator.rs:287`.
- HIGH: `submit` without a target is translated as `id=0`, which the scanner will reject; this breaks `test-harness/scripts/07_intents_builtin.oil:58` at `crates/oryn-parser/src/translator.rs:144`.
- MEDIUM: Text resolution doesn't consider `id` or `aria-label`, so `type "coupon-code"` won't match the coupon input (label text is "Have a coupon code?", placeholder "Enter code"), causing a NoMatch in the multipage flow: `test-harness/scripts/08_multipage_flows.oil:20`, `crates/oryn-parser/src/resolver.rs:223`, `test-harness/scenarios/intent-tests/flow-cart.html:110`, `test-harness/scenarios/intent-tests/flow-cart.html:112`.

## Open Questions
- Should selector targets be supported for `click`/`type`/`select` by resolving selectors to IDs in the resolver, or by adding selector fields to the scanner actions?
- For `wait visible/hidden`, do you want to skip resolution for text targets or extend `WaitRequest` + scanner to accept `id`?
- Should `submit` without a target infer a submit control or return a user-visible error?
- Is matching on `id` and `aria-label` expected for text targets (e.g., "coupon-code")?

## Notes
- Review only; no code changes.
- Tests not run.

---

## REVIEW_ISSUES.md

# Code Review Issues - Oryn Parser Refactor

## File: crates/oryn-parser/src/translator.rs
### L148: [HIGH] Regression in `submit` command inference.
The translation for `Command::Submit` defaults to ID `0` when no target is provided, marking it with a `TODO`. The legacy implementation used inference logic (finding the first submit button/form) which is now lost. This breaks `submit` (without arguments) behavior.

**Suggested change:**
Implement the inference logic in the resolver or translator, or explicitly error if target is missing until inference is restored.
```rust
        Command::Submit(cmd) => {
            let id = if let Some(t) = &cmd.target {
                extract_id(t, "Submit")?
            } else {
-               0 // TODO: Global submit? Or translate error?
+               // Restore inference or return error
+               return Err(TranslationError::Unsupported("Submit inference not yet implemented".into()));
            };
            Ok(Action::Scanner(ScannerAction::Submit(SubmitRequest { id })))
        },
```

## File: crates/oryn-parser/src/translator.rs
### L187: [HIGH] Potential protocol mismatch for `Wait` command.
The translator maps `WaitCondition::Until` to a "until" condition with the script as `target`.
```rust
WaitCondition::Until(s) => ("until", Some(s.clone()), None),
```
However, the `WaitRequest` struct in `crates/oryn-common/src/protocol.rs` (which `ScannerAction::Wait` wraps) does not appear to have been updated to support `expression` or handle `target` as a script for the "until" condition. If the scanner expects an `expression` field (as per the updated docs), this data will be lost or potentially mis-serialized.

## File: crates/oryn-common/src/protocol.rs
### L1: [MEDIUM] Missing `WaitRequest` update.
The documentation (`docs/SPEC-SCANNER-PROTOCOL.md`) was updated to include an `expression` parameter for `wait_for`, but the `WaitRequest` struct definition in `protocol.rs` does not show this field in the diff. This will cause `wait until` commands to fail or behave unpredictably.

**Suggested change:**
Update `WaitRequest` to include the `expression` field.
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitRequest {
    pub condition: String,
    pub target: Option<String>,
    pub text: Option<String>,
+   pub expression: Option<String>,
    pub timeout_ms: Option<u64>,
}
```

## File: crates/oryn-parser/src/resolver.rs
### L140: [LOW] Unnecessary string allocation/cloning in resolver error.
In `resolve_relation`, the error string creation involves an allocation that occurs even if not strictly needed in the match arm (though here it's an error path).
```rust
_ => return Err(ResolverError::RelationalError("Anchor could not be resolved to ID".into())),
```
Consider more specific error variants for `ResolverError` to avoid loose string matching if this becomes a hot path, or keep as is for now.

## File: crates/oryn-parser/src/builder/primitives.rs
### L7: [MEDIUM] Fallible unwrap in `build_string_value`.
```rust
let inner = pair.into_inner().next().unwrap();
```
While the grammar likely enforces `string_value` structure, relying on `unwrap()` in the builder makes the parser potentially panic-prone if the grammar changes slightly (e.g. if `string_inner` became optional). It is safer to use `next().ok_or(ParseError::...)`.

## File: crates/oryn-parser/src/normalizer.rs
### L136: [MEDIUM] Incomplete JSON balancing logic.
The `count_balance` function counts `{` and `}` to detect JSON boundaries. This naive counting fails if braces appear inside strings within the JSON (e.g., `{"key": "value with } brace"}`).
```rust
fn count_balance(s: &str) -> i32 {
    let mut b = 0;
    for c in s.chars() {
        if c == '{' { b += 1; }
        else if c == '}' { b -= 1; }
    }
    b
}
```
This should iterate characters while tracking string state (in/out of quotes) to correctly balance braces.

## File: crates/oryn-r/src/backend.rs
### L90: [LOW] Inefficient serialization for action extraction.
The backend serializes the entire `command` to `serde_json::Value` just to extract the "action" string tag, then re-uses the value.
```rust
let value = serde_json::to_value(&command)?;
let action = value.get("action")...
```
Since `command` is known to be `ScannerAction`, you could potentially match on the enum variant to get the action string directly, avoiding the allocation of the intermediate Value map, though this might require a helper method on `ScannerAction`.

---

