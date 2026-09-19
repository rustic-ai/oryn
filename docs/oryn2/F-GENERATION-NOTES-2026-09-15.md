# Package F parallel source-generation notes

September 15, 2026. **Source generation only; no build, test, formatter, dependency
installation, deployment, browser run, or qualification was performed.** This does
not close package F or change any historical outcome.

## Reviewed foundations

The frozen `five-family-contract-v1.json` contains 29 behavior cells. Existing
research/forms/shopping/SaaS/booking SQLite fixtures and runners preserve exact
controlled-case request and outcome checks, sealed/reopened databases and artifact
identities. The integrated runner reuses these plus the existing framework/auth
gates. SaaS and booking already have versioned frontend additions. None of these
facts establishes full matrix completion.

## Generated implementation and test sources

- `test-harness/workflows/research_fixture_v2.py`: extends the existing research
  backend with server-rendered search markup, hydration and JavaScript module
  assets. Existing case existence checks and independent request database remain
  authoritative. Delivered shell and asset bodies have persisted hashes.
- `test-harness/workflows/research_fixture.py`: optional fixture type/extra exact
  routes and MIME override support for that adapter; default v1 behavior retained.
- `test-harness/workflows/v2/research-client.mjs` and `research-search.mjs`:
  hydration, dynamic route chunk, asynchronous source requests, pagination/history,
  empty-result recovery, request cancellation and generation-bound stale-response
  suppression. No synthetic extracted facts or successful browser observations.
- `test-harness/workflows/research-cases-v2.json`: new hydrated-pagination and
  client-search/empty-recovery OIL tasks with exact independent request/extraction
  contracts. Existing immutable cases remain unchanged.
- `scripts/run-oil-research-outcomes.py`: explicit `--frontend-v2` selection with
  fixture, case and module hashes using the existing execution/oracle path.
- `scripts/run-oil-integrated-outcomes.py`: runs this added frontend corpus as
  another separately attributed result, still explicitly unqualified for the full
  contract; no historical summaries are rewritten.
- `scripts/test_oil_research_frontend.py` and
  `test-harness/workflows/v2/research-search.test.mjs` and
  `research-client.test.mjs`: sources for shell/MIME/hash,
  exact oracle, missing/foreign evidence, response/chunk failure, stale response,
  retirement and abort controls.
- `scripts/workflow_contract.py`: mixed-family planned attempts now explicitly
  include each frozen context count (1/5/20), three samples per count. Plan schema
  v2 and context-qualified attempt IDs distinguish this corrected expansion from
  earlier plans; the immutable v1 product contract is unchanged. Other modes retain
  their sample requirements. `scripts/test_workflow_contract.py` updated accordingly.
- `scripts/workflow_evidence.py` and `test_workflow_evidence.py`: full-contract
  structural evidence reconciliation: no dropped failures/replaced retries,
  candidate identity, pinned independent oracle and native-action artifacts,
  fault injection records, successful cleanup, frontend dimension accounting and
  overlapping distinct context activity. Expected outcomes are supplied separately
  from the runner. This validator is a library for the future full-contract
  controller; it does not turn the existing regression run into full qualification.

## Open work (not an exhaustive Core inventory)

- Research: same/cross-origin nested frames, real stale semantic reference rejection,
  detached/foreign-document controls and complete browser integration of history,
  cancellation and chunk-error recovery. Client generation guards do not establish
  native stale-reference conformance.
- Forms: expanded multistep/custom-control frontend, asynchronous validation and
  full confinement/reentrancy/authority matrix beyond controlled legacy cases.
- Shopping: hydrated/CSR frontend and cross-origin framed controlled payment,
  asynchronous price/stock behavior and complete interruption/retry controls.
- SaaS: complete session expiry, isolation/redaction, OAuth rejection and popup
  recovery matrix; existing OAuthlib-v2 source requires subsequent qualification.
