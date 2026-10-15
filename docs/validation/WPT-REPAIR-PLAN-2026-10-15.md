# Oryn2 repair plan — 2026-10-15 evidence update

Owner/coordinator: /root. Integration checkout: oryn-roadmap-next on
codex/conformance-next. Analyzed base: b0b34294429b60391f41283f9493c0827eac8e7c.
State: investigated plan, not implementation or qualification. Recovery checkout
oryn stays untouched. This update supersedes historical failure priorities for
this selected inventory; broader delivery and platform gates remain binding.

## Current evidence

Signed core-full build git-b0b34294-core-full-01 passed build and assembly.
Manifest digest: 7923c0e77eca56f41dc59484c1b9d4bc378f29104ec43860ec39bf44aa2dd2cf.
Run: oryn-data/development/wpt-b0-4021-20261014-01, six upstream workers,
6,278.685 seconds. All 4,021 URLs started/ended uniquely; no duplicate/missing
URL events; owned cleanup completed. Report SHA-256:
df6cad455bcd854d6f3f71c5dc447a1a58667e4ce621705f47773298f97eb130.

Harness: 3,561 OK, one PASS, 295 CRASH, 146 TIMEOUT, 18 ERROR. Assertions:
84,925 PASS, 4,605 FAIL, 971 NOTRUN, 194 TIMEOUT, 14 PRECONDITION_FAILED.
Harness OK is not whole-execution pass. Strict observed whole-pass count is
3,086/4,021; 935 executions remain nonpassing or lack required assertion output.
This is developer execution against a signed runtime, not official qualification.
Finalization failed: fixture bootstrap interpreter differs. Existing receipt
must remain unsuccessful; a corrected verifier cannot relabel it as qualified.

Approved inventory interpretation retains all 4,022 IDs: 4,020 testharness,
one genuine crashtest, one parent-derived support obligation. The support child
is not a second independent upstream execution. Its parent currently crashed;
existing diagnostics do not yet prove child resource bytes/assertion custody.

## Exact comparison, not a bug count

Comparison key: unchanged URL/query + subtest name + occurrence ordinal.
Against old full 76d31ea3 run: 4,020 common URLs, 84 observed whole gains and
154 whole regressions; 322 common assertions recover to PASS, 214 prior PASS
become nonpassing, and 77,074 prior assertions disappear. 236 prior harness-OK
URLs now CRASH. Against ccb34108 affected run: 1,178 common URLs, 169 observed
whole gains/56 losses; 956 assertion recoveries/39 regressions, 65,353 missing
prior assertions. Do not subtract these populations to claim net fixed bugs.
Crashes erase observations; recover them before accepting semantic gains.

## Family backlog

| Family | Selected | Observed whole pass | Nonpassing | Crashes |
| --- | ---: | ---: | ---: | ---: |
| Editing | 704 | 366 | 338 | 170 |
| Referrer policy | 1,391 | 1,231 | 160 | 22 |
| DOM | 662 | 504 | 158 | 33 |
| CSS | 564 | 411 | 153 | 57 |
| HTML/lifecycle | 136 | 86 | 50 | 5 |
| IndexedDB | 455 | 413 | 42 | 4 |
| WebIDL | 58 | 36 | 22 | 2 |
| Accname | 19 | 13 | 6 | 1 |
| WAI-ARIA | 32 | 26 | 6 | 1 |

## Priority batches and gates

### P0 — trustworthy evidence and observation recovery

1. Repair verifier-context separation. fixture_tls.load_contract checks the
   executing interpreter; developer parent finalization invokes that child-side
   check under Homebrew Python, while actual child preflight used the selected
   direct Python. Validate child interpreter through admitted process/receipt
   identity; retain strict executing-interpreter checks in child bootstrap.
   Do not disable TLS checks. Negative tests: foreign child, substituted bytes,
   missing receipts, parent verifier using another Python. Run a short signed
   TLS smoke first; failed original remains preserved.
2. Diagnose retained-allocation scope authority before semantic repairs.
   231/295 crash messages contain expired snapshot; all 170 editing crashes do.
   reserve/copy_text require a current owner-thread snapshot; refresh establishes
   it, invalidate and scope Drop clear it. Inspect nested/reentrant binding calls,
   guard exit, parser callbacks, stylesheet/layout consumers and error unwind.
   Hypothesis: an inner scope clears authority still needed by an outer caller,
   or a native caller runs outside the admitted scope; neither is yet proven.
   Instrument first caller/entry/refresh/invalidate/exit and compare failing
   whitespace/paragraph fixture with passing sibling. Preserve heap accounting,
   thread/retired checks and callback cancellation. Never reuse a stale snapshot.
   Gate: minimal reproduction plus nested callback/error/detachment controls,
   then all affected 231 URLs and complete editing slice. Missing observations
   must recover without new losses before promotion.
3. Diagnose 24 stylesheet-completion routing crash signatures separately
   (three also contain expired-snapshot diagnostics; 21 in exclusive grouping). Signature:
   non-stylesheet completion in parser stylesheet owner. Trace exact request,
   resource kind, owner/publication generation and completion destination across
   CSS imports/font/child CSS/referrer fixtures. Check broker dispatch and parser
   pending-completion routing. Keep wrong-kind/stale-completion rejection; do not
   convert all completions into stylesheet success. Gate: import/media/layer/
   supports, cancellation, owner replacement and referrer controls.

