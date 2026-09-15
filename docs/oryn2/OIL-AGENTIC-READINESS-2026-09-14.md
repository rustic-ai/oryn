# OIL agentic browser readiness

Objective: a functional, sufficiently conformant native browser for five main
agentic use cases through OIL. The following five families are provisional
engineering targets, not a measured popularity ranking. None is qualified yet.
They supplement, not replace, the browser architecture/readiness plan and its
security gates. A passing narrow fixture does not close a workflow family.

| Family | Required end-to-end behavior | Important failure/recovery controls |
| --- | --- | --- |
| Research and extraction | Search, follow results, read and extract structured facts, compare multiple pages, retain source URLs | Dynamic loading, pagination, stale references, redirects, missing results, cross-origin frame boundaries |
| Forms and document submission | Fill and validate multi-step forms, select options, upload authorized files, submit and verify server receipt | Invalid input, cancelled submit, multipart/body fidelity, disabled controls, navigation, file/secret confinement |
| Shopping | Search/filter catalog, compare products, select variants, edit cart, complete a controlled checkout | Stock/price changes, cart persistence, quantity correctness, duplicate-submit prevention; no real purchases during tests |
| Authenticated SaaS work | Login/SSO, navigate an SPA, create/read/update a record and verify persistence | OAuth popup/redirect, MFA handoff, expired session, async updates, context isolation, sensitive-output redaction |
| Booking and scheduling | Search availability, select date/time/options, enter details, review and submit a controlled reservation | Time zones, unavailable slot, validation, stale selection, duplicate booking; no real reservations during tests |

## Evidence required for each family

1. Versioned OIL tasks with explicit expected outcomes, including negative and
   recovery cases. No JavaScript used to mutate the application instead of OIL
   actions. Read-only inspection cannot be the sole proof of server-side writes.
2. Controlled fixtures with independent server-side or persisted-state oracles,
   plus representative real application/framework behavior. Fixture success
   alone is not proof of general website compatibility.
3. Exact signed parent/worker/source/fixture/task identities, raw command outputs,
   timeouts, action errors and cleanup results. Failures stay in the denominator.
4. Relevant WPT families and native security/lifecycle regressions, including
   initial iframe realms, forms/adoption, connected-window routing and task
   retirement where the workflow depends on them.
5. Repeated runs and concurrency/isolation checks on a frozen candidate. Preserve
   upstream conformance, task success, harness validity and performance as
   separate results. A full completion audit must cover every family and the
   applicable architecture gates, not merely aggregate successful commands.

## Current implementation audit

The original twelve-script smoke corpus is not sufficient task-completion
evidence: an observation and absence of unsupported commands do not prove final
outcomes. Fixture dependencies have since been restored. Current outcome work is:

- Forms: 19 controlled cases with independent submission/upload receipt checks.
- Shopping: seven controlled cases with persisted carts, quotes, inventory and
  orders, including stale price/stock and duplicate-submit controls.
- Booking: six versioned OIL cases in `booking-cases.json`, with a SQLite fixture
  and `run-oil-booking-outcomes.py`. They cover the distinct UTC instants in a DST
  fold, required-input rejection, stale availability and recovery, duplicate
  submission and confirmation reload. No real reservations are made.
- Authenticated SaaS: five controlled auth-mechanics cases exist, but persisted
  authenticated create/read/update workflows and real token-exchange security
  coverage remain incomplete. A new SQLite fixture backend in
  `test-harness/workflows/saas_fixture.py` now implements synthetic login, scoped
  session/CSRF rotation, versioned record creation/update, logout and oracle-only
  session expiry. Six unit tests cover persisted writes, expiry/re-login,
  cross-case isolation, duplicate/stale saves, invalid login and duplicate-field
  rejection. Session and CSRF values are excluded from snapshots. Loopback HTTP
  serving and bounded stdin-only controls are implemented; a seventh integration
  test verifies cookie login, redirects, persisted create/update, expired-session
  rejection and clean sealing. `saas-cases.json` defines an initial OIL
  create/update/reload/logout contract. The independent runner now verifies
  native action counts, extracted values, final location, persisted records,
  authorization history and final session state after server shutdown. Four
  validator tests reject missing writes/actions and forged authority/identity.
  The initial browser diagnostic passes **3/3**, with final persisted oracles,
  unchanged artifacts and clean shutdown. Its source does **not** match the
  existing signed build; see
  [initial diagnostic](../../benchmarks/evidence/saas-initial-2026-09-14/summary.json).
  The expanded three-case contract adds invalid-login recovery and logged-out
  access denial. The validator now requires the exact persisted page-authority
  sequence as well as mutation/session state. The stricter diagnostic passes
  **9/9**, with final database checks and cleanup verified; it remains
  source-mismatched to the previous signed worker. See
  [authorization diagnostic](../../benchmarks/evidence/saas-view-authorization-2026-09-14/summary.json).
  A fourth case now deterministically expires the session after the created
  record's edit page is rendered. OIL observes a rejected save, re-authenticates,
  updates the same record and logs out. The database verifies an expiry event,
  rejected unauthorized mutation, two successful logins and one record at
  version two. All **12/12** expanded diagnostics pass; source mismatch remains
  explicit. See [expiry diagnostic](../../benchmarks/evidence/saas-session-expiry-2026-09-14/summary.json).
  Signed current-source qualification, representative
  authenticated SPAs, MFA and token-exchange coverage remain pending. This does
  not qualify the family.
- Research/extraction: six controlled cases now cover paginated comparison,
  canonical redirects, fetch-loaded results, empty results, missing-source
  recovery and refreshed results. Exact text/table/link outputs and canonical
  observation URLs are checked against a persisted SQLite request sequence.
  This is not broad research or cross-origin-frame qualification. Native
  extraction now scopes all content kinds to the current document and requested
  subtree; detached and other-document arena nodes are excluded.

Eight real framework builds provide a separate compatibility corpus, not blanket
qualification of these families. See [implementation status](IMPLEMENTATION_STATUS.md)
for candidate-specific results and source/signature identities. Fixture/backend
unit tests do not substitute for browser OIL runs, and a previous signed binary
with changed source is only a diagnostic until rebuilt and requalified.

Next work includes broader research and persisted SaaS outcomes, synchronous
initial iframe realms, native WindowProxy/connected-window ownership and
per-document routing. The remaining architecture, WPT, security and performance
gates remain open regardless of controlled workflow success.