- Booking: full frontend options/DST/fault validation and mechanism closure for the
  original timeout; later non-recurrence is insufficient.
- Full-contract orchestration must bind every behavior cell to concrete adapters,
  independently frozen oracle inputs, all required repetitions and actual fault
  injection. Current integrated controller executes regressions and expanded
  frontend subsets only. It does not schedule the full corrected attempt matrix.
- Measured context activity and dimension/oracle artifacts must come from audited
  runtime/controller collection, not hand-authored claims. Structural acceptance
  by `workflow_evidence` is not cryptographic or runtime/security qualification.
- All eight framework gates, source/signature identity, mixed 1/5/20 concurrency,
  cleanup, performance and security still need the same final frozen candidate.

All generated sources remain unbuilt and untested under the user's global gate:
finish all implementation and test generation first, then build, test, repair and
verify complete conformance.

## Additional forms/shopping generation

The second parallel batch adds:

- `forms_fixture_v2.py`, `v2/forms-client.mjs`, `forms-cases-v2.json`:
  multi-step form, a portalled custom role picker backed by the original successful
  control, asynchronous eligibility validation with stale-response suppression,
  cancellation/recovery and actual native authorized upload. The native form still
  owns encoding, Unicode/order, file bytes and submitter behavior. Five new cases
  retain exact receipt and validation-history oracles. `forms_fixture.py` exposes
  optional rendering/GET extension hooks; v1 defaults are preserved.
- `scripts/run-oil-forms-outcomes.py`: explicit frontend selection, action counts
  that exclude waits, async validation-history checks and post-run input/candidate
  integrity checks. Added Python receiver/byte-fidelity tests and JS asynchronous
  validation/retirement controls (`test_oil_forms_frontend.py`,
  `v2/forms-client.test.mjs`). No native upload or submission result is claimed.
- `shopping_fixture_v2.py`, `v2/shopping-client.mjs`, `shopping-cases-v2.json`:
  hydrated catalog/cart/review routes over the existing SQLite mutations, plus a
  genuinely distinct-origin controlled payment iframe. One-time quote authority,
  payment receipt and authority history persist independently; checkout still
  enforces session, stock, price, inventory and duplicate-order rules. The
  additional server is retired before fixture sealing/database closure.
- Seven new framed-shopping cases use genuine OIL `frame css(#payment-frame)` /
  `frame main` commands, native frame click, and exact payment/oracle checks.
  `test_oil_shopping_frontend.py` and `v2/shopping-client.test.mjs` contain controls
  for foreign origin/source/quote, replay, cross-case use, stock/price changes and
  one-order inventory mutation. Action identity is page-local across frames.
- Both new frontend corpora are separately scheduled by the existing integrated
  controller. Full-matrix qualification remains false, regardless of subset results.

Source review found `parse_frame` previously discarded its target and returned
Main. The parent reports generating the parser repair and regression source;
**native frame selection/dispatch/integration remains an acceptance dependency**.
No fallback bypass was introduced in the framed tasks. The fixture's payment is
entirely controlled test activity, with no external commerce.

Remaining F scope is still substantial: full GET/POST/submitter/reentrancy and
cross-context file/secret confinement; comprehensive frame research; shopping
interruption/fault controls; full auth session/rejection/isolation matrix; and
actual full-contract adapter scheduling, instrumentation and frozen oracle
collection. The new forms and shopping sources reduce frontend gaps but do not
close these requirements. All code and tests remain unexecuted.

## Additional PKCE expiry recovery corpus

`saas-cases-v3.json` preserves v2 cases and adds actual PKCE relogin after controlled
session expiry. The case asserts a denied expired save, two consumed code flows,
exact persisted authorization/expiry history and one recovered record version.
`run-oil-saas-outcomes.py --frontend-v3` selects it explicitly; the integrated
controller retains v2 and runs v3 under a separate output directory. Additional
`test_saas_frontend_v2.py` source checks cookie/CSRF rotation, rejection of old
credentials after relogin, persisted record recovery and redacted independent
snapshots. This is generated coverage, not executed evidence; complete OAuth
negative/recovery OIL cases and cross-context runtime qualification remain open.

