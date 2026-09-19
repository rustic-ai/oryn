# Native prompt lifecycle source-generation notes

All changes are **unbuilt and untested**. This work does not establish WebDriver,
HTML prompt, or complete Core conformance. No build, test, formatter, installation,
repository generator or deployment was run.

## Generated implementation

- `crates/oryn-js/src/prompts.rs` is V8-free and always built. It defines serializable
  prompt kinds, realm/document-generation tokens, requests, pending input, responses,
  errors, thread-safe controller, forwarding backend and modal wait observer.
- The controller exposes inspect, set_input, accept, dismiss, lifecycle cancel and
  retire independently of the JS actor queue. A Condvar preserves the suspended JS
  call stack. Stale tokens cannot resolve another prompt; input applies only to
  prompt dialogs. Resolution is single-use. Cancellation wakes with an error,
  never a fabricated user acceptance. Exact token cancellation tombstones handle
  a cancellation IPC message arriving before its request is published.
- A process host supplies `PromptBackend`; its synchronous request uses the native
  context-service transport. Cancellation occurs outside controller locks. The
  parent controller can remain V8-free and serve inspect/resolve while a child
  worker waits. The parent/root owns that runtime/session/HTTP transport.
- `PromptWaitObserver` brackets modal waits with balanced RAII begin/end hooks,
  including forwarded calls, cancellation, retirement and error paths. Host watchdog
  integration must exclude modal wall time while preserving explicit cancellation.
- `v8_bindings/prompts.rs` installs native alert, confirm and prompt functions,
  preserves synchronous results, performs argument conversion before publication,
  and preserves DOM UTF-16 strings through the existing internal representation.
- Trusted beforeunload dispatch creates a native Event record with a
  BeforeUnloadEvent prototype and DOMString returnValue, runs native Window handlers,
  applies the special onbeforeunload return convention, checks document generation,
  suppresses recursive beforeunload dispatch, gates prompting on host-provided sticky activation, and returns the navigation
  decision. The host remains responsible for navigation/traversal/activation policy.
- Native global handler registration now includes beforeunload/unload and print
  lifecycle event properties. Existing event cancelBubble/returnValue behavior is
  retained for ordinary Event records.

## Regression source coverage

V8-free controller sources cover token/generation validation, default and replacement
input, duplicate resolution, wrong input kind, concurrent-open rejection, cancellation
and retirement waking blocked waiters, cross-controller forwarding, cancellation
before publication, and balanced watchdog observer entry/exit.

V8 regression sources cover synchronous alert/confirm/prompt results, dismiss/null,
Unicode/lone-surrogate defaults and results, argument conversion exceptions, trusted
BeforeUnloadEvent identity, cancelability and initial returnValue, activation gating,
handler-return and legacy returnValue cancellation, and retirement rejection.

## Required integration and remaining limits

- Parent/root owns context-service IPC, page-controller lifetime, HTTP prompt policy
  and script command ownership while a prompt is pending. Host owner removes the
  previous auto-dismiss JavaScript bootstrap stubs and installs backend/observer in
  every realm. Those production routes require later source reconciliation and tests.
- Beforeunload must be called before native navigation commits, with real sticky
  activation and correct descendant traversal. This binding does not claim those
  host behaviors are complete or default activation to true.
- The native watchdog implementation is in `oryn-native/src/host.rs`, outside this
  ownership. The observer contract and accounting regression sources are generated;
  the host owner must shift active deadlines around modal waits.
- Sandboxed modal suppression, cross-origin modal policy, complete BeforeUnloadEvent
  IDL descriptor placement and all multi-window
  races still need reconciliation. The prototype is installed natively but does not
  yet replace the generated WebIDL interface catalog entry.
- UTF-16 prompt payloads use Oryn's existing internal DOM-string representation;
  external wire serialization must preserve/decode it rather than exposing its
  internal marker as user-visible text.

Source review also added recursive-beforeunload suppression and catches/reports
exceptions from converting an onbeforeunload handler's returned object to DOMString.
Those conversion errors do not fabricate a prompt or escape with a pending V8
exception. Regression sources cover dispatch-guard reset and error reporting.
Observer accounting and IPC cancellation-before-publication regression sources are
also present; all remain unrun.

## Subsequent policy and interface generation

Generated committed native modal policy (`set_modals_allowed`), enforced by all
three simple dialogs and beforeunload. Frame binding snapshots iframe allow-modals
and conjoins the parent's committed permission; descendants inherit this native
snapshot. Parent context-service frame responses now carry a boolean compiled from
all enforcing response CSP headers. Static, prefetched and dynamic loaders carry
that flag and constrain the committed frame before author scripts. Top-level CSP
snapshots apply before bootstrap. Report-only policy and meta markup do not expand
or revoke this header-derived modal grant. Missing wire permissions fail closed.

Native sticky activation can be set only through the host input API. Synthetic
DOM events, Element.click and author helpers do not set it. Native notifications
propagate to ancestor documents and same-origin Window states. Beforeunload now
requires both native sticky activation and the host's activation gate. Initial
Window/document reuse clears activation, reinstalls modal policy, and renews the
prompt controller only after old waits unwind, preserving backend/watchdog wiring.
The host input owner is wiring the realm-specific activation entry point.

`browser.webidl` now defines nonconstructible BeforeUnloadEvent : Event with its
own DOMString returnValue attribute. Native branded getter/setter dispatch is
separate from Event's boolean returnValue, including borrowed-accessor validation.
The updated generated registry is intentionally deferred with all other generators;
a native prototype fallback remains for the old checked-in registry. Tests cover
IDL placement/brand, blocked modal results, author permission forgery, activation,
policy persistence until navigation, committed-document resets, and response-header
CSP conjunction. None have run.

The earlier blanket sandbox/IDL-generation gaps are therefore narrowed to remaining
cross-origin/top-level modal policy variants, final generated-registry integration,
and complete host input/navigation reconciliation and validation. This remains a
source-generation status, not a conformance result.


### Native navigation and simple-dialog eligibility source completion

Host `dispatch_before_unload_tree` snapshots live frame lifetimes (including
initial child contexts), dispatches trusted native events in tree order and
rechecks document generation/epoch/ownership between recipients. Removed child
documents are skipped; retired/replaced roots and dismissed prompts veto the
navigation. RustExecutableDocument exposes top and selected-frame wrappers for
runtime navigation integration. Parent owns commit ordering/cancellation at the
navigation runtime boundary. Added host regression sources for initial descendant
order and removal during a parent's beforeunload handler.

Simple dialogs additionally validate the native top Window and compare native
origin identities. Author `top` replacement cannot authorize a cross-origin
frame; opaque origins use native identity. Added cross-origin suppressed-return
regression source. Beforeunload retains its separate sandbox/sticky-activation
policy. Same-origin-domain relaxation via document.domain remains dependent on
that broader origin feature; current tuple/opaque origin model is explicit.
No build, test, generator or conformance execution has occurred in this batch.
