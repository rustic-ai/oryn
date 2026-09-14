# SubmitEvent and shared click submission

SubmitEvent is now a Window-exposed Event subclass with a readonly nullable
HTMLElement submitter. Constructor conversion reads type, inherited EventInit
members and submitter in WebIDL order. Null/undefined submitters default to null;
non-HTML nodes, proxies and lookalikes are rejected. The submitter is stored as
a V8 private traced reference, not an author-overwritable own property or a native
strong-handle table. Receiver checks reject borrowed access on plain Event,
inherited objects and proxies. initEvent retains the submitter.

requestSubmit creates trusted SubmitEvent objects with the actual button or null
when invoked without one. Mouse/pointer click activation on submit buttons now
runs synchronously through the same validation and submission-event guard.
Canceled clicks, disabled controls and controls changed to non-submit types do
not submit. Descendant clicks resolve the owning submit button. Generic untyped
Event('click') does not gain activation behavior.

Focused tests cover constructor conversion order, exception identity, subclasses,
receiver checks, detached submitter identity, all click/requestSubmit reentrancy
combinations during both invalid and submit events, and cancellation/type changes.
A GC regression retains an event alone and verifies its detached submitter remains
live; 128 unreachable event/submitter cycles collect and dropping the held event
restores the wrapper count.

The first full native rerun exposed a document-targeted-click regression caused
by reading an element attribute before checking the candidate node type. The
shared submit-button predicate now rejects non-input/non-button nodes first.
The corrected full reruns passed 237 JS tests (three existing ignores) and 159
native executable tests (six existing ignores). Generated IDL consistency passed.
Targeted DOM/JS/native clippy with warnings denied, formatting and diff checks
also passed after the guard correction.

This source is newer than the last signed worker. No signed WPT or clean-source
attestation is claimed. Complete submission still needs submitter propagation
into the navigation/entry-list queue, action/target/method overrides, interactive
validation reporting, user-validity state, all native control constraints and
full navigation/sandbox guards. Existing pending Submit actions retain only the
form; this report does not claim submitter data reaches the submitted payload.

IDL generation follows the existing event declarations: an omitted optional init
dictionary is handled as empty at runtime because the local parser does not yet
accept an explicit `{}` argument default.
