# S form-associated custom elements — source generation 2026-09-16

**Source only; not compiled, tested, generated through WebIDL, or certified.** The global all-code-before-execution gate remains in force.

## Native behavior generated

- Definition snapshots `formAssociated` and four form lifecycle callbacks in addition to existing custom-element callbacks. Native upgrade registers exact generational NodeIds; failed upgrade unregisters. Registry state follows adoption; checkpoints reassociate form ownership and fieldset disabled state using the DOM, including external form IDs.
- DOM form/fieldset collections, successful-control membership, labelability, disabled first-legend exceptions and validation eligibility recognize registered custom controls. Registration and validity changes emit native FormState invalidations. Document replacement and node reclamation clear registration/flags.
- ElementInternals source exposes form, labels, setFormValue, setValidity, validity, willValidate, validationMessage, checkValidity and reportValidity. Non-form-associated hosts are rejected. Validity flags remain separate from message text, are live through ValidityState, and enter native form validation/invalid events. Validation anchors use private native identity and are retained through traced wrapper edges; reporting focuses a still-descendant anchor.
- FormData/native submission entry construction consumes ordered native snapshots. A string/File uses the control name; FormData contributes its own names, including duplicate/empty names. Later mutation of the supplied FormData does not alter the snapshot. Disabled and datalist controls are excluded. Null/undefined clears the submission value.
- Accepted form reset queues the native custom-control reset callback; canceled reset does not. Reset callbacks retain existing reaction ownership and exception processing.
- `oryn-js/src/form_state.rs` defines worker-safe state independent of submission: Null, String, File with native bytes, and ordered FormData entries. Captures include exact node/document, definition, control name, form node/id/name/action, and occurrence ordinal.
- Public V8 binding capture/restore entry points accept exact same-document identity or an explicit native cross-document mapping. They reject retired/wrong-document/stale/nonmatching controls and never guess by name alone. Restoration enqueues the definition callback without silently overwriting current submission state. Null saved state has no callback.

## Regression sources and source references

New DOM regressions cover native collection registration, external owners, disabled legend and validity flag separation. V8 regressions cover construction, entry snapshots, null submission, live flags, failure atomicity, reset, label ownership, reassociation, disabled callbacks, non-FACE rejection, receiver branding, saved/submitted value separation, stale generation and retired realm rejection.

Reviewed pinned WPT sources under `custom-elements/form-associated`: ElementInternals-validation.html, ElementInternals-setFormValue-nullish-value.html, form-associated-callback.html and form-disabled-callback.html (checkout 719d5e38fdd0903a18ed9007aba816c98cc491e0). These are source references, not passing results.

## Remaining integration and limits

- Parent owns actual native history capture/routing and cross-document matching. Typed APIs alone do not make reload/history restoration complete. Matching metadata is supplied; native mapping policy remains explicitly unresolved.
- Parent/layout owner must project FACE registration/validity into layout selector state (`:enabled`, `:disabled`, `:valid`, `:invalid`). DOM truth and invalidations are generated, but layout consumers use their own state paths.
- WebIDL source is updated; generated binding files deliberately await the global gate. All new Rust and V8 regression sources await the same gate.
- Broader ElementInternals accessibility/default ARIA APIs and experimental submit-behavior APIs are not implemented in this batch. Native source review and full pinned custom-element/WPT execution remain necessary; no full S completion claim.
