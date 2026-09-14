# Live selectedOptions collection

HTMLSelectElement.selectedOptions now exposes a native SameObject HTMLCollection,
not a snapshot. Every indexed/named/length access resolves the current native
option list and selectedness. This preserves invalid-nesting barriers, tree order,
explicit empty selections, multiple selection and live removal.

The existing collection implementation supplies branding, item/namedItem and
legacy indexed/named property behavior. A weak per-select cache preserves
SameObject identity without globally rooting detached selects. GC regression
tests retain owner/member expandos while the collection is held, then collect
all lists after releasing references, including 128 unreachable owner/list cycles.

The implementation follows the HTML selectedOptions algorithm:
https://html.spec.whatwg.org/multipage/form-elements.html#dom-select-selectedoptions
Both pinned selectedOptions tests were reviewed, including all eleven invalid
nesting cases and eight identity/membership assertions.

The V8 binding suite passes 220 tests with three existing ignores, including the
expanded GC test. Targeted JS/native clippy with warnings denied passes. The
signed build passed signature/designated-requirement validation, protocol-v9
handshake and filesystem/listener/outbound/secret denial probes. Independent
inert hit-testing returned ok. Upstream WPT verification is running; no new
qualified pass is claimed yet. Native executable tests pass 158 cases with six existing ignores;
generated IDL and formatting checks pass. Prior evidence remains immutable and all new runs remain dirty-tree
diagnostics, not release or full-Core attestation.

This does not implement HTMLOptionsCollection or the mirrored select indexed
properties, length mutation, add/remove/item/namedItem. Selection/reset algorithm
failures, constraint validity, picker and layout interaction remain separate
unfinished groups. The full Core objective remains unchanged.

Next-action evidence: the prior select-ask-for-reset file passes insertion but
fails removal and selectedness changes. Both failing test paths set select.size
to two; the IDL currently lacks size, so that assignment is merely an expando
and does not reach the native size-aware normalization algorithm. Implement and
test WebIDL unsigned-long reflection (including conversion and invalid/default
values), rather than changing the reset algorithm to accommodate that expando.
The selectedness-script-mutation test's five synchronous cases can now exercise
the live selectedOptions prerequisite. These are hypotheses/prerequisite repairs
to verify with the next signed raw results, not waived failures.

Further specification review confirms the insertion-order issue is independent
of the collection API: when a selected option is added to a single select's
option list, other options must be deselected even if they follow the inserted
option in tree order. Current reconcile_option_owners only normalizes the final
list, choosing its last selected option. Fix owner-change insertion processing
and cover insertion before an existing selected option as well as selected
optgroup/fragment insertion; retain same-owner move and explicit-empty behavior.
Reference: https://html.spec.whatwg.org/multipage/form-elements.html#the-select-element

For size reflection, the unsigned-long conversion precedes reflection. Both
getters and setters use zero outside the allowed reflected range 0..2147483647;
the getter parses the content attribute as a non-negative integer prefix. The
display-size algorithm is a separate consumer and must not be replaced by this
IDL getter. Reference:
https://html.spec.whatwg.org/multipage/common-dom-interfaces.html#reflecting-content-attributes-in-idl-attributes

Frozen candidate identities:

- Parent SHA-256: `82bc29d03383a89f88bd5ab01a2b66106a03af14d286181cce53a5fe94547fa6`.
- Worker SHA-256: `52d627622999346d5744b50bf2cd4b79fdc6342b26813dff7c070db48932022a`.
- CDHash: `be178ae498806de2fdd8cd9d777b1dd0aee37bb4`.
- IDL SHA-256: `c2e12da2df29319e6cfbce4d7bbac7b8a127267404f4719f4a38e3b607263c18`.

The unchanged 125-file select/options and 66-file regression cohorts are running
with persistent parents, isolated prewarmed workers and host-serialized Chromium
reference focus. Raw paths:
`artifacts/wpt-full-baseline/selected-options-v1-broad-2026-09-12.json` and
`artifacts/wpt-full-baseline/selected-options-v1-regression-2026-09-12.json`.

The regression cohort completed in 276.457 seconds with 43 raw passes and 23
failures, identical file outcomes to option-accessors-v1. Rechecked pinned
sources, response bytes and the independent inert probe qualify 41 passing
files / 336 subtests; 16 executions remain review candidates and nine unresolved.
The two excluded raw passes are radio-double-activate-pseudo (missing audit) and
clicking-noninteractive-unlabelable-content (reference subtest identity mismatch).
Qualification artifacts use the selected-options-v1-regression prefix. The broad
run remains pending; no result is inferred for its unfinished executions.

The broad cohort subsequently completed in 513.543 seconds: 28 raw passing
files, 97 failures, 204 passing / 244 failing / three timed-out / four not-run
subtests. Compared with option-select-v1, thirteen files improved and no raw
pass was lost. Nine improvements are the option accessors, one is the related
option form-ancestor test, and three are selectedOptions/state-mutation files.
The latter three pass all 24 subtests. Review artifacts use the
selected-options-v1-broad prefix and audit the eleven option files plus these
three files, including independently reconstructed upstream WindowHandler bytes.

The insertion test now reaches its final assertion and exposes the predicted
failure: inserting selected B before C leaves C selected. The next candidate
repairs that state transition and size reflection; this completed raw evidence
is not retroactively attributed to those later changes.
