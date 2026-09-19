# F/Q acceptance gate source reconciliation

September 18, 2026. Source inspection and generation only. No validators, tests,
generators, native services or qualification runs executed. Every parent row
below retains pending execution acceptance.

## Frozen population and replay

`workflow_contract.py` pins the original behavior contract SHA-256
`3fb9e1173b54ae60828c73f86af6f80dd31956c1c4ccda296bf970af68a0a556`.
It contains research 5, forms 6, shopping 6, SaaS 7 and booking 5 cells: **29**.
Ten cells are recovery-sensitive. Fixed planning gives 87 serial observations,
261 mixed-family observations across 1/5/20 contexts, 300 recovery repetitions
and 40 fault-injection observations: **688 original planned attempts**. Retries
are additional observations, never replacements for failed originals.

`workflow_input_lock.for_candidate` validates either the original immutable
contract or the separate complete current input lock with the same immutable
behavior population. `completion_qualification.workflow` independently derives
the exact plan and expected projections using all research and family recipes.
`workflow_evidence.reconcile` rejects missing/duplicate/unplanned originals,
candidate mismatch, failed or missing result fields, timeout and cleanup failure.
Every candidate-bound row requires `workflow_raw_evidence.validate`, which
reconstructs actual recipes/actions, native run identity, fault/commit traces,
sealed database outcomes, isolation probes and resource cleanup from retained
raw bytes. Duplicate JSON keys, nonfinite values and malformed objects are
rejected at its typed boundary. The earlier lane document contains the exact
29-cell production fixture/oracle mapping.

The eight framework names remain React, Preact, Vue, Angular, Svelte, Next, Nuxt
and SvelteKit. `run-oil-integrated-outcomes.validate_summary` requires the exact
Counter of three samples for each name (**24 originals**), fixed build/parent/
worker/source identities, successful explicit process outcomes and descendant
cleanup. `completion_qualification.framework` replays each retained OIL output
through the actual framework validator and checks exact task bytes, rather than
trusting summary pass counts.

## Parent-row disposition

| Parent row | Actual source / evidence owner | Execution still required |
| --- | --- | --- |
| F-01 | `workflow_native_adapter.py`, `workflow_family_recipes.py`, existing family receivers/runners; `workflow_raw_evidence.py` durable replay | All genuine native OIL paths, reopened SQLite outcomes and negative controls on one final candidate |
| F-02 | Frozen 29-cell contract and versioned frontend fixtures; `run-oil-framework-outcomes.py` and exact 8-framework summary validation | All frontend dimensions, controlled frames, portals, hydration/chunks and 24 framework originals |
| F-03 | `completion_provenance.load_candidate`, complete source archive and engine/signature validation; producer pre/post checks | Fresh matching source/binary/signature runs; historical persisted-SaaS mismatch is not replaced by an assertion |
| F-04 | Independently derived expected oracles, raw database/authority/action replay, unique original attempt IDs and retained retries | Actual final facts/orders/reservations and all original failures, retries and cleanup |
| Q-01 | `completion_qualification.booking_resolution`, exact original result and six-file diagnostic trace; original before/after regression receipts | Confirmed independently reviewed cause, bounded mixed reproduction and same named regression failing before the fix and passing on final candidate |
| Q-02 | Fault decoration, commit gates/recovery retry tables, raw native process/ownership traces and supplements | Delayed response, worker failure, duplicate delivery and cancellation controls, including no repeated writes and checked reaping |
| Q-03 | `workflow_qualification` fixed thresholds and 210 profile attempts; `qualify-workflow-q.py` raw output/OS/native accounting replay | Cold P50 <=250 ms; warm >=300 actions/s; binding P50 <=50 ms; worker RSS P50 <=64 MiB/P95 <=160 MiB; integrated 1/5/20 contexts and aggregate accounting |
| Q-04 | Immutable sampling expansion and exact-original/retry reconciliation | All 688 original workflow attempts, additional retained retries and required group/churn samples |
| Q-05 | Source snapshot, full candidate artifact identities, workflow input lock, original-run preflights/receipts, feature/cookie profile evidence | Deliberate final lock/snapshot/build/signature freeze after complete generation; matching complete Core/family runs |
| Q-06 | `completion_qualification.qualify`, all domain validators and final original-input comparison; `wpt_execution_evidence.py` profile/raw-report replay | Every Core ID/assertion, manual/source dispositions, security, native prerequisites/topology/media, F/Q originals, immutable final input join |

## Confirmed source repairs in this review

1. **Profile consumer identity gap:** final Q replay now compares the entire
   `candidate-inputs.json` identity and all page/group/churn/worker executable
   digests to the final candidate. Every raw native worker digest must match that
   candidate. Producer-only checking and syntactically valid hashes are
   insufficient. Missing profile `errors` is rejected instead of defaulting to
   successful empty errors.
2. **Unreceipted booking before log:** a text file containing `FAILED` is no
   longer adequate. Diagnosis requires `regression_before_candidate` and
   `regression_before_receipt`, the preserved prior source archive and artifact
   inventory, exact named executable command and original stdout. The prior
   receipt must record a positive nonzero exit, no timeout/transport error,
   complete cleanup and only the expected producer failure; signal termination,
   candidate drift, successful exit and substituted receipts fail. Final passing
   regression remains bound to the final candidate and original output. Every
   claimed changed source must occur with its exact bytes in the final source
   archive and differ from (or be absent in) the prior source archive.
3. **Original booking trace omission:** `original_trace` now requires all six
   preserved task/stdout/stderr/final-database/oracle/preliminary-result byte
   identities, in addition to the already pinned original result. Original stdout
   is empty and remains valid as historical evidence; it does not become proof of
   command-level causality. A fresh green repeat cannot replace any original.
4. **Candidate hash cycle:** the frozen qualification plan now holds an absolute
   diagnosis output **path**, as it already does for receipt paths. After
   execution, the diagnosis is hash-pinned in the final original-input inventory
   and checked again immediately before use and at final reconciliation. This
   permits candidate -> fixed plan path, then diagnosis -> candidate without an
   impossible candidate -> diagnosis hash -> candidate cycle. Mutable/symlinked
   paths and post-inventory substitution are rejected.
5. **Impossible final component equality:** `native_media` was executed but
   omitted from the exact required component set. It is now required explicitly,
   allowing a complete set to qualify while retaining missing-media rejection.
6. Original component receipt replay now checks its preflight source snapshot
   identity against the bound candidate, including the prior regression receipt.

Generated regressions in `test_workflow_qualification.py`,
`test_completion_execution.py` and `test_completion_qualification.py` cover exact
profile identities, missing error custody, genuine supervised failing receipt,
timeout/signal/drift rejection, a cycle-free valid before/after chain, original
trace substitution, receipt substitution and diagnosis mutation. Existing full
qualification fixture checks require all eleven components, including media.
These are regression **sources**, not passing test evidence.

## Required freeze order and evidence limits

After the generation barrier, retain the prior regression executable/source
snapshot and original failing supervised receipt. Freeze the final qualification
plan with output paths and the exact regression name, then freeze the complete
final candidate. Execute its component producers. Write the independently
reviewed diagnosis referencing both original regression receipts, the original
historical trace and the final candidate. Finally run qualification, which pins
all completed inputs and rejects changes through the end of validation.

The original booking failure remains exit 1 at 30.12 seconds with
`timed_out:false`; missing command-level trace still prevents a confirmed cause.
The gates verify producer custody and exact artifacts; human independent causal
review remains a required assessment, not something a hash or successful retry
can prove. No cause, measurement, platform outcome or acceptance completion was
manufactured in this source phase.
