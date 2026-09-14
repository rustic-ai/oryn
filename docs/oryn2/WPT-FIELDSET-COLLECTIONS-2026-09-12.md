# Fieldset collections and UTF-16 follow-up

Core conformance remains incomplete. This source checkpoint follows the
[form collection evidence](WPT-FORM-COLLECTIONS-2026-09-12.md); it does not
rewrite that signed candidate or claim a new WPT pass.

Implemented native HTMLFieldSetElement branding and receiver checks, type,
form association, disabled/name reflection and SameObject elements. The
collection is HTMLCollection, not HTMLFormControlsCollection. Its built-in
listed descendant membership includes image inputs and ignores form ownership;
it excludes the fieldset itself and shadow descendants. Membership, indexing,
named access, weak caching and owner/member retention remain live.

This follows the [fieldset elements definition](https://html.spec.whatwg.org/multipage/form-elements.html#dom-fieldset-elements).
Form-associated custom elements and fieldset constraint-validation APIs are
still unfinished; the built-in helper does not claim to cover them.

Form collection namedItem and RadioNodeList value setters now preserve UTF-16
code units during conversion. Shared HTMLCollection named-property lookup,
descriptors, enumeration and base namedItem use the same preserving conversion.
Regression coverage distinguishes lone high/low surrogates from U+FFFD,
preserves surrogate pairs, and checks object conversion and exception identity.

Validation completed before the signed rebuild:

- Full V8 binding suite: 215 passed, three existing ignores.
- Extended GC regression verifies fieldset owner/member expando retention,
  SameObject after GC, collectible detached cycles and final zero collections.
- Fieldset regression covers all seven built-in listed categories, image inputs,
  duplicate names, reassociation, reorder/removal, detached roots, open/closed
  shadow boundaries, reflection and invalid receivers.
- Targeted DOM/JS clippy with warnings denied, generated WebIDL check,
  formatting and diff checks passed.

The native executable suite also passed: 158 tests, six existing ignores.
The signed rebuild passed, including signature validation, protocol-v9 handshake
and filesystem/listener/outbound/secret sandbox probes. Independent signed
inert hit-testing returned `ok`.

Worker SHA-256: `49638ddedf1c29aca913956d4cc929f85f75e8651ccc1e8ae8a3e7cc612657fc`.
Code Directory hash: `3c983c573579da35ce6b6a6ed7f092574a0e36f7`.
WebIDL SHA-256: `2c560d4c1ed6f9b005ca2eb835a05fc7e4a946148d2eafb4baa09e76a2cce848`.

The unchanged 66-test WPT regression run completed in 276.079 seconds, writing
`artifacts/wpt-full-baseline/fieldset-collections-v1-2026-09-12.json`.
A separate baseline of every mapped Core constraints test (28 executions)
also completed on the same candidate in 217.843 seconds, writing
`artifacts/wpt-full-baseline/form-constraints-v1-2026-09-12.json`.
The new `benchmarks/conformance/wpt-form-constraints-v1.json` was selected
from inventory-v3 by directory and tier, with pinned hashes checked and no
outcome filtering. Full-Core unsupported/manual coverage remains unfinished.
Both runners use the existing host-serialized reference focus lease.

The regression cohort records 43/66 raw passes, up from 42/66, with no previous
raw passes lost. The collection interface now passes all 16 subtests. Total
observations are 417 passing subtests, 69 failing and one timeout. The fresh
strict overlay qualifies 40 passing files / 310 subtests, with 15 review
candidates and 11 unresolved files. Lower qualified subtest totals than the
previous run reflect different reference failures, not raw-pass regressions.

Current raw passes excluded from qualification are radio-double-activate-pseudo
and labelable-elements (reference focus/completion incomplete), and
clicking-noninteractive-unlabelable-content (reference subtest identities differ).
No reference gate was waived. Pinned source hashes and actual response bytes
were independently checked before reviews were rebound. The fresh independent
inert probe is bound to the raw artifact and both executable hashes.

Qualification artifacts:

- `benchmarks/conformance/wpt-fieldset-collections-v1-reviews.json`
- `artifacts/wpt-full-baseline/fieldset-collections-v1-qualification-2026-09-12.json`
- `artifacts/wpt-full-baseline/inert-hit-testing-fieldset-collections-v1-probe-2026-09-12.json`

Parent SHA-256: `a80d12dcb6b98677a2112eebc31672bae6e95273bfb001938dbbf7d69307dd90`.
A direct signed probe additionally checked image-input membership, SameObject,
the HTMLCollection rather than HTMLFormControlsCollection brand, lone-surrogate
named lookup/enumeration and radio value setting; it returned `ok`.

The constraints baseline records 0/28 passing files, 5 passing and 880 failing
subtests. Six files terminate with `Uncaught ReferenceError: Option is not defined`;
those are incomplete observations, not complete validity test results. One other
file (`inputwillvalidate.html`) lacks complete reference focus evidence. No
constraints pass or failure qualification is claimed.

Next native
dependency is shared constraint-validation state and algorithms: current form
validation scans descendants rather than native form-owned members, and the
public ValidityState interface is absent. Do not implement a constant property
or qualify a valueMissing-only stub as complete constraint validation.
Do not promote the previous 42/66 raw score as evidence for this source version.
