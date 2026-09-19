# Required Core profiles and whole-run custody

Status: production source and regression sources generated; execution remains subject to the complete repair39 source/review gate. This document does not supply a production fixture, registry, scope approval, disposition decision, candidate or pass ledger.

## Scope and meaning

The independently reviewed scope choice is [CORE-PROFILE-SCOPE-2026-09-18.md](CORE-PROFILE-SCOPE-2026-09-18.md): standard/block primary, standard/allow, and legacy/block. Each profile retains all **16,737 Core entries**, comprising **15,985 automatic originals and 752 manual/visual assessments**. Both cookie and feature pairs remain independently required. Profile IDs are declared by the reviewed registry; the validator does not generate a fourth combination or accept a union of passing URLs.

Non-Core producers currently execute native standard/block defaults. Their ten component claims must name that primary profile. Passing another Core profile does not extend those claims. Architecture independently derives platform scope from the same registry and consumes successfully validated original security evidence after security validation.

## Frozen registry and cycle-free review

Candidate artifacts include `core_profile_registry`, `core_disposition_policy`, the primary `fixture_contract`, and `qualification_plan`. The registry records every required fixture file and its SHA-256, platform, processor, entire-run native execution policy, exact required ID set, both invariant-environment policy pairs, pinned WPT revision/manifest/policy/coverage and the 16,737 denominator.

`scope_review.requirement_sources` binds completion, cookie, feature, executor and scope-choice source files. The review evidence is exactly `{schema_version:1,status:"approved",author,reviewer,registry_body_sha256}`. The body digest is SHA-256 of UTF-8 JSON with sorted keys and compact separators, excluding only `scope_review.evidence`; all remaining selection semantics and source references remain included. Author and reviewer must be distinct. The outer registry hashes the review evidence. Later revisions retain the complete prior-registry chain and its transitive evidence. No production review is synthesized by the tooling.

JSON schemas describe structure; `scripts/core_profile_contract.py` is the semantic validator. It checks actual fixture configuration and hashes, invariant pair environments, canonical paths, primary execution semantics, independent review and recursive evidence custody. See:

- `benchmarks/schema/core-profile-registry-v1.schema.json`
- `benchmarks/schema/core-run-envelope-v2.schema.json`
- `benchmarks/schema/completion-qualification-plan-v2.schema.json`
- `benchmarks/schema/core-profile-aggregate-v2.schema.json`

## One immutable envelope per original or retry

After candidate freeze, create a schema2 run envelope using its actual path and hash, selected registry fixture/hash, and distinct `run_id`, `attempt_id` and three `collection_ids`. It declares canonical, nonoverlapping future `prepared` and `outputs.{canaries,placement,core}` directories. A retry receives new run/attempt/collection IDs and retains the original `original_run_id`. One profile's report list must contain exactly one designated original and every retained retry; each automatic report independently includes all 15,985 automatic URLs. Raw failures stay failures even when an approved disposition satisfies a separate obligation.

The producer binds ten authoritative identity fields into workflow lock (except the per-collection ID), preflight, execution record and `run_info` (prefixed `oryn_`): `candidate_sha256`, `registry_sha256`, `profile_id`, `profile_sha256`, `envelope_sha256`, `run_id`, `collection_id`, `attempt_id`, `original_run_id`, `platform`. **`profile_sha256` means the exact fixture file hash**, independent of feature/cookie/environment hashes. Actual report OS/processor must match the declaration.

Each original profile has real canary and shared-placement collections. Core launch recomputes their actual validators after native-prerequisite/topology admission and repeats custody checks before and after execution. A retry uses its designated original profile's validated controls, with candidate/registry/fixture/platform and original-run identity checked. Controls from another profile or original are rejected. Upstream remains one process, one repeat/rerun, no automatic retries and no per-URL policy switches. Fixed fixture ports serialize official collections.

## Final consumers and dispositions

`load_profile_context(candidate_path, profile_id, report_paths)` verifies every original execution and report before returning the ten-field identity, `reports` mapping by report hash, original report hash, registry/fixture/policy paths, coverage hash, complete `profiles` mapping and asserted input records. It captures report and execution hashes before verification, rechecks them afterward and rechecks the candidate. It does not choose outcomes.

The strict reconciler accepts `--candidate`, `--candidate-sha256`, `--profile-id`, `--disposition-policy`, `--disposition-applications` and the existing pinned inventory, reports, reviews, assessments and attribution arguments. `wpt_reviewed_dispositions.py` separately validates source/assertion-bound decisions and per-attempt applications. All original/retry identities and raw attempts remain in the per-profile ledger. Distinct totals are raw passes, qualified passes, reviewed dispositions and open obligations.

`qualify-completion.py` requires a schema2 plan for current completion. It runs the real reconciler for every exact required registry member and rechecks each complete 16,737-ID ledger, counts, summary and original/retry identity. It emits `core-profiles-summary.json`; `single_profile_conformance` is derived only from the primary profile's 15,985 raw automatic passes and zero reviewed dispositions. The separate required-profile completion result may include valid reviewed dispositions, without relabeling them as raw conformance.

Recursive input inventory retains asserted hashes for registry history, review/policy/decision/application evidence, fixture TLS, checkout-relative `source_files`, assessment packet sources/helpers, envelopes, locks and every original raw file. Final inventory detects mutation, additions, removals and symlinks. Generated per-profile summaries/ledgers and the aggregate are captured when created and rechecked after later Q/security/architecture validators; later evidence collection cannot silently adopt changed generated results.

Historical schema1 reproduction requires explicit `--allow-historical` at both the final qualifier and reconciler. It cannot consume a registry candidate and never sets `current_plan_complete`. Current profile execution cannot downgrade to schema1 custody.

## Execution prerequisites

Materialize actual owned fixture routing/TLS/native-control evidence and an independently reviewed registry/policy after the scope decision. Use the qualification dispatch and release preparation runbooks for candidate freeze and commands. No production values are inferred from example fixture JSON. Security and architecture prebuilt artifact maps must be merged completely before freezing the candidate; immutable execution cannot build or sign candidate bytes. Native/UI prerequisites and fixed-port fixture lifecycle still apply.

Regression sources are listed with exact callable commands in `artifacts/completion-validation-2026-09-18/profile-envelope-implementation-delivery-2026-09-18.md`. They have not been executed during generation.
