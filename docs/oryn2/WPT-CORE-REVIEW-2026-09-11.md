# Core baseline classification review

Reviewed completed `artifacts/wpt-full-baseline/results-v2` with
`scripts/review-core-wpt-baseline.py`. Reproducible exact-ID symptom clusters and
input hashes are in `artifacts/wpt-full-baseline/core-review.json`.
No runtime changes or tier reassignment have been applied by this review.

## Main finding

Core contains 12,222 executed cases: 1,907 pass, 10,315 fail. Of those failures,
4,270 are assertion outcomes, 4,287 harness outcomes, 1,743 runtime outcomes,
10 outer timeouts and five incomplete outcomes. These labels describe where the
run ended, not who owns the defect. Missing browser APIs can cause harness errors;
broken fixture delivery can cause assertion failures.

The baseline cannot yet support a clean browser-compatibility ranking. In particular,
referrer-policy now passes 11/1,391 compared with historical 1,383/1,391. CSS retains
368/564 and DOM passes 430/661 in this selection. Selection differs from historical
DOM coverage, so those totals are not directly interchangeable.

## Classification corrections to make in the next policy version

- Keep product priority (Core/Necessary/Extended), execution environment, runner
  support, outcome domain and diagnosis confidence as separate fields. Platform
  dependence is not intrinsically low product priority; WebAuthn is an example.
- Restore iframe/object document lifecycle to Core using precise path overrides.
  The current blanket `html/semantics/embedded-content` Necessary rule also
  demotes iframe behavior, despite frames being central to Core navigation.
- Review promotion of FileAPI, streams and ordinary WebSockets to Core because
  they support uploads, Fetch bodies and common application communication.
  Promote SRI, Trusted Types and permissions-policy enforcement alongside the
  existing Core security contract. Keep specialized subfeatures separately ranked.
- Audit default-Extended roots: clear-site-data, credential-management, permissions,
  compatibility/quirks and loading/session behavior deserve explicit decisions.
  An unmatched prefix is a classification backlog, not a low-priority verdict.
- Split the broad Core HTML tree: ordinary forms/scripts/navigation are urgent;
  drag-and-drop, specialized interaction and peripheral legacy cases should not
  automatically share their implementation priority.
- Keep Test262 and visual/manual suites as separately reported test types. The
  93,544 unsupported entries are unsupported by the runner, not 93,544 missing APIs.

Version these changes and reproject existing results by exact IDs; preserve the
original policy and baseline counts. Do not rerun merely to relabel tiers.

## Prioritized Core work queue

| Order | Group | Evidence / next action |
|---|---|---|
| 0A | Fixture serving and WPT origin topology | 3,130 executions have the exact harness message `Unexpected token ':'`. Recorded script diagnostics identify common.sub.js in 2,100 executions and get-host-info.sub.js in 674 (resource counts may overlap). Their templates require ws/wss and second HTTP/HTTPS ports, while the helper config supplies only one HTTP and HTTPS port. Capture actual responses, correct the full origin/transport setup, and rerun representative fixtures plus referrer-policy before interpreting these failures. |
| 0B | Encoding fidelity and execution budgets | Encoding has 963 runtime outcomes; the HTML serving path decodes UTF-8 with replacement. Preserve original bytes and declared charset around instrumentation. Across Core, 1,660 first diagnostic errors explicitly report worker wall-time limits, plus seven generated-operation wall-time limits. Profile representative fixtures; these are not equivalent to crashes and are not represented by the ten outer timeouts. |
| 1 | Frames, Window identity, navigation and messaging | HTML browsing-the-web: 298 failures; origins: 116; webmessaging: 129. Null document/open and missing postMessage recur. Reproduce initial iframe documents, popup creation, stable WindowProxy identity, cross-document navigation and message delivery. Repair shared lifecycle causes with exact fixture proof. |
| 2 | URL, Fetch/XHR and session state | Fetch: 637 failures; XHR: 367; cookies: 80; CORS: 27; Web Storage: 46. First establish valid secure/cross-origin fixtures, then group request/response body semantics, credentials, redirects, cancellation and storage isolation. Seventy-six exact first failures say synchronous main-thread XHR is unsupported; distinguish explicit policy from implementation defects, including worker-context behavior. |
| 3 | HTML parser, script/module loading and components | HTML scripting: 398 failures; parsing: 209; custom elements: 173; Shadow DOM: 244; import maps: 56. Trace parsing and script order before patching component symptoms. Twenty first failures report invalid/collected WrapperId, a concrete binding/lifetime candidate. |
| 4 | Forms, input, focus and selection | HTML forms: 405 failures; HTML focus: 152; Selection: 86. Prioritize ordinary form value/validation/submission, activation, event ordering, focus and caret/range mutation. Audit testdriver support before calling input failures runtime defects. |
| 5 | CSS integration and accessible semantics | Core CSS: 196 failures; accname/ARIA/HTML-AAM: 23 total failures. Preserve Stylo and address stylesheet ownership/loading, live CSSOM, state invalidation and accessible-name updates. These are smaller, better-established pools with passing neighbors. |
| 6 | Remaining DOM/WebIDL and specialized APIs | Screen bounded traversal/removal, constructor/brand/default-value and XPath gaps after shared realm fixes. Do not infer a complete-fixture gain from fixing its first missing symbol. |

Counts above are overlapping diagnostic pools, not additive gain forecasts. Group
0 is evidence repair; groups 1–6 are the initial runtime priority order, subject to
the repaired baseline. Security defects with actual reproduction can preempt it.

## Acceptance for each batch

Record exact execution IDs, representative symptoms, reproduced cause, production
owner, passing neighbors, later blockers and expected observable behavior. First
rerun a bounded family, then its affected roots. Require whole-execution and subtest
reconciliation, all observations present, and no new regressions. Keep production
limits and policy decisions explicit. Publish a new fingerprint for any harness
change; do not merge it silently into the original completed baseline.

The immediate next task is 0A: repair and qualify shared WPT helper responses and
origin topology. The one-case substitution canary used before the baseline was
insufficient to qualify all shared `.sub.js` dependencies.
