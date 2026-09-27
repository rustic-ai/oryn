# Consolidated validation migration

The user approved the September 27 consolidation plan. This runbook adopts its development process while the production pipeline is migrated. The implementation is not yet a complete replacement for every legacy gate. Missing semantic gates fail closed; they must never be removed just to launch WPT.

## Storage and identities

Configure one local data root (currently the sibling `oryn-data`). New typed documents contain store-relative or bundle-relative paths, never a user's home directory. Source lives in Git. Compiler caches live under `cache/cargo/<configuration>`; pinned engines, environments, and datasets are separate. Sealed builds live under `builds/<build-id>`, original observations under `runs/<run-id>`, and mutable progress under `status/`.

Build, run, result, and release schemas live in `tools/validation/oryn_validation/manifests.py`. A manifest schema check validates structure only. Production admission must also recompute source, engine, profile, signature, fixture, prerequisite, canary, placement, and review semantics. Saved `passed` booleans cannot stand in for those validators.

Only declared immutable inputs and explicit environment observations affect admission. Historical references are opaque citations. Output destinations and mutable status are not executable dependencies. Directory inputs include exact membership, including empty directories; unexpected files, symlinks, escapes, and same-size content changes are rejected. Admission and finalization hash the closure; in-run checkpoints preserve lightweight custody.

## Command entry point

Run `python3 scripts/oryn-validate.py --help` from the checkout. Commands emit JSON and return nonzero on a rejected operation.

| Command | Purpose |
| --- | --- |
| `manifest-check` | Check one build/run/result/release schema, without execution |
| `input-snapshot`, `inputs-check` | Create or verify explicit immutable input declarations |
| `source-audit` | Check Git identity and the independently reviewed effective-input inventory |
| `source-export`, `source-info`, `generation-prepare` | Export committed source with Git object proof and prepare its exact generation declaration for independent review |
| `dataset-export` | Export the pinned WPT commit with reviewed, explicit normalization of documentation links |
| `environment-prepare`, `environment-verify` | Stage the exact offline Python payload and check its declared system dependencies |
| `engine-inputs-stage` | Copy the declared engine/compiler inputs into the data root |
| `engine-profile-refresh`, `typed-runtime-prepare`, `typed-profile-check` | Bind and verify typed metadata; these operations do not prove a native build happened |
| `build-plan` | Produce the canonical build command/cache plan; does not launch a build |
| `build-run`, `build-assemble`, `bundle-seal` | Execute the owned canonical snapshot/parent/driver/worker build, join its original evidence, and seal its signed output |
| `native-reuse-check` | Prove unchanged native input, configuration, artifact, and signature bindings |
| `controls-prepare`, `prerequisites-run` | Prepare upstream control manifests or collect explicit runtime/native/topology prerequisites |
| `wpt-inspect` | Inspect migrated WPT checks and report remaining gate ports |
| `wpt-run` | Admit and execute a fresh original only when all production gates exist and pass |
| `wpt-reconcile` | Join one original's starts, ends, unique URLs, outcomes, and process evidence |
| `cleanup-preview`, `cleanup-execute` | Preview and execute unchanged, eligible compiler-cache eviction |

Use explicit `--store-root` and `--bundle-root`. Do not infer them from a historical manifest path. A build plan is an inspection result and is not build authorization or production admission. It still requires a current source-generation gate, complete configuration, free-space check, and resource ownership. The canonical build scripts retain their signing and runtime assertions.

The WPT adapter accepts typed `kind=run` documents through the new package and retains the legacy adapter path for old evidence. Upstream wptrunner still owns test semantics; the new code does not substitute a browser loop. A current standard-block original selects all 15,985 automated URLs with six workers and records the 752 assessment obligations separately. Do not stitch partial originals, replace failures, or loosen deadlines to manufacture completeness.

