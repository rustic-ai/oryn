# Form activation checkpoint

Follow-up: [label associations, focus and inert hit testing](WPT-LABEL-ASSOCIATIONS-2026-09-12.md).
The results below remain the historical form-activation checkpoint.

This is a bounded repair checkpoint, not completion of Core conformance. The
[full completion gates](WPT-CORE-COMPLETION-GATES.md) remain unchanged.

## Implemented mechanisms

- Native live checkedness, dirty checkedness and indeterminate state, separate
  from content attributes; clone/import preservation and reset handling.
- Radio grouping by tree, case-sensitive name and form owner, including changes
  to connectivity and explicit form association.
- Shared checkbox/radio preactivation, canceled-click restoration, input/change
  events, recursion guards and label forwarding that preserves pointer metadata.
- Cancelable form reset and live checkedness consumers in FormData, selectors,
  CSS state and semantic/accessibility projection.
- Native pointer dispatch for test-driver element clicks, with hit testing and
  interception rejection instead of an untrusted HTMLElement.click call.

The Boolean conversion repair from the preceding checkpoint is included in the
signed candidate below. These changes do not implement every form API or every
activation edge case.

## Signed evidence

The 57-execution inventory includes all mapped Core inert tests, mapped label
tests and mapped input tests whose filenames contain checkbox, radio or checked.
Selection is inventory-based, not pass-based. It is not the complete forms suite.

| Run | Raw passing executions | Elapsed |
| --- | ---: | ---: |
| form-activation-v1 | 21 / 57 | 238.411 s |
| form-activation-v2 | 23 / 57 | 237.640 s |
| form-activation-v3 (navigation follow-up) | 24 / 57 | 237.629 s |

V2 observed 330 subtests, of which 204 passed. The source-bound qualification
overlay qualifies **20 executions / 108 subtests**; raw passes must not be used
as the qualified count. The inert subset now has 10/29 raw passes, preserving
all nine previous raw passes and adding the inert form-control test.

Evidence paths, relative to the repository:

- `benchmarks/conformance/wpt-form-activation-v1.json`
- `benchmarks/conformance/wpt-form-activation-v2-reviews.json`
- `artifacts/wpt-full-baseline/form-activation-v2-2026-09-12.json`
- `artifacts/wpt-full-baseline/form-activation-v2-qualification-2026-09-12.json`

V2 raw SHA-256:
`8bd8ec41739af68992ab823cbe9e57808b932175eb8eb774a2d79485c2298dfa`.

V2 identities:

- Parent: `1f87152e3aea147dfef239f56f07cd34649915a6ef5142225404b7519d52cdff`
- Worker: `4fc8b7058ddfd76f3fe175792ebe7d685b71a0a1d80253686b1fcbe0ce31fef7`
- CDHash: `1dad213d59229eab322095e86c44f3e59c71a128`
- WebIDL: `ecbdaccbf3b266a595371cf10847837b900fb41983b0d822754c290262e5160a`

The signed build passed designated-requirement verification, protocol-v9
handshake and the build script's sandbox probes. These are **dirty-tree
diagnostic artifacts**, not clean-source release/security attestation.
Runs used two persistent parents, isolated prewarmed fresh worker pages,
fresh fixtures and Chromium comparison. Historical artifacts are unchanged.

Three raw passes are deliberately not qualified:

- `clicking-noninteractive-unlabelable-content`: all 51 native assertions pass,
  but attribute serialization order changes generated names versus Chromium.
- `radio-double-activate-pseudo`: its final assertion does not establish that
  the incomplete keyboard activation default actually occurred.
- `dynamic-inert-on-focused-element`: six native assertions pass, but this
  execution's Chromium reference failed native focus verification.

The live harness suite passed 125 tests with zero skips in 109.934 seconds.
That does not erase the reference-focus failure recorded in the raw run or
prove the intermittent focus defect resolved.

## Validation before the navigation follow-up

DOM: 65 tests passed. Layout: 74 passed. V8 bindings: 209 passed, three existing
ignored tests. Native form regressions: two passed; native focus regressions:
seven passed. Node action/adapter tests: 22 passed. Formatting, WebIDL generation
check and targeted clippy with warnings denied passed.

## Navigation follow-up (after the signed V2 run)

A new regression reproduced unwanted ancestor-link navigation after a label
forwarded activation to its checkbox. Native navigation now stops at a label or
checkable input instead of also following its surrounding hyperlink. The
regression covers label descendants, direct checkbox clicks and a positive
ordinary link-descendant navigation control. All three native form regressions
pass after this change.

This follow-up is not included in V2. The subsequent signed **V3** execution
repairs `label-inside-anchor.html`: its one subtest passes, matches Chromium,
and qualifies. All 23 V2 raw passing executions remain passing. V3 observes
331 subtests / 205 passing and qualifies **21 executions / 112 subtests**.

The qualified count is not simply V2 plus one: the six-subtest dynamic-inert
execution now passes reference gates, while the three-subtest
`nested-inert-unfocusable` execution fails Chromium reference-focus verification.
The latter remains a raw native pass, not a qualified pass. The serialization
identity and keyboard-audit exclusions also remain. This variation does not
establish a fix for intermittent reference focus.

V3 artifacts:

- `artifacts/wpt-full-baseline/form-activation-v3-2026-09-12.json`
- `benchmarks/conformance/wpt-form-activation-v3-reviews.json`
- `artifacts/wpt-full-baseline/form-activation-v3-qualification-2026-09-12.json`

V3 raw SHA-256:
`d17aa6e1f127744885d5b5f37393f4466142d62974ce29e58895bca57d5ed15f`.
Parent SHA is unchanged from V2. Worker SHA:
`0244e5b4b7ae17d0bd938430b06ba77fe66c92edf742b56e9b94a2aa50ef3fe5`;
CDHash: `95f08b4abcf77d7ac417f95ee5322fb5e15c304f`.
The signed build's designated requirement, protocol-v9 handshake and sandbox
probes pass. V3 remains dirty-tree diagnostic evidence, not release attestation.

Post-follow-up validation: all 148 enabled native executable tests pass, with
six existing ignored tests; targeted clippy with warnings denied and formatting
pass. The broader 430-test native run was interrupted before release binaries
were replaced and is **not** a full-suite pass. Two loopback fixture tests
(XHR and cross-origin credentials) failed in the filesystem/network sandbox;
both pass when rerun with local listener access. The sandboxed XHR reproduction
explicitly failed fixture bind with PermissionDenied.

Activation target changes during dispatch still require a more complete
dispatch-time ownership implementation; V3 does not claim that broader repair.

## Remaining work, in dependency order

1. Finish activation ownership and dispatch-time target retention, including
   DOM/type mutations during listeners; verify against signed WPT.
2. Implement label IDL, live labels collections and associated-control focus.
3. Implement validity state, complete successful-control/form reset behavior,
   radio keyboard navigation and keyboard activation defaults.
4. Preserve attribute insertion order in serialization without weakening
   reference subtest identity checks.
5. Repair remaining inert/frame/selection coverage and intermittent reference
   focus; keep the deferred Range.insertNode crash visible and unresolved.
6. Reconcile and execute the entire Core inventory on a frozen signed candidate,
   then satisfy the clean-source and security completion gates.

Other known limitations include select/file reset state, broader FormData
successful-control rules, form-associated custom elements, and native checkbox
pixel rendering. The current evidence establishes none of these as complete.