## Full-contract scheduling source

`workflow_execution.py` adds a reusable complete-plan scheduler with an exclusive
output directory, pinned plan identity, fsynced append-only attempt journal,
separate serial/mixed/recovery/fault phases, candidate probes and identity-owned
adapter results. Missing adapters, cancellation, adapter errors and interrupted
infrastructure retain a record for every planned obligation. It invokes the
structural evidence reconciler and never sets family/release qualification true.
`test_workflow_execution.py` contains missing-adapter, cancellation, identity,
candidate-change and evidence-overwrite controls.

This is an execution library, **not a claim that all family adapters exist**.
Concrete per-cell adapters must supply actual native concurrent-context activity,
fault injection and independent frozen-oracle artifacts; the regression controller
cannot truthfully substitute its narrower case batches for those adapters. Full
CLI/controller integration remains open until those complete adapters exist.

## September 16: concrete native execution and expanded durable fixtures

Source generation remains unexecuted. The new implementation sources are:

- `crates/oryn/src/bin/oryn-workflow-qualification.rs`: a single production
  Browser runs explicit 1/5/20 native context plans concurrently. Real OIL output,
  worker handshake metadata, context IDs, operation results, resource audits,
  close/retirement errors and explicit fault results are emitted as artifacts.
  Plans are bounded and strict; outputs use create-new semantics. Authorized
  uploads are imported only into their named context. No environment variable
  enables fault behavior. Cancellation is not called injected merely because a
  future was dropped; worker retirement must complete. Worker failure uses the
  root-owned checked kill/reap API at an idle pre-step boundary, not a claimed
  mid-operation crash. Duplicate delivery performs an actual second OIL action.
- `crates/oryn-native/src/workflow_instrumentation.rs` and the narrow OIL session
  wrapper: opt-in shared monotonic-clock spans for actual native execute futures,
  terminal completed/failed/dropped state, unique operation IDs and hashed input
  (no credential text). The Python validator uses individual overlapping spans,
  not context lifetime envelopes or time spent waiting at the launch barrier.
- `scripts/workflow_native_adapter.py`, `workflow_family_recipes.py` and
  `run-oil-full-contract.py`: concrete recipes for 22 cells reuse existing tasks
  and validators across all five families. SQLite fixtures are sealed and reopened
  independently. Actual facts, receipts, records, carts, orders, stock,
  reservations and authority histories are projected against pre-execution
  expectations. The exhaustive scheduler persists those frozen expected oracles.
  Missing adapters and fault recipes remain failures in the complete denominator.
- `research_fixture_v3.py` and `research-cases-v3.json`: actual same-origin,
  distinct-origin and nested frames, parent access-denial observation, extraction
  of each document's facts, and removal without leaking detached subtree facts.
  Real OIL frame commands are retained. The existing research runner and integrated
  controller now select this separate corpus. Page-local action IDs are scoped to
  their page when validating framed output.
- `workflow_faults.py`: exact-route, one-shot HTTP response withholding controlled
  only through the private fixture pipe. SQLite records distinguish armed,
  actually withholding and released states; arming without a request is not
  injection evidence. Research v3 exposes this hook and retains the final database.
- `forms_fixture_v3.py` and `forms-cases-v3.json`: the native form receiver and v2
  controls now have a durable SQLite oracle. Validation, requests, receipts and
  receipt reload counts persist before the relevant HTTP operation completes.
  Upload artifacts retain exact authorized filename, size and byte hash.
- New regression sources: `test_workflow_native_adapter.py`,
  `test_oil_research_frames.py`, `test_workflow_forms_durable.py`, and native
  harness/instrumentation unit sources. These include false-overlap, failed or
  dropped operations, missing adapters, response-fault non-injection, unknown-case
  isolation, durable rejection and malformed plan controls.
