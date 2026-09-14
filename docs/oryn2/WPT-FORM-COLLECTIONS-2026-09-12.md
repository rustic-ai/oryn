# Form collection bindings checkpoint

Core conformance remains incomplete under the unchanged
[completion gates](WPT-CORE-COMPLETION-GATES.md). This is a signed diagnostic
checkpoint, not a full-Core score or clean-source release attestation.

## Implemented

Public native `HTMLFormElement.elements` now returns a live, SameObject
`HTMLFormControlsCollection`, with `form.length`, indexed access and named
lookup. Duplicate names return a live `RadioNodeList`; its value getter and
setter use native radio checkedness and group behavior. Membership observes
native form ownership and tree order, including external associated controls,
mutation and disconnected trees. Image inputs and obsolete keygen are excluded.

Generated WebIDL brands, inherited collection methods, receiver validation,
weak wrapper caching and owner/member GC retention are integrated. Two new
binding tests cover mutation, ownership, brands, conversion side effects,
radio values and event suppression. The collection GC regression now includes
retained forms/radio lists and collectible detached cycles.

## Signed evidence

The new 66-test selection retains every original 64-test entry and adds both
collection-interface tests, regardless of outcome. The frozen signed candidate
ran with two persistent parents and isolated prewarmed workers. Final artifact
elapsed time is **271.212 seconds** (last progress event: 269.3 seconds).

| Selection | Raw passing files |
| --- | ---: |
| Previous radio-arrows-v1, original cohort | 40/64 |
| Current form-collections-v1, original cohort | 41/64 |
| Current expanded cohort | 42/66 |

No previous raw passes were lost. Radio keyboard navigation order now passes
and qualifies, with twenty-two source-derived native action sequences.
RadioNodeList passes all five subtests and qualifies. HTMLFormControlsCollection
passes 15/16 subtests; its fieldset case fails because `fieldset.elements` is
not implemented. This failure remains in the selection and raw evidence.

Across all 66 files, observations are 416 passing subtests, 70 failing and one
timeout. The strict overlay qualifies **39 passing files / 319 subtests**;
16 files await prerequisite review and 11 remain unresolved. These categories
are not additional passes. All 66 executions finished; none was environment-blocked.

Three raw passes are excluded:

- `radio-double-activate-pseudo.html`: the final inactive assertion alone does
  not establish keyboard activation.
- `clicking-noninteractive-unlabelable-content.html`: generated subtest names
  differ from Chromium because of attribute serialization order.
- `inert-does-not-match-disabled-selector.html`: this execution's Chromium
  focus/completion prerequisites failed; historical reference evidence is not
  substituted.

The reference terminal-drain repair is included. The nine-Tab order test now
qualifies with complete current automation evidence. This does not establish
that intermittent reference focus is resolved generally.

Every retained review's pinned source and actual response bytes were rechecked;
template substitutions were checked explicitly. Newly passing collection and
radio-order sources were reviewed before qualification. The independent signed
inert hit-testing probe returned `ok` and is bound to this parent, worker and raw
artifact. Historical raw, reviews and overlays remain unchanged.

Artifacts:

- `benchmarks/conformance/wpt-form-collections-v1.json`
- `artifacts/wpt-full-baseline/form-collections-v1-2026-09-12.json`
- `benchmarks/conformance/wpt-form-collections-v1-reviews.json`
- `artifacts/wpt-full-baseline/form-collections-v1-qualification-2026-09-12.json`
- `artifacts/wpt-full-baseline/inert-hit-testing-form-collections-v1-probe-2026-09-12.json`

Parent SHA-256: `28b34b81a104f509b3b498b3a7b299708fefcfa720b363390aaf2fafc7a328f9`.
Worker SHA-256: `7fa5589342ffe88c16d5f2b25ef4db005eb20c997c9a1b16f62e84e1404f9e67`.
Code Directory hash: `030645a1bfbaf7238e6e504669f532bc7e91b520`.
WebIDL SHA-256: `0cc4fce7d2a4857b709ab83728d369d7869bacfdc24a7e35a5879bbba99e005b`.
Remaining runtime identities are recorded in the raw artifact.

## Validation and unfinished work

- V8 binding tests: 213 passed, three existing ignores.
- Native executable tests: 158 passed, six existing ignores.
- Native DOM foundation: 71 passed.
- Chromium driver tests: 12 passed, including bounded terminal drain.
- Targeted clippy with warnings denied, generated WebIDL check, formatting
  and diff checks passed. GC regression reran after the clippy-only cleanup.
- Signed build, protocol-v9 handshake and filesystem/listener/outbound/secret
  sandbox probes passed. No full-workspace, Linux or full-Core run is claimed.

Immediate remaining collection work is native fieldset descendant membership
and `fieldset.elements`, with its own brand/liveness/GC tests. Fieldset membership
must not reuse form-owner filtering. Form-associated custom elements remain
unsupported. New namedItem/value setter conversions still use lossy string
conversion and need UTF-16-preserving regression coverage; passing the selected
ASCII WPT cases does not prove that edge case.

Form named/indexed access and past-name behavior, validity, cross-frame focus,
broader keyboard activation, inert editing/selection and the previously deferred
Range.insertNode crash remain unfinished. None is waived by this checkpoint.
