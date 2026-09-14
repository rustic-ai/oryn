# Form boolean conversion checkpoint

Follow-up: [native form activation and signed evidence](WPT-FORM-ACTIVATION-2026-09-12.md).
The historical results and limitations below describe this earlier checkpoint.

Boolean HTML property setters now apply V8's JavaScript boolean conversion
before the generic string conversion path. Nonempty strings (including
`"false"`), numbers other than zero/NaN, symbols, big integers other than zero,
and objects are handled correctly. Object conversion does not call author
`toString` or `Symbol.toPrimitive` hooks.

The regression exercises defaultChecked, defaultSelected, disabled, readOnly,
required, multiple, open, and inert with truthy and falsy values, including an
object whose conversion hooks throw. The shared checked/selected setter path
also receives boolean conversion, but its existing attribute-backed live-state
model is **not** corrected by this change.

Validation: 204 oryn-js tests passed, zero failed, three existing tests ignored
(serial V8 execution using the pinned local sandbox archive and bindings).
Formatting and diff whitespace checks passed. No signed binary was rebuilt and
no WPT evidence was generated or requalified for this checkpoint.

Remaining form work includes native live checkedness and dirty flags, radio
group membership, preactivation and cancellation restoration, label forwarding,
form reset, and consistent FormData/selector consumers. The intermittent
Chromium reference-focus issue and deferred Range.insertNode crash also remain
unresolved. This checkpoint does not establish Core conformance; the scope in
[the completion gates](WPT-CORE-COMPLETION-GATES.md) is unchanged.