- Two existing SaaS v3 cases had the same ID after merging corpus additions.
  Both task sources are preserved; the additional regression now has a distinct
  `-v3-regression` suffix so concurrent output paths cannot collide.

### Remaining actual source dependencies

This batch does not complete F/Q. In particular:

- The current per-cell adapter waves contain same-cell concurrent contexts. They
  execute genuinely in one parent but **do not satisfy heterogeneous mixed-family
  scheduling**; mixed-mode attempts explicitly retain that failure until companion
  recipes are wired into the same native plan.
- Fault primitives are real, but per-cell interrupted-action/restart/retry recipes
  are not yet connected to the full adapter matrix. Fault-mode attempts reject
  normal-run substitution. The idle worker kill boundary is not mid-operation
  interruption coverage. Private response-delay events need orchestration and
  durable fault-artifact reconciliation.
- Seven cell adapters remain absent: research stale references; forms full
  GET/POST/multipart delivery, submitter overrides, reentrancy and confinement;
  SaaS OAuth rejection and cross-context isolation. Existing native/backend
  controls are not substituted for OIL behavior evidence.
- Some mapped recipes still need broader recovery semantics: SaaS duplicate saves
  and shopping interruption/retry, for example. A mapped recipe is executable
  source, not a declaration that its full cell requirement is closed.
- Signed packaging of the qualification executable, all final-candidate source
  and signature checks, the eight framework gates and performance/reliability
  qualification remain in the final execution phase. No current passing result
  has been collected or claimed.

## September 16: shared-parent qualification and commit-boundary source update

This section supersedes earlier source-gap lists that described seven unmapped
cells or homogeneous-only mixed workloads. It is a source inventory, not a test
result. No build, formatter, generator, test, fixture server, or qualification run
has been executed during this generation work.

- All 29 frozen behavior cells now have concrete native task/fixture adapters.
  Added expanded forms delivery/submitter/reentrancy/confinement cases, five real
  OAuth rejection cases, isolated SaaS probes, and native stale-reference tasks.
- Mixed workloads assemble all five families in one native Browser parent at
  cohort counts 5 and 20, with count 1 retained as the control. Additional
  adversarial contexts are explicitly identified and included in total live
  context topology. Real per-operation audit spans provide overlap evidence.
- Each of the ten recovery-sensitive cells has explicit delayed-response, idle
  worker-loss, actual duplicate-delivery, and inflight read-cancellation recipes.
  These mechanisms remain separately labeled; idle termination is not presented
  as mid-write interruption.
- Mutation-family recovery attempts now additionally execute both before-commit
  and after-commit/before-response waves for worker-failure and cancellation, at
  the requested cohort count. Research remains a read-only family and retains its
  actual reference/read interruption recipes.
- The fixture commit gate persists the original request privately, records real
  entry/release events, and publishes a token-bound marker only when the request
  reaches its selected route/ordinal. Before-commit faults permanently abort the
  original mutation, including on timeout; after-commit faults preserve the
  committed effect. No timer-only configuration counts as injection.
- The native executor monitors the gate while an OIL operation is active, requires
  the parent runtime's checked `interrupted_inflight` and reaped-worker evidence,
  releases the fixture only afterward, and waits for its persisted completion
  acknowledgement. A replacement page stays in the original context.
- Recovery reads actual persisted receipts/orders/reservations/record versions
  through a token/session-protected fixture page. Only `not_committed` exposes
  the original request for native form retry. `committed` exposes continuation.
  Conflict/unresolved outcomes fail. Before-commit retries must reproduce exact
  original body bytes, cookie binding and method. Sealed recovery ledgers and
  actual durable projections must match the unchanged source baseline; committed
  writes must produce no retry ledger entries.
