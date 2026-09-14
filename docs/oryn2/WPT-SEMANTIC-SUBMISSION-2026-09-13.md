# Semantic submission shares generated activation

The native semantic action path previously detected a submit button independently,
dispatched another generic Event('submit'), and navigated directly. After shared
generated click activation was added, this could produce duplicate submit events
and bypass validation or cancellation.

That duplicate path and its ancestor-only form lookup have been removed. Semantic
form-submit actions invoke requestSubmit, and semantic clicks use the generated
mouse activation path. Queued accepted submissions own navigation. Ordinary link
navigation is preserved; button and submit/image input activation do not also
activate an enclosing hyperlink.

The native regression exercises internal buttons, externally associated buttons
and direct form-submit actions. Each produces exactly one invalid or typed submit
event; invalid or canceled cases preserve location, including a button nested in
an anchor. The focused regression passed.
The full native executable suite passed 160 tests (six existing ignores).
Targeted DOM/JS/native clippy with warnings denied, formatting and diff checks
also passed.

This does not complete the queued submission implementation. Pending Submit still
carries only the form, so submitter identity/data, entry-list snapshots and
submission overrides remain open. Full control constraints, interactive reporting,
navigation guards and signed upstream WPT verification are also outstanding.
These source changes do not alter or qualify historical signed evidence.
