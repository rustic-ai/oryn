# Development workflow

Adopted during the user-authorized September 27 consolidation. This process governs new work; it does not alter the identity or interpretation of legacy evidence.

## One source project

The existing 17-member Cargo workspace is the product. Keep its package names, public interfaces, and feature contracts. Use a bounded branch and coherent commits for changes. The integration checkout remains `oryn-roadmap-next` on `codex/conformance-next` during migration; it shares Git storage with the `oryn` main worktree. Never delete the shared repository as obsolete candidate data.

The preservation commit `96b7fd349baff7a5d8e4e19ba61132067762e7ae` records the pre-consolidation implementation as it existed, including the reviewed dependency patches. It is a recovery point, not a qualified browser release. Do not rewrite that historical work into invented smaller commits.

## Change process

### Local patch custody

Use the sibling workspace `.local-patches/<batch>/<lane>/` for unfinished
patches, source fragments, review notes and diagnostics. Do not stage development
work in `/tmp`, `/private/tmp` or `target`. This local, untracked directory is not
an external backup. Record the integration base commit, lane owner, affected
paths, SHA-256 hashes and state (`draft`, `reviewed`, `applied`, `verified`,
`committed`, `archived`). Stop a lane before changing its base.

Apply only reviewed patches to `oryn-roadmap-next`; preserve overlapping dirty
files first. Applying a patch is not completion. Retain it through regression
checks and a recorded integration commit. Archive the manifest and unresolved
evidence before deleting exact applied staging files. Never bulk-delete pending
patches, raw failures or recovery snapshots. Do not execute imported draft scripts
without review, and keep secrets out of source and patch inventories.

`scripts/preserve-development-work.py` is a one-time recovery helper for the
existing split checkout incident. It copies and verifies temporary regular files,
archives both checkout sources and preserves Git history without altering either
index. Its receipts are recovery evidence, not qualification. Ignored effective
build inputs and nested repositories require separate reconciliation.

1. Identify the component, callers, features, generated inputs, and meaningful regression coverage. See [the component inventory](COMPONENTS.md).
2. Implement and run focused developer checks while iterating. Use `cargo test -p <package>` and the existing feature/test serialization rules where applicable. A development result is not official qualification.
3. Review the integrated batch, generated output, source ownership, and cross-feature effects. Commit before official validation. Account explicitly for ignored effective build inputs, especially the prepared V8 checkout; a clean top-level status alone is insufficient.
4. Reuse existing CI in `.github/workflows/ci-rust.yml`, JS/extension/E2E workflows, and canonical build/signing scripts. Preserve workspace/default/all-feature coverage and serialized V8 tests. The component inventory is a conservative aid, not a complete compiler dependency analyzer.
5. Produce one clean-source build, verify it, and seal its executable/runtime dependencies. Run acceptance against that identity without editing or re-signing it. For official qualification, runner/reporting changes require explicit native-input equivalence before reusing native output. Ordinary developer runs reuse the existing binary directly; rebuild when native code or its compiled inputs change.
6. Record original outcomes, including failures, and review validity separately. Fixes produce commits and new build identities when needed; retries receive new run identities. Never combine fragments into a complete original.

Temporary Git worktrees are appropriate for genuine isolation, such as a frozen validation checkout. They share history and are removed after integration and process settlement. Ordinary changes must not create another permanent `acceptance17-rNN` tree or copy of the compiler cache.

## Everyday WPT and profiling

### Fresh-worker integration requirement

When Rust, native support libraries, generated bindings, startup snapshots or
other worker inputs change, rebuild and sign the page worker before the complete
native integration suite. Build the test targets and worker from the same frozen
source and configuration. Set `ORYN_PAGE_WORKER` explicitly to the new signed
bundle, record its executable SHA-256 and signature verification, and recheck
the executable identity after the suite. Never silently use the fallback worker.
If rebuilding/signing is blocked, report the blocker; a retained-worker run is
only a diagnostic and cannot verify the changed batch. Keep its original evidence.

The coordinator owns the compiler-cache and signing leases through build and
test settlement. Do not edit native inputs between building the worker and
running integration tests. Reuse is appropriate only for unchanged worker
inputs (for example Python-only edits), with that fact explicitly recorded.

Use `python3 scripts/oryn-validate.py wpt-dev --help` for selected developer tests.
Supply an existing run manifest or a prior `developer-config.json` as a path/configuration template, its store and
bundle roots, an include file, and a fresh output under `<store>/development/`.
The first run extracts a compact configuration; reuse that file to avoid reloading the legacy dataset inventory. Run with the selected WPT Python interpreter and its `PYTHONHOME`. The command
records the actual checkout (including uncommitted changes), existing native
build identity, exact selected URLs, raw results and elapsed phases.

No commit, source export, generation gate, independent review receipt, signing,
native equivalence proof, or complete dependency audit is a prerequisite for
this developer loop. Run from the working checkout. Build native code when it
changes; Python runner changes do not require rebuilding or resealing Rust.
`developer_protocol_smoke` uses the same lightweight default for window profiling.

Expensive operations are explicit:

| Operation | Everyday development | Qualification / investigation |
| --- | --- | --- |
| Full dataset/environment/source hashing | Omitted | `--audit-inputs` hashes declared inputs once before the run |
| Production build/signature checks | Reuse the selected executable | `--audit-inputs` verifies the retained native build once |
| Repeated child/adapter/shutdown input audits | Removed from developer paths | Existing official immutable-run checks retained |
| Source export, generation gate and native reuse proof | Not required | Release/qualification preparation only |
| `wpt-inspect`, `wpt-run` | Do not use as a developer startup step | Expensive qualification commands |
| Per-experiment helper packages and review manifests | Retired | Preserve historical evidence; no new copies |

The opt-in input audit covers the retained template and native build, not the edited checkout. It is a pre-run observation, not proof of immutability during
execution or release qualification. Lightweight runs still check executable
paths, worker and fixture configuration, real TLS behavior, exclusive port/focus
ownership, process cleanup and exact outcome completeness. They retain failures.
They do not certify unchanged source/dependency trees. Raw test outcomes and
qualification are separate facts.

## Ownership and generated sources

Keep scanner edits in `crates/oryn-scanner/src/scanner.js`; run `scripts/sync-scanner.sh` to update extension copies. WebIDL input and generator ownership remain under `oryn-webidl`; native build assets, startup snapshots, ICU, and V8 patches follow their canonical build contracts. Textual Rust `include!` files are part of their caller's compilation unit and must be reviewed with the caller.

Disjoint implementation/review lanes may work concurrently. One coordinator owns shared build caches, signing, fixed ports, native focus, and performance execution. Use the recorded resource lease, not a missing receipt or a remembered PID, to decide whether execution can begin. The existing descendant supervisor remains authoritative.

## Evidence and scope

Preserve all nine work packages and 51 parent requirements; three Core profiles with 15,985 automated URLs and 752 assessments per profile; 688 family/reliability originals plus 24 framework originals; 210 performance/churn samples; and the required security, architecture, native, fixture, media, manual, and independent reviews. Build success and regression success do not establish WPT conformance.

Follow [the migration runbook](../validation/CONSOLIDATION.md) for the current implementation limits and cutover gates. Schedules stay disabled. A future monitor must observe only unless the user separately authorizes continuation actions.
