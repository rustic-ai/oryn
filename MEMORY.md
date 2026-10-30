# Oryn project memory

Persistent project notes for future agents. Read with AGENTS.md and the adopted
workflow. This is project-local memory, not account-wide assistant memory;
current source, receipts and handover status remain authoritative.

## October 29, 2026: coherent repair groups before expensive verification

The user explicitly instructed: always implement fixes in mid-to-large coherent
groups, then build and test. Repeated small edits followed by compilation and
testing consume substantial time and fragment delivery.

Carry-forward execution rule: inventory the related native, IDL/generator,
caller and lifecycle changes; implement and review the dependency-coherent group
with cheap source/generation checks first. Then compile and execute grouped
regressions, followed by required fresh signed-worker integration and affected
WPT coverage. Avoid a build/test cycle for each small edit. Do not manufacture
large groups from unrelated changes or weaken required verification. Preserve
failure evidence and diagnose a failed group before launching the next run.

## October 26, 2026: scheduling and honest closure

The user challenged verification taking hours. The macOS one-shot launcher had
explicitly selected launchd `ProcessType=Background`. A reviewed Standard owner
reran the exact compiled DOM/JS/layout/native test binaries against the unchanged
fresh signed worker, without source edits or recompilation.

Normal-priority verification completed in about nine minutes. Identical
DOM/JS/layout tests ran approximately 4.6/5.1/5.2 times faster. Seven of nine
original native failures disappeared, but two remained. This measures an
execution scheduling penalty, not a corrected compiler build or particular
CPU-core residency. Do not compare nine-minute execution directly with a prior
run that included hours of compilation.

At this checkpoint, four of seven WPT fixtures passed all eleven assertions at
both one and four workers. Cross-partition BlobURL and dedicated-worker CSP
timed out; encoding still failed semantic allocation admission. Batch A stayed
open. Original failures were retained, cleanup settled, and no job remained
running. Consult `docs/validation/WPT-HANDOVER-2026-10-25.md` and later updates
for current status; these dated outcomes are not permanently current.

Carry-forward rules:

- Default user-requested builds/tests to normal scheduling, verify policy and
  startup, and diagnose unusual slowness before repeating expensive work.
- Group coherent repairs/builds. Reuse unchanged binaries for diagnostics
  without weakening fresh-worker verification or production limits.
- Separate compilation, execution, cleanup and qualification. Preserve failures;
  do not widen limits to hide them or close a batch on partial success.
- Finish authorized implementation through required gates. If stopping, report
  why and the real active-job state; a promise is not background execution.