### P1 — remaining execution losses

4. Group 23 JavaScript argument-conversion crashes by complete fixture and
   receiver realm: cross-realm IDB/WebIDL, OpaqueRange, history/inherited frames.
   Exception text alone is not one cause. Trace generated conversion and thrown
   exception identity; preserve authoritative WebIDL/receiver checks.
5. Profile 11 RSS crashes under production limits. Ten are Range fixtures:
   clone/extract/delete/surround/insert and character-data mutations. Inspect
   retained ranges, wrappers, mutation journals and repeated cloning/traversal;
   distinguish retained growth from legitimate peak allocation. Record per-case
   memory and assertion progress. No cap/deadline increases or assertion drops.
6. Diagnose 146 timeouts and 18 errors after P0 recovery. Prioritize lifecycle
   HTML (31 timeouts), editing (49), IDB (22), referrer (16), DOM (12), CSS (9).
   Retain exact transport/action/native timings. Separate completion bugs from
   missing APIs, unavailable fixture dependencies and scheduler costs.

### P2 — semantic and capability groups after observations stabilize

7. Editing: retain verified list-selection/extra-text gains; repair completion
   losses in join-pre/typing/link/word-deletion/caret cases. Then paragraph/line
   breaks, whitespace/plaintext, selection, formatting, undo/redo/input events.
   Current 2,191 FAIL observations include clipboard and whitespace groups;
   inspect full fixtures rather than mapping counts directly to defects.
8. CSS: current 1,250 FAIL observations include 220 SyntaxError messages and
   repeated top/bottom percentage/calc used-value mismatches. Partition CSSOM
   parsing/serialization from layout computed-value consumers and resource graph
   lifecycle. Run full 564 slice after the coherent CSS batch.
9. DOM/WebIDL: Range compareBoundaryPoints conversion/exception precedence,
   ProcessingInstruction attribute APIs, Range geometry and remaining interface
   behavior. Do not implement parallel JS substitutes. Preserve traversal tests.
10. IDB: independent exact comparison finds 168 whole gains/two losses versus
    ccb34108, but 17 gains/11 losses versus the older full run. Restore missing
    assertions before accepting gains. Source-backed createIndex investigation:
    multiEntry checked before key-path syntax, missing synchronous duplicate-name
    check and missing versionchange-mode check before store-context handling;
    matches eight FAIL and two timeout observations across globals. Reproduce
    exception precedence/transaction-mode controls before changing the algorithm.
    IDL groups include databases return metadata, static KeyRange members and
    IDBVersionChangeEvent metadata (nine failures per global). Keep remaining
    origin/realm/callback/timeouts separate; no measured performance cause yet.
11. Lifecycle/referrer/accessibility: diagnose parent/frame/popup/history fixture
    chains; implement native algorithms, not one blanket history fix. Keep
    EditContext/clipboard/animation/scrolling/moves capability work explicit.

### P3 — typed inventory completion and release qualification

12. Review staged genuine crashtest mapping/reconciliation with zero-subtest
    PASS permitted only for canonical crashtest. Add negative type/URL/hash and
    duplicate-observation controls; testharness strictness stays unchanged.
13. Implement a native child-resource evidence producer for the support
    obligation: signed candidate + original parent + child URL/realm/generation
    + broker resource content hash + imported assertion identities/multiplicity.
    Retired/stale/missing/mismatched evidence fails closed. Resolve parent crash
    first. A caller-supplied arbitrary receipt hash is insufficient.
14. Freeze and independently review candidate/runner; preserve native reuse
    proof for runner-only changes. Execute official native/topology/canary/
    placement and semantic/oracle/Stylo-Chromium differential gates; Linux and
    macOS full/lite, security/sandbox/recovery/production-performance gates.
    Two fully passing originals on identical signed artifacts must satisfy all
    4,022 typed obligations with no missing required observations. This does not
    establish universal Chromium/web-platform parity.

## Execution workflow

Independent agents diagnose disjoint lanes; coordinator integrates reviewed
patches and owns source/builds/signing/ports. Start with P0, not broad API edits.
Use focused developer regression controls during diagnosis. Integrate coherent
fix groups, then run their complete affected slices; do not run the entire
inventory after every small fix. Promote one full ledger after broad batches;
repeat only on a frozen candidate for final acceptance. After two unsuccessful
repair cycles revisit measured diagnosis. Keep all failed originals and staging
manifests; preserve .angular and recovery inputs. No changes to inventory,
assertions, production limits, networking authority or sandbox protections.

## Analysis artifacts

analyze.py SHA-256 4a6ff6bdb34e065abbf628680db1140e7d831b06968c5f4a89c907a628d3f464;
analysis.json SHA-256 d572cfd7d66cebd86fd11c372b3b6d5e70c6863255fb05a181461eca233a606f.
Contains exact report hashes, per-family failure URLs/messages and baseline
transitions. editing/RECEIPT.md records independent occurrence-based review.
Independent validity and IDB diagnostics are companion evidence, not approval
of current run. No runtime fixes were made for this analysis request.
