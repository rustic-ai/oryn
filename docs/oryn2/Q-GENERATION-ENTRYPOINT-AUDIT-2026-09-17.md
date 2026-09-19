# Q-05.c/f execution admission source audit

Source inspection only. No script, build, test, generator, fixture server or
qualification process was executed for this review. Parent Q acceptance remains
open; declarations are not generated automatically from checked child rows.

## Shared contract

`generation_gate.py` still requires all 51 parent requirements, the current full
plan hash, and nonempty implementation/regression source records with matching
hashes. `GenerationLease` freezes declaration bytes before candidate/setup work,
validates both before and after each check, and permanently rejects an observed
failure. Child environments carry the declaration path and original digest;
there is no boolean bypass. Nested process admission retains failure even if the
original file is restored after the first failed check.

## Execution caller inventory

| Entry / helper | Production admission and custody |
| --- | --- |
| `run-official-wpt.py` | Calls only `wpt_official_workflow.prepare/execute`; both freeze admission before candidate inspection. Preparation rechecks before output, each actual upstream manifest build and workflow lock. Execution rechecks before output and prelaunch/postrun custody. |
| `wpt-upstream-entry.py` | Requires explicit declaration, checks inherited path/digest, revalidates after pinned checkout checks and before manifest generation, protocol discovery/execution or actual upstream runner; checks completion too. |
| `run-oil-full-contract.py` | Freezes gate before candidate/contract binding; exports exact child admission; existing per-attempt candidate/source probes retain the complete denominator and stop later attempts after observed drift. |
| `workflow_native_adapter.py` / forms `Oracle` | Check inherited admission before first output and immediately before process creation. Family fixtures use this Oracle; native wave/supplement execution uses the same adapter helper. Cleanup has no new admission requirement. |
| `run-workflow-qualification-profiles.py` | Frozen declaration before candidate setup; source/binary probes before output and each attempt; process helper rechecks after output opening, before Popen. |
| `qualify-completion.py` / `completion_qualification.qualify` | Freeze before candidate inspection and before output; every component validator checks the same lease before/after; child validation commands receive its exact environment. |
| `run-completion-component.py` / `completion_execution.produce` | Freeze before candidate inspection, recheck before first output and child launch, propagate declaration to all child commands, retain existing postrun integrity receipts. Fixed kinds cover framework, reliability, security, architecture, booking regression, topology, native prerequisites and media. |
| `run-oil-framework-outcomes.py` | Optional ungated diagnostic branch removed. Explicit declaration or exact inherited admission is required; fixture/runtime/case checks preserve original lease and failure latch. |
| `run-wpt-native-controls.py` / `wpt_native_controls` | Freeze before setup/output; exact inherited admission before TLS fixture binds and every native/reference driver launch; bootstrap protocol child receives frozen environment. |
| `run-wpt-topology-probe.py` | Freeze before candidate/setup; check before first output and each network route, retaining failed routes and final validation. |
| `run-native-media-qualification.py` / `native_media_qualification.qualify` | Freeze before candidate inspection; check before output and native test launch, propagate original declaration, retain postrun verification. |
| Legacy `run-full-wpt-baseline.py`, `run-upstream-wpt.py` | Diagnostic status unchanged; standalone entry and every native/direct fixture execution now require inherited full generation admission. They cannot bypass the phase restriction. |
| Legacy forms/booking/research/shopping/saas/integrated OIL CLIs | Require inherited declaration before output/setup; process helpers check at native launch. Reports remain historical diagnostic reports, not full-contract qualification. |
| `build_app_compatibility_framework_corpus.py` | Direct CLI and command helper require inherited admission before output/build and each process creation. |
| Security component shell tree | Guarded security/layout suites, software wrappers, native benchmarks, rejection checks, sandbox-worker/WASM builds and executing validators. Direct cargo/native runtime/benchmark commands recheck admission. Pure manifest/result/hash readers remain read-only utilities. |
| V8/startup/platform build helpers | Direct invocation requires the same frozen declaration before build side effects. |

## Removed bypasses and retained distinctions

Historical diagnostics no longer mean permission to execute during generation.
They remain separate from candidate acceptance. Read-only plan/manifest/source
inspection is still possible without launching applications or fixture services.
The general shell and native binaries are not security sandboxes: this barrier is
applied to the project's qualification/build workflows, not a claim to prevent a
user from independently launching arbitrary commands.

Fresh workflow input locks now include shell children, media launch/producer,
framework build helper, and legacy WPT execution scripts. Historical lock files
are unchanged; candidate generation must produce the fresh lock after source
closure.

## Regression sources and pending qualification

New `test_generation_lease.py` covers valid replacement during validation,
restoration after revocation, child digest mismatch, first-failure retry, and
helper rejection before output/process creation. Official preparation regression
replaces the gate during candidate inspection and requires no output or manifest
process. Existing native cleanup tests explicitly mock admission so their
process-cleanup assertions remain independent. Earlier declaration/candidate
replacement and complete-denominator regression sources are retained.

These are generated source contracts. Python/shell execution validation,
all regression execution, signed candidate identity, platform qualification and
all 51 requirement acceptance gates are not established by this document.