- Added source regressions for marker identity, exact request ordinal/body,
  precommit timeout/no late write, postcommit preservation, foreign recovery
  authority, changed durable state, wrong retry body, and all mutation-cell
  recipe boundaries. These tests are unrun.

### Current files for this batch

`crates/oryn/src/bin/oryn-workflow-qualification.rs`,
`crates/oryn-native/src/workflow_instrumentation.rs`, narrow `oil.rs` audit hooks
and `lib.rs` export; `scripts/workflow_native_adapter.py`,
`workflow_family_recipes.py`, `workflow_fault_recipes.py`,
`run-oil-full-contract.py`, `workflow_execution.py`, `workflow_evidence.py`;
`test-harness/workflows/workflow_fixture.py`, `workflow_faults.py`, versioned
research/forms/SaaS fixture/task sources and narrow booking/shopping routes;
associated Python/Rust/JS test sources.

### Remaining acceptance and integration work

The source paths still need the final permitted generation/build/test/fix phase.
In particular, actual native cancellation must prove a worker operation remains
in flight at each asynchronously observed fixture gate; an idle result fails and
cannot qualify the wave. The gate implementation does not claim arbitrary
production-network fault injection. It uses explicit controlled fixture commit
boundaries and actual production native sessions. Full repeated sampling,
resource-budget acceptance, zero unclassified Core failures and all family
oracles remain unverified. All qualification/conformance completion flags remain
false until genuine runs establish them.

## Q source audit: resource and existing performance profile integration

Inspected F-01..F-04 and Q-01..Q-06 against the current source and retained the
fixed acceptance targets. Existing `sandbox_page_benchmark` and
`page_group_benchmark` measure pages within a context, not independent browser
contexts; the new independent-context workflow executor remains the source for
that separate obligation.

Generated `workflow_resources.py`: bounded OS descendant process sampling with
native HTTP audit PID/page/context reconciliation, sampled per-worker peaks,
parent RSS and simultaneous aggregate/group RSS. Every descendant contributes to
aggregate footprint; unknown ownership is explicit and fails integrated Q
checks. Sampling is labeled as sampling, not a kernel high-water measurement.
Short worker lifetimes missed by sampling remain missing evidence. Raw samples
and accounting artifacts are retained and hash-bound by the workflow evidence
layer. Native OIL steps additionally retain measured execution duration.

Generated `run-workflow-qualification-profiles.py` and
`workflow_qualification.py`: source-frozen 30-sample isolated/prewarmed/recycled
profiles, 30 group samples at each of 1/5/20 pages, and 30 existing reference-churn
runs. Uses explicitly supplied prebuilt binaries; performs no automatic build or
signing. Freezes executable hashes and the plan before execution, records every
stdout/stderr/resource/exit/cleanup result, rejects candidate drift and refuses
replacement of failed attempts. The fixed cold/warm/binding/RSS thresholds are
checked against raw measurements. Component checks never certify complete Core,
frameworks, signing, manual/security or independent-context obligations.

Regression source covers malformed/duplicate process data, missed worker
lifetimes, simultaneous aggregate versus summed peaks, nonfinite measurements,
fixed threshold boundaries, missing/duplicate/failed samples, teardown failures,
churn false preservation, and incomplete independent-context denominators.
All source remains unbuilt and unrun. Existing benchmark instrumentation and
native ownership attestation still require same-candidate runtime verification.

Q attribution source now separates observed native operation, session,
infrastructure, fault-injection and teardown failures. Expected negative-control
errors remain successful negative controls. A timeout does not claim a WPT/shared
root cause; every attribution explicitly leaves confirmed cause unset. The
performance profile uses pooled warm actions divided by summed execution time,
not a median throughput that could conceal slow samples. Raw profile stdout,
stderr and resource artifacts are hashed in each retained attempt.

