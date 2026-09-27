# Development workflow

Adopted during the user-authorized September 27 consolidation. This process governs new work; it does not alter the identity or interpretation of legacy evidence.

## One source project

The existing 17-member Cargo workspace is the product. Keep its package names, public interfaces, and feature contracts. Use a bounded branch and coherent commits for changes. The integration checkout remains `oryn-roadmap-next` on `codex/conformance-next` during migration; it shares Git storage with the `oryn` main worktree. Never delete the shared repository as obsolete candidate data.

The preservation commit `96b7fd349baff7a5d8e4e19ba61132067762e7ae` records the pre-consolidation implementation as it existed, including the reviewed dependency patches. It is a recovery point, not a qualified browser release. Do not rewrite that historical work into invented smaller commits.

## Change process

1. Identify the component, callers, features, generated inputs, and meaningful regression coverage. See [the component inventory](COMPONENTS.md).
2. Implement and run focused developer checks while iterating. Use `cargo test -p <package>` and the existing feature/test serialization rules where applicable. A development result is not official qualification.
3. Review the integrated batch, generated output, source ownership, and cross-feature effects. Commit before official validation. Account explicitly for ignored effective build inputs, especially the prepared V8 checkout; a clean top-level status alone is insufficient.
4. Reuse existing CI in `.github/workflows/ci-rust.yml`, JS/extension/E2E workflows, and canonical build/signing scripts. Preserve workspace/default/all-feature coverage and serialized V8 tests. The component inventory is a conservative aid, not a complete compiler dependency analyzer.
5. Produce one clean-source build, verify it, and seal its executable/runtime dependencies. Run acceptance against that identity without editing or re-signing it. Changes to runner/reporting code can reuse native output only after explicit native-input equivalence succeeds.
6. Record original outcomes, including failures, and review validity separately. Fixes produce commits and new build identities when needed; retries receive new run identities. Never combine fragments into a complete original.

Temporary Git worktrees are appropriate for genuine isolation, such as a frozen validation checkout. They share history and are removed after integration and process settlement. Ordinary changes must not create another permanent `acceptance17-rNN` tree or copy of the compiler cache.

## Ownership and generated sources

Keep scanner edits in `crates/oryn-scanner/src/scanner.js`; run `scripts/sync-scanner.sh` to update extension copies. WebIDL input and generator ownership remain under `oryn-webidl`; native build assets, startup snapshots, ICU, and V8 patches follow their canonical build contracts. Textual Rust `include!` files are part of their caller's compilation unit and must be reviewed with the caller.

Disjoint implementation/review lanes may work concurrently. One coordinator owns shared build caches, signing, fixed ports, native focus, and performance execution. Use the recorded resource lease, not a missing receipt or a remembered PID, to decide whether execution can begin. The existing descendant supervisor remains authoritative.

## Evidence and scope

Preserve all nine work packages and 51 parent requirements; three Core profiles with 15,985 automated URLs and 752 assessments per profile; 688 family/reliability originals plus 24 framework originals; 210 performance/churn samples; and the required security, architecture, native, fixture, media, manual, and independent reviews. Build success and regression success do not establish WPT conformance.

Follow [the migration runbook](../validation/CONSOLIDATION.md) for the current implementation limits and cutover gates. Schedules stay disabled. A future monitor must observe only unless the user separately authorizes continuation actions.
