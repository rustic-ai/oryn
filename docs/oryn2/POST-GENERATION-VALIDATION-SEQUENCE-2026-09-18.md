# Complete-plan validation sequence after source closure

Status: preparation only. No command below was executed for this document. The global source gate remains open while final implementation and regression sources are integrated.

## Source declaration and repair phase

1. Reconcile all 51 parents using the evidence map and final caller/disposition reports. Resolve actual missing source, integration and regression requirements. Keep execution and independent-assessment rows pending; they are not a reason to regenerate completed algorithms.
2. Author the exact generation declaration bound to the current plan and reviewed implementation/test-source hashes. Preserve the draft separately. Only after honest global source closure may build, generator, formatter or test execution begin.
3. Build the complete implementation and repair compiler/generator errors. The native crate has `default = []`: an unqualified workspace build/test cannot verify its V8 implementation. Include the native V8 and in-process probe feature configuration, and the production worker targets.
4. Run required formatting/linting and regression checks. `scripts/run-tests.sh` may format source and start its harness; it is not a read-only qualification command. Any resulting source change invalidates earlier hash-bound admission. Reconcile and refresh the declaration before subsequent admitted execution; never alter an active lease or reuse evidence from a different source candidate.
5. Run the native regression configuration explicitly, including `cargo test -p oryn-native --features v8-host,in-process-probe`, alongside the required workspace checks and Python/JS qualification-source tests. Ignored pinned-fixture and native-platform tests require their actual prerequisites and explicit scheduling; ordinary test success does not imply they ran.

The commands and features above identify coverage that would otherwise be missed. They are not permission to bypass the repository's V8/toolchain, signing, environment or generation-admission requirements.

## Final candidate and complete qualification

After repair, commit/freeze the clean reproducible source candidate and build/sign fresh compatible native artifacts. Current authored protocol versions are worker **17**, context **50**, agent **19**; an old signed binary or manifest cannot be relabeled. Retain exact source/build/toolchain identities and all original reports.

Use the existing completion qualification orchestration and its frozen plan/candidate/generation inputs for:

- Every pinned Core entry and declared global/variant: **16,737** entries, preserving unsupported/failing outcomes and all applicable manual/visual/platform assessments.
- Five families and all eight frameworks: the complete frozen behavior matrix and original attempts, independently checked outcomes, negative/recovery behavior and concurrent isolation.
- Security, process/agent lifetime, memory/performance/churn, media, topology, provenance and every remaining qualification component required by the full plan.
- Original RSS/timeout/Range/booking reproductions and required independently reviewed explanations. Newly identified pinned contradictions stay explicit and unwaived until the existing source/assertion-bound review process is satisfied.

Each claimed platform and execution policy needs its own results. The primary signed macOS BrowserContext runtime is distinct from raw ownerless helper APIs. Source snapshots, path inventories, successful compilation and probe-mode tests cannot substitute for production qualification.

If fixes are needed during qualification, retain the prior failed results, create the next reproducible candidate and rerun affected checks plus all final same-candidate gates. Completion requires the full plan's exit criteria on one final candidate, not an aggregate of passing rows from different binaries or profiles.
