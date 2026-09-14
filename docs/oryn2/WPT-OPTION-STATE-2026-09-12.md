# Native option state foundation

Core conformance remains incomplete. The current constraints baseline still
has six files aborting because Option is missing; this checkpoint does not
claim those aborts are resolved or promote new WPT evidence.

Native Node state now distinguishes option selectedness, the selected content
attribute and selectedness dirtiness. Initialization can override selectedness
without making it dirty; IDL/user-style updates mark it dirty. Reset restores
the default and clears dirtiness. Attribute mutation and Attr value updates
only change non-dirty selectedness. Clone and cross-arena import preserve both
state fields, including the paths used for recursive subtrees.

These distinctions are required by the
[Option factory algorithm](https://html.spec.whatwg.org/multipage/form-elements.html#the-option-element):
the third argument can set the default while the fourth independently sets
current selectedness. Reusing the existing attribute-only select implementation
would conflate those states.

Three native regressions cover clean/dirty state transitions, reset, parser
defaults, shallow/deep clone, import independence and Attr updates. All 74 DOM
tests and all 215 V8 binding tests passed (three existing V8 ignores). Targeted
DOM clippy with warnings denied, formatting and diff checks passed.

Unfinished integration, required before claiming the Option repair:

1. Native select option-list membership and single/multiple selection
   normalization after insertion/removal, attribute changes and selectedness
   changes, with disabled/default-selection and explicit-empty selection rules.
2. Wire option selected, select value/selectedIndex, reset, selectors and form
   data consumers to that shared state instead of selected attributes.
3. Implement native Option factory conversion, realm/prototype behavior and
   initialization, preserving DOMString units and conversion exceptions; expose
   the needed option text/value semantics and live options collection behavior.
4. Test integration, then rebuild/sign and rerun the entire constraints cohort
   and regression cohort without weakening source, completion or reference gates.

No signed rebuild was performed for this foundation. The current signed worker
and prior immutable WPT artifacts predate these source changes.
