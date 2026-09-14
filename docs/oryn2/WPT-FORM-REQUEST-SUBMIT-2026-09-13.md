# requestSubmit validation and reentrancy follow-up

The requestSubmit binding now validates the optional submitter before submission:
non-submit controls are TypeErrors and submit buttons owned by another form cause
NotFoundError. Native namespace-aware attribute reads handle input submit/image
types and button submit/Auto behavior, including command attributes and select
parents. Disabled submitters are accepted by requestSubmit.

Connected forms run the existing static validation phase unless the form has
novalidate or the supplied submitter has formnovalidate. Namespaced lookalikes do
not bypass validation. Invalid results suppress the submit event. A per-form
submission-events guard spans validation and submit dispatch, and is cleared on
success, cancellation and error. Nested requestSubmit calls cannot recursively
validate or submit that form. Submit events are trusted, bubbling and cancelable.
Disconnected forms do not submit; connectivity is checked again after callbacks.
The submit() method continues to bypass validation and submit events.

The focused regression covers submitter types and ownership, external/disabled
submitters, namespace-aware bypass, invalid and submit callback reentrancy, guard
release, omitted/null/undefined submitters and disconnected forms.

Verification: 234 JS tests passed (three existing ignores), 159 native executable
tests passed (six existing ignores), and targeted DOM/JS/native clippy passed with
warnings denied. The additional submit() bypass assertion is covered by a fresh
focused regression rerun.

Specification: [requestSubmit](https://html.spec.whatwg.org/multipage/forms.html#dom-form-requestsubmit)
and [form submission](https://html.spec.whatwg.org/multipage/form-control-infrastructure.html#form-submission-algorithm).

## Not yet complete

This is source-ahead work, not signed WPT evidence. The earlier signed worker and
WPT artifacts remain unchanged. Interactive error reporting, user-validity state,
SubmitEvent branding and submitter retention, click/default-action integration,
submitter data and action/target overrides, and full navigation/sandbox/entry-list
guards remain open. Input/textarea constraint calculation remains incomplete.
In particular, the new requestSubmit guard is not proof of click/requestSubmit
cross-entry-point reentrancy correctness. This does not claim complete submission
or Core conformance.
