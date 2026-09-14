# Audited WPT harness v3

This supersedes the implementation checkpoint in
`WPT-HARNESS-SESSION-READINESS-2026-09-11.md`. Historical evidence is unchanged.

## Implementation and gates

- Persistent parent with signed workers, eight isolated prewarmed slots, fresh browser context
  per execution, permanent worker/page assignment, explicit cleanup and replacement.
- Bounded JSONL input/output, execution IDs, completion validation, fail-closed
  process/timeout/teardown handling, and conservative interrupted-run recovery.
- Unmodified upstream HTML and a versioned reporting adapter read by evaluation.
  Stateful/query-dependent prerequisites are observed during execution, not
  prefetched in a way that changes fixture state.
- Opt-in bounded parent HTTP audit: context/page/worker identity, request identity,
  URL with query values redacted, destination, status, MIME, incremental response
  SHA-256, byte count, timestamps, cancellation/failure. No bodies or credential
  headers retained. Cache hits and redirects are observed at the shared broker.
- Uncovered WebSocket traffic and audit overflow explicitly prevent qualification;
  they are never silently counted as complete HTTP evidence.
- Qualification binds source hashes, exact observed prerequisites, expected-error
  allowances, automation operations, assertion counts and successful shutdown.
  Missing evidence remains unresolved. Old schemas cannot establish a current
  release gate, and the release validator still requires clean source.
- Pool preparation now reserves capacity atomically and publishes prepared workers
  while holding the idle-pool lock. Checkout's empty-pool check and reservation
  are atomic with preparation, avoiding transient false exhaustion.
- Auxiliary popup pages use separate isolated on-demand workers in the WPT
  session only, so they cannot consume the eight primary prewarmed slots.
  Self-closed pages are accepted only on a confirmed closed/PageNotFound race;
  crashed pages and other teardown errors remain failures.
- Context-wide retirement drains workers even on logically closed/crashed pages
  and prevents late requests from launching workers after teardown starts.
- Submission is bounded to eight slots. Parent/pool integrity failures halt new
  submissions; ordinary per-test failures remain recorded without silent retries.
- Single-cohort full controller retains every manifest entry, including unsupported
  entries, and publishes a hash-bound mixed ledger plus category/tier totals.
  Thirty-second checkpoints include timing, queue depth and parent/worker RSS.

Validation after the auxiliary-page, retirement and checkout repairs:

- 157 native Rust unit tests passed, plus two bounded-session Rust tests.
- 95 Python harness tests passed with no skips, including signed live canaries,
  qualification-ledger merging, fixture shutdown ordering and final-summary state.
  The expanded tier/root reporting tests also passed after report finalization.
- Formatting, native/core-full check and clippy with warnings denied passed.
- Signed protocol-v9, designated-requirement and sandbox probes passed.
- Signed live checks covered DOM/global/cookie/localStorage isolation, twenty
  distinct workers, dynamic resources and intentional 404s, timeout replacement,
  and forced worker-crash replacement without retrying the failed execution.
  Added concurrent popups and repeated navigation/self-close/timeout churn.
- Chromium completion/subtest parity covered dynamic scripts, redirects, CSP,
  modules and nested frames.
- All 82 cookie executions finished with zero pool-capacity errors and zero
  retirement timeouts (9 raw passes, 73 raw failures; not qualified conformance).
- Four pinned canaries matched Chromium's subtest names/statuses and matched
  cold versus prewarmed execution exactly.
- A 1,000-execution eight-way stress check used 1,000 distinct primary workers,
  with no execution/cleanup/replacement/resource-audit errors and clean shutdown.
  Its 33.615-second elapsed time is a stability observation, not a performance
  acceptance benchmark.
- One additional encoding comparison completed all 1,000 subtests in Chromium
  while Oryn reached its 120-second deadline. See
  `WPT-ENCODING-TIMEOUT-TRIAGE-2026-09-11.md`; it is not a qualified codec failure.

Current parent binary SHA-256:
`c8b2c88e27a5d8b8171d7a35eca6779a3a23154ec737a1d697c866729e9ff8c7`.

Current signed worker SHA-256:
`935773342e4af046d6172f97d5bc9479907f6fb622283680356d63867749ace2`.

## Full baseline acceptance

The new 35,010-execution cohort is complete. The pinned
manifest has 128,554 entries, with 93,544 unsupported entries retained explicitly.
Results are signed-worker diagnostics from a dirty worktree, not a clean-source release
attestation. Raw passes are not automatically qualified conformance passes.

