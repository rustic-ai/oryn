# Native select algorithms checkpoint

Core conformance remains incomplete. This extends native option state with
shared select algorithms, not a new WPT pass or a completed public Option API.

Implemented native option-list enumeration with tree-order traversal and
boundaries at nested select, option, datalist, hr, nested optgroup and shadow
roots. Unlike querySelectorAll(option), this models the specified list rather
than arbitrary descendants. Native option text strips/collapses only ASCII
whitespace, skips HTML/SVG script subtrees and preserves UTF-16 units. Value
attributes, including empty values, override text fallback.

Explicit normalization implements last-selected-wins for a non-multiple select,
and eligible-first fallback when display size is one and selection is empty.
Option disability includes optgroup disability, not disabled-fieldset rules.
Multiple selection is preserved. Native value/index setters do not normalize
away deliberate empty selection; the value setter chooses only the first match.
Setters change selectedness without rewriting default-selected attributes.
Reset restores defaults and clears dirtiness before normalization.

Algorithms follow the [select definition](https://html.spec.whatwg.org/multipage/form-elements.html#the-select-element).
Four new tests exercise list pruning and order, disabled fallback, explicit
empty selection, display-size/multiple differences, reset, duplicate values,
surrogates, attribute precedence, text whitespace and script exclusion.

All 78 DOM tests passed. The full V8 suite passed 215 tests with three existing
ignores. DOM clippy passed with warnings denied after replacing a full reverse
search with rfind. No worker rebuild or signed WPT promotion occurred.

Remaining required integration:

- Trigger normalization at parser initialization, specified attribute/state
  changes and option nearest-select insertion/removal transitions. Do not run it
  indiscriminately from getters or unrelated mutations, which would destroy
  an explicitly empty selection.
- Wire selected/value/index/reset, form-data and selector consumers to native
  state; current binding consumers still use the older attribute algorithms.
- Complete the Option factory and live options collection and validate through
  the full constraints and existing regression cohorts on a frozen signed build.

The preceding option-state checklist remains active. These methods are native
building blocks; their isolated tests do not prove browser-level integration.
