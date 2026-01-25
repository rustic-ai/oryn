# E2E OIL Resolver Gaps

## Findings
- HIGH: Action commands with CSS selector targets can’t resolve because selectors are passed through and `extract_id` rejects non-ID targets, so these script lines fail before any scanner request: `test-harness/scripts/06_edge_cases.oil:14`, `test-harness/scripts/07_intents_builtin.oil:23`, `test-harness/scripts/07_intents_builtin.oil:79`, `test-harness/scripts/07_intents_builtin.oil:139`, `test-harness/scripts/07_intents_builtin.oil:140`, `test-harness/scripts/09_target_resolution.oil:58`. Root cause in `crates/oryn-parser/src/resolver.rs:110` and `crates/oryn-parser/src/translator.rs:33`.
- HIGH: `wait visible` with text targets gets resolved to numeric IDs, but `WaitRequest` only carries `target`/`text` and the scanner treats `target` as a CSS selector, so waits like these will time out: `test-harness/scripts/03_ecommerce.oil:7`, `test-harness/scripts/03_ecommerce.oil:12`, `test-harness/scripts/05_dynamic.oil:10`, `test-harness/scripts/05_dynamic.oil:13`, `test-harness/scripts/09_target_resolution.oil:79`. Root cause in `crates/oryn-parser/src/lib.rs:72`, `crates/oryn-parser/src/translator.rs:171`, `crates/oryn-common/src/protocol.rs:169`, `crates/oryn-scanner/src/scanner.js:1451`.
- HIGH: `dismiss` is stub-parsed and never translated, so the commands in these scripts will always error: `test-harness/scripts/02_forms.oil:8`, `test-harness/scripts/07_intents_builtin.oil:43`. Root cause in `crates/oryn-parser/src/parser.rs:759` and `crates/oryn-parser/src/translator.rs:287`.
- HIGH: `submit` without a target is translated as `id=0`, which the scanner will reject; this breaks `test-harness/scripts/07_intents_builtin.oil:58` at `crates/oryn-parser/src/translator.rs:144`.
- MEDIUM: Text resolution doesn’t consider `id` or `aria-label`, so `type "coupon-code"` won’t match the coupon input (label text is “Have a coupon code?”, placeholder “Enter code”), causing a NoMatch in the multipage flow: `test-harness/scripts/08_multipage_flows.oil:20`, `crates/oryn-parser/src/resolver.rs:223`, `test-harness/scenarios/intent-tests/flow-cart.html:110`, `test-harness/scenarios/intent-tests/flow-cart.html:112`.

## Open Questions
- Should selector targets be supported for `click`/`type`/`select` by resolving selectors to IDs in the resolver, or by adding selector fields to the scanner actions?
- For `wait visible/hidden`, do you want to skip resolution for text targets or extend `WaitRequest` + scanner to accept `id`?
- Should `submit` without a target infer a submit control or return a user-visible error?
- Is matching on `id` and `aria-label` expected for text targets (e.g., "coupon-code")?

## Notes
- Review only; no code changes.
- Tests not run.