Current regression evidence is under `artifacts/wpt-full-baseline/audited-retirement-*`.
Older `audited-final-*` and `audited-popup-repair-*` slices remain historical.
The first `results-v3-audited-cohort/` attempt failed in preflight. Attempt
`results-v3-audited-cohort-2/` was stopped after auxiliary pages exhausted the
primary pool. Neither attempt is valid conformance evidence; both are retained
for diagnosis.

Attempt `results-v3-audited-cohort-3/` was also stopped: cookie/background
navigation fixtures exposed retained reservations on logically closed/crashed
pages. It is incomplete diagnostic evidence, not an accepted baseline. The
follow-up adds context-wide worker retirement, prevents late starts after
retirement, makes checkout atomic with preparation, and stops further test
submission immediately on parent/pool integrity failure.

The completed replacement is `artifacts/wpt-full-baseline/results-v3-audited-cohort-4/`.
All 35,010 exact IDs were verified; the native parent acknowledged shutdown and
exited with code zero. The controller completed in 24,442.2 seconds (6h 47m 22s).
Thirty-second normal and 120-second declared-long deadlines were preserved.
There were zero pool-capacity failures and zero worker-retirement timeouts.
Individual runtime crashes, limits and teardown failures remain failures, not retries.

| Tier | Raw pass | Raw failed | Environment blocked |
| --- | ---: | ---: | ---: |
| Core | 3,818 | 9,912 | 17 |
| Necessary | 2,583 | 12,609 | 0 |
| Extended | 143 | 5,879 | 49 |
| Total | 6,544 | 28,400 | 66 |

Strict qualification produced four explicitly reviewed passing canaries,
25,359 review candidates and 9,581 unresolved executions. Neither candidates
nor raw passes are qualified conformance claims. The 66 environmental blocks
are 47 confirmed upstream template failures and 19 cases requiring unavailable
distinct local/public IP spaces. All 93,544 unsupported entries remain in the ledger.

Final artifacts:

- [Category/tier/type report](../../artifacts/wpt-full-baseline/diagnostic-v3-final-cohort-4/README.md)
- [Full 128,554-entry qualified/unsupported ledger summary](../../artifacts/wpt-full-baseline/ledger-v3-audited-cohort-4/summary.json)
- [Strict qualification overlay](../../artifacts/wpt-full-baseline/qualified-v3-audited-cohort-4/summary.json)
- [Explicit four-canary reviews](../../benchmarks/conformance/wpt-qualification-reviews-v3.json)

### Finalization repairs and provenance

After every execution and native parent exit, two fixture-proxy threads remained
in upstream response reads. Inactivity timeouts do not bound trickling responses.
The unused fixture helper received SIGTERM; it and the runner then exited normally.
No test was retried and no raw outcome was changed. This is a disclosed recovered
fixture shutdown, not evidence of a fully unattended teardown on the old runner.
Future teardown stops upstream after page retirement but before joining proxy
handlers, with a bounded terminate/kill fallback. Ordering and fallback tests pass.

The controller also retained its last `running` checkpoint in an otherwise
completed summary. It now refreshes terminal outcome/progress from the final raw
result. The original summary is preserved as `summary.before-final-status-repair.json`
with SHA-256 `5613dbc1d5b3e2526c61c49381a9c6c8377774bec3ba7c43ec6c174c3b3f286a`;
the corrected summary records the repair and original hash. The raw result remains
unchanged at SHA-256 `37d482dff827ae332b0a66c73dd8c4ef3d3a354a0bcfa7ff9dcec6565cfd6e47`.
Both executable hashes above were rechecked unchanged. Final qualification and
report hashes bind the corrected summary and the unchanged raw evidence.

### Core fix priorities

1. Reproduce and eliminate crashes/resource/transport failures before interpreting
   assertion results. Across all tiers, there are 12 explicit worker crashes,
   65 resource-limit outcomes and 775 runtime/transport errors; these are not all
   separate defects. Core contributes six explicit crashes and 568 transport/runtime errors.
2. Investigate HTML lifecycle, forms and navigation first: 3,633 Core raw failures,
   including 2,329 assertion-only and 987 incomplete outcomes. Encoding's 1,299
   failures include 960 timeout/limit outcomes; the existing Chromium form-submission
   comparison makes named-frame/form/load behavior a concrete investigation lead,
   not proof of 1,299 codec defects.
3. Audit representative CSP/fetch/XHR cases, then fix shared algorithms: Core has
   641 CSP, 591 fetch and 333 XHR raw failures. Keep fixture/network/automation
   qualification separate from engine fixes, especially for the unaudited WebSocket transport.
4. Follow with DOM/shadow DOM/custom elements, trusted types and streams. The
   report contains exact Core root/type counts; choose source-reviewed canaries
   and rerun focused slices before claiming wider conformance gains.