Execute official WPT from the committed runner export named in its manifest. The everyday checkout can prepare that export; its mutable source cannot impersonate the frozen runner. Follow [the first-build recipe](FIRST-BUILD.md) for the coordinated build. Every accepted build joins original process logs, source/generation review, declared compiler inputs, and all seven products, including the fresh startup snapshot. A metadata conversion or copied historical binary cannot satisfy that join.

The [Python environment recipe](ENVIRONMENTS.md) preserves exact installed dependency bytes without requiring the old candidate at runtime. Declare its system libraries explicitly and recheck them at admission and finalization. Metadata checkpoints still enumerate every declared member; an experimental filesystem-event shortcut was rejected because it could miss an immediate rename and restoration.

## Process ownership and failures

`processes.run_owned` wraps `scripts/completion_process_supervisor.py`. Each fresh attempt records its argv, owner, child birth identity, raw output, process outcome, cleanup failures, and terminal receipt. Native process inspection requires normal macOS process visibility; an environment that denies observation yields an incomplete custody result, not assumed success.

`ResourceLease` locks named resources and records durable ownership in `retention.json` plus `status/resources/`. Reserve all required cache, signing, port, focus, and performance resources before launch. Active or unsettled leases block conflicting work and cleanup. A stale PID does not settle custody. Investigate missing terminal receipts using kernel birth identity and retained descendants before explicitly recovering an unsettled lease; there is no automatic stale-lease bypass.

The process result, test observations, evidence completeness, and release decision remain separate. A complete original may have failing tests. Missing starts/ends, duplicate URLs, input drift, failed teardown, or lost custody make it incomplete/invalid. Preserve the primary error when settlement itself fails.

## Retention

All index publishers and cache writers participate in the same retention lock and resource leases. The explicit index stores object classes, dependencies, pins, last use, and active/unsettled/terminal leases. Keep the active build and verified fallback pinned along with unresolved evidence dependencies.

The initial ordinary compiler-cache budget is 120 GiB. Cleanup previews exact eligible LRU paths, rechecks index and directory identity under lock, and records actual filesystem free-space change. A deleted-cache tombstone requires explicit re-registration before reuse. Cache cleanup does not delete builds, raw originals, assessments, source archives, or historical evidence. Archive and restoration verification precede later evidence retirement.

Before a large build, measure required build/staging demand and require an additional 50 GiB reserve. Do not create another large target directory to evade an active-cache conflict or a space shortage.

## Migration checkpoints and rollback

| Phase | Exit condition |
| --- | --- |
| M0–M1 | Baseline inventoried and preserved in Git, omitted effective inputs reconciled, schedules disabled |
| M2 | Strict typed contracts/CLI and negative tests pass |
| M3 | Reusable build/run workflow preserves every production gate and has no new candidate helper directory |
| M4 | A clean-source independent signed bundle has zero legacy runtime dependencies and verifies after relocation |
| M5 | Native/topology prerequisites, five canaries, six-worker placement, failure paths, and independent reviews pass |
| M6 | One full 15,985-URL original completes and is reviewed, preserving all failures and separate assessments |
| M7 | Archives restore correctly, current status/docs switch, legacy wrappers and live dependencies retire |

The recovery checkpoint is `96b7fd349baff7a5d8e4e19ba61132067762e7ae`; backup inventory and source patches are under `oryn-data/migration/baseline`. R27 source archives, signed artifacts, original receipts, and required dependencies remain untouched until M6 is complete. New source changes can invalidate legacy live-checkout comparisons; rollback requires the exact source/layout expected by its original manifests, not relabeling old binaries with a new commit.

During migration, current progress belongs to `oryn-data/status/consolidation.json`. Legacy `docs/oryn2/CURRENT_STATUS.md`, `docs/oryn2/current-status.json`, and dated repair directives describe historical runs and their old contracts. Do not rewrite hashed history to make it current. At M7, switch entry points only after restoration and legacy-root-unavailable verification.

Consolidation completion requires M0–M7, including the complete reviewed original. Browser delivery still has the other profiles and remaining qualification obligations. No overall completion percentage follows from scaffolding the new tooling.
