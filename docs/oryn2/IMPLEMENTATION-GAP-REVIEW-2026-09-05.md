# Implementation gap review — 2026-09-05

The proposal-v1 architecture is substantially implemented, but the current runtime
is not release-ready. The remaining work combines browser behavior fixes,
production-path validation, platform artifacts and release qualification.
The actionable backlog is in [the completion plan](RUNTIME-COMPLETION-ACTION-PLAN.md#4-prioritized-execution-backlog--reviewed-2026-09-05).

## Review basis and limits

Reviewed clean HEAD `b21c171`, repository guidance, current status and proposal
review, runtime source landmarks, WPT runner, result validator and frozen manifests.
HEAD differs from diagnostic source `e1b97d7` only in three documentation files.
The 2,267/4,022 result below is the documented diagnostic baseline, not a suite
rerun performed for this review. No current exhaustive raw aggregate was found
among the tracked benchmark evidence inspected. No new Rust build, full browser
suite, platform build, signing or paid model evaluation was performed.

## Status and gaps

| Area | Implementation status | Remaining gap |
|---|---|---|
| Runtime architecture | Shared Rust DOM, generated V8 interfaces, asynchronous broker/streaming navigation, persistent semantic graph and native Stylo are documented as implemented; source landmarks corroborate these paths. | Exhaustive behavioral correctness and exact packaged-worker evidence remain open. Architecture presence alone does not establish conformance. |
| Browser conformance | Documented in-process baseline passes 2,267/4,022 variants (56.4%). | 1,755 non-passes; no passing complete production-worker release aggregate. |
| Semantics/accessibility | Persistent graph exists; the full projector is gated behind tests or `semantic-oracle`. | 41/51 mapped cases pass; broaden mutation/churn/oracle and style-dependent comparisons. |
| Style/layout | Native computed values feed layout and semantic visibility. | CSS passes 113/564; mapped CSSOM and Chromium differential closure remain. |
| Runtime profiles | macOS ARM64 full/lite builds and reduced ICU/snapshots are documented. | Linux x86_64 full/lite, refreshed full macOS manifest and exact four-profile qualification remain. |
| Product/security/replay/rendering | Historical scoped completion evidence exists. | Requalify changed runtime and exact candidate; old commit-bound evidence cannot certify current bytes. Broader GPU/SVG/print work remains separately scoped. |
| Performance/release | Prior signed runs and release tooling exist. | Historical startup exceeds the 250 ms target; no current shipped-worker concurrency/RSS attestation or complete runtime-completion result. |

### Diagnostic backlog

These are non-passing execution variants, not counts of independent bugs or a
measure of total product implementation percentage.

| Root | Passed / required | Non-passing |
|---|---:|---:|
| Editing | 118 / 704 | 586 |
| CSS | 113 / 564 | 451 |
| DOM | 269 / 662 | 393 |
| IndexedDB | 304 / 456 | 152 |
| HTML | 30 / 136 | 106 |
| WebIDL | 26 / 58 | 32 |
| Referrer policy | 1,366 / 1,391 | 25 |
| Accessible Name | 13 / 19 | 6 |
| WAI-ARIA | 28 / 32 | 4 |
| Total | 2,267 / 4,022 | 1,755 |

DOM, editing and CSS account for 1,430 non-passes, about 81.5% of the backlog.
Across all roots, the documented categories are 1,527 assertion failures,
146 harness failures, 54 timeouts and 28 unclassified outcomes. The latest run
gained 32 passing cases but regressed 12 referrer cases: report both sides of
that change, not just the net gain of 20.

## Review findings that change the plan

1. **Evidence tooling is the first dependency.**
   `scripts/run-upstream-wpt.py::inventoried_tests` expands HTML variants from
   source when a record contains only the empty variant. In contrast,
   `scripts/validate-runtime-api-style-semantics-result.py::expected_assertions`
   expands only the stored inventory. Both tools still default to v1 paths.
   The documented mismatch is consistent with this implementation difference;
   exact missing/extra IDs need a pinned-checkout comparison. A valid 4,022 count
   does not prove set equality. Share one canonical expansion and test identity.
2. **Exercise production early.** The validator correctly requires
   `sandboxed_worker`, a clean source and matching hardened runtime fingerprints.
   The reported aggregate uses `in_process_probe`, where compatibility behavior
   can differ. Establish a full production baseline before treating diagnostic
   failure counts as the authoritative implementation backlog.
3. **Fix shared causes and regressions before isolated cases.** Completion,
   worker globals, scheduler timing and resource-policy behavior may affect many
   cases. Classify unknowns and retain traces; do not assume every timeout is a
   harness bug or increase deadlines without evidence.
4. **Separate focused success from root completion.** IndexedDB 100/100 and
   corrected editing 40/40 are useful focused results, while their complete
   roots still have 152 and 586 non-passes respectively. Preserve raw outcomes
   and the pinned upstream-defect ledger.
5. **Refresh planning drift.** The previous immediate list used 45 timeouts and
   CSS 90/564 despite a newer status table. Its foundation-commit instruction
   also referred to an old uncommitted state. The updated backlog replaces those
   priorities and adds owners by role, dependencies and measurable exit gates.

## Checks performed

- `python3 scripts/validate-runtime-completion-manifest.py`: passed, nine required classifications.
- `python3 scripts/validate-runtime-conformance-inventory.py benchmarks/conformance/runtime-api-style-semantics-v2-inventory.json --manifest benchmarks/conformance/runtime-api-style-semantics-v2.json`: passed, 3,184 files and 4,022 executions (3,749 window, 265 dedicated worker, four shared worker, four service worker).
- `python3 -m unittest discover -s scripts/tests -p 'test_upstream_wpt_runner.py'`: all 22 tests passed.
- `python3 scripts/check-docs-truth.py`: passed. Its success does not detect the planning drift above or establish production conformance.

Recommended first implementation batch: unify v2 execution IDs and validator
coverage, reproduce the 12 referrer regressions, classify unknown outcomes and
establish a representative packaged-worker smoke ledger. Then run the full
production inventory and prioritize shared-cause fixes from that result.


## Follow-up implementation audit

The subsequent conformance work located the complete local diagnostic at
`artifacts/upstream-conformance/full-4022-e1b97d7.json`. Comparing the pinned v2
runner and validator against the cached upstream checkout established exact
set equality (4,022 IDs, no missing/extra IDs); the old mismatch warning was
consistent with v1 defaults, not an actual mismatch when explicitly using v2.
The tooling now defaults to v2 and locked execution no longer rediscovers IDs.

The ledger also exposed four historical passes recovered from partial DOM-probe
assertions without verified completion. The corrected runner preserves partial
assertions as incomplete failures, and the release validator rejects probe-only
passes. Open-ended editing partitions now use the instrumented total assertion
count, continue beyond 1,000 assertions when necessary and retain failed empty
partitions. These changes invalidate any inference that the historical pass
count alone proves completion; they do not rewrite the historical result.