The parent agent added a native process lifecycle journal. The workflow executor
now holds its guard before startup verification until explicit Browser teardown.
RSS reconciliation uses native unique launch IDs, PID lifetimes, exact worker
binary identities, page leases and checked reap records. Verification/prewarm
processes contribute to aggregate footprint and are distinct from page-worker RSS
percentiles. Missing reap/binary/lease records, ambiguous PID reuse, missed page
lifetimes and backwards wall-clock samples fail accounting. Added regression
sources for those negative controls. Native lifecycle hooks and all accounting
remain unrun; `complete` is checked together with individual records, not trusted
as a sole proof.

`qualify-workflow-q.py` now reopens hash-bound raw profile/workflow artifacts,
reconciles the frozen workflow denominator again, checks same-candidate identity,
and combines profile and independent-context resource gates. It retains explicit
remaining full Core, eight-framework, signing/build-identity and manual/security
gates and does not issue final certification.

With parent approval, both existing native page/group benchmark examples now
hold the lifecycle guard, report runtime attestation, explicitly retire context
workers and drop the Browser, and retain lifecycle output. The group benchmark
retains close errors instead of ignoring them. Runtime reporting occurs after
timed work so it does not prewarm the cold-launch measurement. Profile validation
requires each raw run's sandbox/worker identity and checked process reap.

Source review resolved the suspected verification-helper gap: macOS static and
running signature verification use in-process Security.framework/csops APIs, and
RSS uses proc_pidinfo. No codesign/security helper subprocess is present there.
The parent is replacing the actual non-macOS /bin/ps RSS helper with a direct
/proc read. Unknown descendants still fail ownership reconciliation; the
Python sampler process is outside the native executor descendant tree.

The existing group example did contain its own `/bin/ps` and `pgrep` calls.
Those are now replaced by active native lifecycle PID records and the public
native RSS reader, preserving the measurement definition while removing guessed
process ownership and helper children. The profile runner retains additional
lifecycle-reconciled resource accounting where available. Fast process lifetimes
missed by periodic sampling remain explicitly incomplete rather than invented.
Warm profiles now record completed action count separately from requested count;
qualification requires the exact full count and checks the native placement mode.

## Full-plan execution barrier and combined provenance — September 16

Generated shared `scripts/generation_gate.py`, preserving the existing schema-1
all-51-parent-requirement gate and exact plan/implementation/regression hashes.
The official WPT wrapper delegates to it. Full OIL execution and performance
profiles now require `--generation-gate` and validate it before creating evidence
or invoking source hashing/native processes. Full OIL rechecks the source gate and
candidate through its probe before each application attempt; profiles recheck the
gate and frozen candidate before each benchmark. Historical runner/evidence formats
remain readable and are not promoted to newly bound qualification.

Generated `completion_provenance.py` and required `--candidate` for the full OIL
entrypoint. The shared candidate is the same schema-1 artifact record consumed by
WPT, extended with complete workflow/engine/performance identities. Required
artifacts include runtime, driver, workflow executor, signed worker, source
snapshot, engine archive/bindings/ICU/startup snapshot/external references,
unsigned engine worker, schema-4 engine profile, signed worker manifest, signature
evidence, fixture/workflow contracts, framework build and three benchmark binaries.
The actual current schema-4 V8 validator checks exact patches/options/artifacts
and the full profile. Signed worker manifest fields must agree with source,
unsigned worker, archive and profile hashes. The unsigned build and signed
executable are deliberately distinct artifact roles.

The full OIL producer now records `candidate_manifest_sha256` on every attempt
and its summary. The existing workflow validator requires that field on every
attempt whenever the candidate declares it; legacy records cannot be relabelled.
The profile producer binds benchmark executables to their exact manifest paths
and digests. Q component output propagates the same identity and rejects a
profile candidate-file mismatch.

