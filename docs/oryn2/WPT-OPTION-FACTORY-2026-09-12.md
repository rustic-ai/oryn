# Native Option factory checkpoint

Core conformance remains incomplete. The Option factory is now exposed in source,
but no signed constraints rerun or qualification promotion has occurred.

The factory constructs a native option with independent default/current selection,
preserves DOMString code units and conversion exceptions, treats omitted/undefined
arguments correctly, and converts boolean arguments without invoking object
coercion. It requires new, has length zero and a read-only/non-enumerable/
non-configurable prototype property equal to HTMLOptionElement.prototype.
Same-realm subclass prototypes are preserved.

A regression caught ordinary V8 FunctionTemplate construction reading
newTarget.prototype before WebIDL argument conversion. A native construct trap
now defers that read until after conversion and propagates prototype getter
exceptions. Missing argument positions are not read through Array.prototype.
The trap shares the existing native factory dispatch callback; it does not add
a new external-reference callback entry. Snapshot compatibility still requires
the forthcoming signed build verification.

Tests cover defaults, empty nodes/attributes, null and undefined, UTF-16 lone
surrogates, selectedness/dirtiness, coercion ordering, exceptions, descriptors,
subclassing, and side-effecting/throwing newTarget.prototype getters. The full
binding suite passes 217 tests with three existing ignores; targeted JS clippy
with warnings denied and diff checks pass.

Unfinished factory edge: when newTarget.prototype is not an object, fallback
currently uses the factory realm's native option prototype. WebIDL requires the
newTarget realm's interface prototype; cross-realm and bound/proxied newTarget
fallback needs explicit implementation and tests. No full realm-conformance
claim is made.

The source of constraints/support/validator.js shows each select setup calling
ele.add(new Option(...)). Therefore the missing select.add API is the next
prerequisite after Option; the six historical aborted files must not be reported
as repaired solely because the constructor exists. Native live options collection,
add/remove/length/indexed behavior and option text/form/index remain unfinished.

Next verification must include the complete constraints cohort and broader
option/select tests on a frozen signed worker, with all prerequisites and
subtest growth audited. Prior evidence remains immutable.
