# Full WPT capability roadmap

This supersedes the scope and 4,022/4,022 completion requirement in older plans.
Full pinned WPT is the coverage universe. Historical ledgers provide overlapping
regression evidence, not a product boundary or a mandatory separate milestone.

The policy is `benchmarks/conformance/wpt-capability-tiers-v1.json`. Longest path
prefix wins. Unmatched areas remain Extended and explicitly require classification.
Membership depends on product relevance, never current pass status.

| Tier | Purpose and implementation priorities |
|---|---|
| Core | Ordinary loading and agent actions: frames/realms/history, Fetch/XHR/CORS, URL/encoding, HTML forms, DOM/events, modules/components, accessible actions, session/security behavior |
| Necessary | Modern apps: CSS/layout, editing, storage/IndexedDB, workers/service workers, streams, observers, SVG/canvas and scheduling |
| Extended | Specialized capabilities and areas awaiting classification |
| Platform-dependent | Device, OS or service prerequisites; retain coverage and assess the actual environment needed |

Test type and environment are separate dimensions. Platform-dependent does not
mean every test requires hardware. Review broad rules at subdirectory granularity,
particularly embedded HTML content and default-Extended security/application APIs.

## Current inventory versus full scope

The entire pinned Git tree maps to 271 non-infrastructure roots; 219 have no entries
in the current expanded subset. These are source areas, not proven missing APIs or
runnable-test counts. The generated `wpt-capability-catalog-v1.json` binds policy
and inventory hashes and marks unclassified source files.

| Initial subset | Files | Execution variants |
|---|---:|---:|
| Core | 9,266 | 11,485 |
| Necessary | 3,414 | 5,320 |
| Extended | 28 | 37 |
| Platform-dependent | 0 | 0 |

These classify only the existing 16,842 executions. They are not pass results or
the full denominator. Do not project the historical 3,144/4,022 score onto them.

## Baseline and implementation sequence

1. Materialize the full pinned checkout and generate the official WPT manifest.
   Sparse checkout and suffix scanning cannot establish full coverage. Include
   XHTML/SVG, generated globals/variants, reftests, print and manual test types.
   Exclude support/reference files from executable denominators.
2. Classify every entry by tier, feature, type, environment and execution support.
   Audit default-Extended roots for priority corrections.
3. Repair harness prerequisites for module workers and actual service-worker
   registration/lifecycle. Unsupported contexts remain explicitly reported.
4. Measure Core, then Necessary, then Extended and environment-specific suites.
   Track pass, assertion/harness/runtime failure, timeout, crash, unsupported,
   not-run and environment-blocked separately. Nonexecuted entries stay visible
   in coverage, outside observed pass-rate denominators.
5. Publish whole-execution and observed/passing-subtest counts per tier and
   feature, plus cumulative tier totals, missing observations and completion.

First implementation candidates remain diagnosed NodeIterator and initial iframe
documents. Broader measurements then rank application loading, navigation,
forms/input, stylesheet ownership and async/storage behavior by user impact,
dependencies, shared cause and complete-fixture yield. Large legacy editing
matrices do not automatically outrank missing application capabilities.

Accept batches against a frozen signed candidate with exact declared selection,
production limits and reconciled regressions. Full conformance requires coverage
of the declared tier/environment, not completion of an arbitrary historical count.

Regenerate the source catalog using `scripts/catalog-wpt-capabilities.py` with
`--checkout /path/to/pinned-wpt --output benchmarks/conformance/wpt-capability-catalog-v1.json`.
Its source counts include support files; full executable manifest generation and
tier baseline runs remain pending.