Generated `join-completion-provenance.py` to write a new exclusive evidence index
for Core, workflows, frameworks, Q, security, architecture and reliability. Every
component requires candidate binding in the report itself, exact report/raw-input/
producer-source digests, and no missing or duplicate inputs. It preserves all
component reports and **always returns `qualified: false`**: a provenance join is
not native signature verification or proof that component assertions passed.

Regression sources cover barriers preceding side effects, source/candidate drift
before an attempt, missing engine/signature artifacts, mandatory profile
validation, unselected executable paths, missing/changed component inputs,
foreign-candidate reports, and manifest-bound attempts rejecting legacy evidence.
No build, test, formatter, generator, install, signing or native execution ran.

### Remaining Q-05/Q-06 scope

The generated barrier parity and bounded identity join can be tracked separately
from full final qualification. Framework/security/architecture/reliability
producers still need candidate binding at their original execution/assessment
boundary and their complete native/raw domain validators must be joined before a
final combined acceptance command can certify completion. The new provenance
index deliberately rejects older reports lacking the manifest identity. Actual
signed candidate construction, infrastructure, runs, all manual/security evidence
and the original booking-timeout mechanism closure remain pending. No package
completion or conformance result changed.

## Final qualification producer custody and validator integration — September 16

Generated `completion_execution.py` and `run-completion-component.py`. The four
framework/security/architecture/reliability producer kinds invoke fixed existing
runners, persist candidate-bound preflight before launch, retain raw stdout/stderr
and every produced file, and record process exit/group cleanup. Missing/changed
artifacts, substituted commands, source drift and any candidate mutation remain
failures. Historical reports cannot be imported into these original-run receipts.

Generated `completion_qualification.py` / `qualify-completion.py` to consume a
qualification plan frozen as a candidate artifact. It verifies original-run
receipts, recomputes complete workflow expectations from recipes, revalidates
framework raw OIL frames against the original tasks, invokes the full-manifest
Core reconciler and Q resource/workflow validator, and requires the strict security
raw-artifact validator without historical bypass. Architecture cutover and release
attestations must both be closed. Final acceptance also requires an independently
reviewed, source-bound diagnosis of the original booking timeout with the same
named regression failing before the fix and passing on the exact final candidate.
Missing evidence remains a final failure; fresh green repeats alone do not close
that unexplained failure. Only the complete validator join may emit qualified=true.

Protocol-tooling reconciliation found stale worker11 consumers and a security v2
schema pinned to worker6. New `native_protocol_versions.py` reads the three
independent authoritative constants (worker, context-service, agent IPC), rejecting
missing/duplicate declarations. Build/signing metadata and security producer/raw
validator now use the actual worker constant. Security output is versioned to
schema3; schema2 remains unchanged and is accepted only by explicit historical
validation. Context/agent versions are never substituted for the page-worker wire.

Regression sources cover original-run custody and retained producer failures,
command/inventory tampering, all required final domains, failing actual validator
commands, partial Core, foreign Q candidate, pending architecture attestation,
unconfirmed booking diagnosis and distinct protocol constants. No generated code,
test, validator, build, signing step or command producer has been executed.

Operational limitations remain explicit: the existing security producer performs
its own build/sign steps, so any changed artifact makes the original-run receipt
fail candidate integrity; its artifacts must reproduce the frozen candidate.
The current architecture release attestations and historical shadow evidence must
be replaced/qualified through their existing real requirements. This implementation
records their failures rather than manufacturing those missing inputs.

The booking mechanism closure also has a fixed `booking-regression` producer:
its prebuilt test executable is a candidate artifact, its exact test name is
frozen in the qualification plan, and the final diagnosis must reference that
original-run receipt's actual stdout with exactly one passing named test. A
manually added candidate hash on a loose regression log cannot satisfy the gate.

Final qualification now also inventories original workflow/profile/component and
Core inputs before validation, records their hashes in the result, and requires an
identical inventory afterward. A successful validator followed by changed raw
input cannot produce combined acceptance. Added an explicit drift regression.
