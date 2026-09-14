# Select size and insertion-state repairs

The selected-options-v1 signed cohort confirmed insertion before an existing
selected option incorrectly preserved the later selection. Incoming selectedness
is now captured before peer deselection, then applied in subtree order. This
lets the last incoming selected option win in optgroup/fragment insertion while
preserving dirtiness and avoiding changes for same-owner moves.

HTMLSelectElement.size is now an unsigned-long reflected property with default
zero, numeric conversion before mutation, integer-prefix parsing and reflected
range handling. This reaches the existing native size-aware reset algorithm.
The separate display-size algorithm is unchanged. The readonly type getter
reports select-one or select-multiple from the multiple attribute.

Binding tests cover every insertion position, multiple selected options in an
inserted group, preservation of non-dirty state, nested-select exclusion,
size conversion/exception/default/range cases and listbox removal/reset behavior.
All 222 binding tests pass (three existing ignores), all 80 DOM tests pass, and
158 native executable tests pass (six existing ignores). Targeted JS/native
clippy passes with warnings denied. The signed rebuild passed signature and
designated-requirement validation, protocol-v9 handshake and filesystem/listener/
outbound/secret denial probes. Independent inert hit-testing returned ok. The
unchanged 125-file broad and 66-file regression cohorts are running with
persistent parents and isolated prewarmed workers; no new WPT pass is claimed
for these changes yet.

Full Core conformance remains incomplete. Live HTMLOptionsCollection and mirrored
select indexed properties/mutation, constraint validity and picker/layout
interaction remain unfinished. Historical raw evidence remains immutable.

Frozen signed candidate:

- Parent SHA-256: `9fbd237893878bd86c6319033cca0e51bf0b3ff4a79d9b5e992f2f37f67d7b9e`.
- Worker SHA-256: `9415771fa78bfa3935a5bc1293c419149b7d4a4a5dc25469694569346895593f`.
- CDHash: `87598412fd84bb43c50c0b9562e5aa4e27b2712c`.
- IDL SHA-256: `4245cec1944ac558dfcd1f62dcc4dacfec9d2cd52b8de587903329c568e677e1`.

Pending raw paths are
`artifacts/wpt-full-baseline/select-size-insertion-v1-broad-2026-09-12.json` and
`artifacts/wpt-full-baseline/select-size-insertion-v1-regression-2026-09-12.json`.
These remain signed dirty-tree diagnostics, not clean-source release attestation.

Both runs completed. The broad cohort took 525.429 seconds and reports 30 raw
passes / 95 failures, with 207 passing, 241 failing, three timed-out and four
not-run subtests. Compared with selected-options-v1, inserted-or-removed and
select-ask-for-reset changed from failure to pass; no passing file was lost.
Both pass the complete current qualification gate (eight subtests). Seventeen
reviewed files / 142 subtests qualify in the broad overlay, with 80 review
candidates and 28 unresolved executions retained.

The regression cohort took 279.478 seconds and remains 43 raw passes / 23
failures with identical file outcomes. Forty-one files / 336 subtests qualify;
16 review candidates and nine unresolved executions remain. Two raw passes
remain excluded for the existing missing audit/reference identity issues.
Review and qualification artifacts use the select-size-insertion-v1-broad and
select-size-insertion-v1-regression prefixes.

While these frozen release binaries were running, the next options-collection
source changes were developed and tested only with debug test binaries. They
are not included in, or evidenced by, this signed candidate's results.
