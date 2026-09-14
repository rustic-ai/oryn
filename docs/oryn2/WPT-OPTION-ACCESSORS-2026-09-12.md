# Option accessor repairs

The completed option-select-v1 broad diagnostic identified missing option text,
label, index and form bindings. This follow-up exposes native implementations:

- text replaces children, preserves UTF-16, and reads collapsed descendant text
  excluding HTML/SVG script contents;
- label/value fall back to option text only when the unnamespaced content
  attribute is absent (an explicitly empty attribute remains empty);
- index follows the current native option list, returning zero outside it;
- form uses the owning select's form owner, including external associations.

Nearest-select traversal now observes datalist/hr/option and nested-optgroup
barriers. Previously, adding an excluded option could normalize an unrelated
select and undo its explicitly empty selection. A native regression covers this.
The implementation follows the HTML option algorithms:
https://html.spec.whatwg.org/multipage/form-elements.html#the-option-element

The binding suite passes 219 tests with three existing ignores; native executable
tests pass 158 cases with six existing ignores, and all 80 DOM tests pass.
Targeted DOM/JS/native clippy with warnings denied, generated IDL consistency and
formatting checks pass. The signed build passed signature/designated-requirement
validation, protocol-v9 handshake and filesystem/listener/outbound/secret denial
probes. Independent inert hit-testing returned `ok`.

All eleven option files passed in 50.085 seconds: 108/108 subtests, compared with
24 passing and 84 failing assertions in the same files of option-select-v1.
Nine files changed from raw failure to pass. Full pinned test/helper review,
actual response-byte hashing and current reference/completion/resource gates
qualify all eleven files and 108 subtests. This is a qualified focused diagnostic,
not full Core completion or clean-source attestation. The prior raw artifacts
remain immutable.

The unchanged 66-file regression completed in 272.681 seconds: 43 raw passes,
23 failures, and 417 passing / 69 failing / one timed-out subtest. Every file
outcome matches option-select-v1; no old raw pass was lost. Fresh source/resource
verification and a new hash-bound inert probe qualify 40 passing files (335
subtests), leaving 16 review candidates and ten unresolved executions. This
qualification count is execution-specific, not a new engine gain: differing
reference outcomes change which old files qualify.

Three raw passes are deliberately excluded: focus-tabindex-positive has failed
reference focus/completion gates; radio-double-activate-pseudo lacks the required
fixture/automation audit; clicking-noninteractive-unlabelable-content has a
reference subtest-identity mismatch. No exclusions were converted into passes.

Review and qualification artifacts:
`benchmarks/conformance/wpt-option-accessors-v1-reviews.json` and
`artifacts/wpt-full-baseline/option-accessors-v1-qualification-2026-09-12.json`.
Raw SHA-256: `de56c3aabf72b9a6c4837359444dc2560669bddabf7694ef0635e9a345cab22f`.

Regression reviews and overlay are
`benchmarks/conformance/wpt-option-accessors-v1-regression-reviews.json` and
`artifacts/wpt-full-baseline/option-accessors-v1-regression-qualification-2026-09-12.json`.
The independent probe is
`artifacts/wpt-full-baseline/inert-hit-testing-option-accessors-v1-probe-2026-09-12.json`.
Qualification/evidence unit tests pass 17 cases. No full-workspace, Linux release
or full-Core acceptance run was performed for this bounded repair.

Remaining gaps include live HTMLOptionsCollection, selectedOptions, select
length/indexed mutation/removal, constraint validation, and the Option factory's
cross-realm non-object prototype fallback. General coexistence of attributes
with identical qualified names in different namespaces remains a separate DOM
storage limitation; ignoring a namespaced label/value here does not repair it.

Frozen signed candidate:

- Parent SHA-256: `c53de771938b7308111c9daaebc46de7c33fbd767815234edc69c6c83ce12166`.
- Worker SHA-256: `04305a58bcd6c75c2627faf8994f8590f6545c4203c6accefa50456d46a8b882`.
- CDHash: `520f3aaf54efbe7b71b9a9ccc008f9d9361db33b`.
- IDL SHA-256: `7c20b47ab823e73b34e2933e3fcd983afa4e9bef56453cff949fe3426e886831`.

The focused inventory contains all eleven mapped Core option files, selected by
directory without outcome filtering. Fresh raw evidence paths:
`artifacts/wpt-full-baseline/option-accessors-v1-2026-09-12.json` and
`artifacts/wpt-full-baseline/option-accessors-v1-regression-2026-09-12.json`.
