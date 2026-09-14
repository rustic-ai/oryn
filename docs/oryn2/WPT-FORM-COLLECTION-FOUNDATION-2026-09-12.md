# Native form collection membership foundation

Core conformance and the public form collection APIs remain incomplete. This
checkpoint adds the native membership queries needed by those bindings; it
does not expose form.elements or claim a new WPT pass.

`DomRuntime::built_in_form_controls` enumerates built-in listed controls by
native form ownership and tree order, rather than selecting form descendants.
It includes external associated controls, fieldsets, disabled/unnamed/hidden/
unchecked controls, and excludes image inputs. It does not cross document or
shadow-root boundaries. Each query observes current attributes and tree state.

`named_built_in_form_controls` returns all case-sensitive id/name matches in
membership order, with no ID precedence and no duplicate for an element whose
id and name both match. Empty names have no matches. The future binding layer
must still choose null, Element or a live RadioNodeList.

The membership target is the [HTML form elements collection](https://html.spec.whatwg.org/multipage/forms.html#dom-form-elements).
Form-associated custom elements remain unsupported by these explicitly
built-in-only queries and require native ElementInternals integration.

Verification: all 71 DOM unit tests passed, including six new tests covering
membership categories, reassociation, reorder/type changes, disconnected trees,
first duplicate-ID resolution, non-inherited form attributes, named matches,
dynamic nested forms, and open/closed shadow isolation. Targeted DOM clippy
with warnings denied, formatting and diff checks passed.

No worker rebuild, signed WPT run or qualification promotion occurred in this
checkpoint. The current signed worker therefore predates these helpers.

Required next work, not satisfied by this foundation:

1. Add HTMLFormControlsCollection and RadioNodeList native brands/prototypes,
   inherited method receiver checks and generated WebIDL definitions.
2. Integrate live indexing, named access, supported property names and wrapper/
   owner retention with the native collection store.
3. Implement RadioNodeList.value and SameObject form.elements, plus form.length.
4. Cover GC, mutation, detached/shadow trees, duplicate names, named property
   visibility, conversion and radio value semantics in binding regressions.
5. Build/sign and rerun the broader forms WPT selection, preserving failures
   and qualifying only current, complete evidence.

The full Core completion gates remain unchanged; live collection exposure is
the immediate unfinished task, not a replacement objective.
