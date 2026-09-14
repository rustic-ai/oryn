# Select lifecycle and consumer integration

Core conformance remains incomplete. This source checkpoint wires the preceding
native option/select algorithms into browser state; no signed WPT promotion is
claimed and the public Option factory remains missing.

Parser initialization now normalizes selects. DOM append, insert and remove
capture option nearest-select ownership before mutation and normalize affected
old/new selects after ownership changes. Same-owner moves and unrelated mutations
do not normalize an explicitly empty selection. No persistent ownership cache or
extra native-root retention is introduced. Relevant size/multiple and non-dirty
selected attribute changes trigger normalization. Option selected updates dirty
state, deselect peers when required, and run the owning select's reset request.

Bindings for option selected/value and select value/selectedIndex now use native
state. Explicit empty selection remains observable. Form reset restores native
option defaults. FormData enumerates form-owned controls, emits every selected,
enabled option in list order, and omits empty selections and disabled options.
DOM and layout selector matching use native option selectedness for :checked.
This does not claim full successful-controls or validation conformance.

Regressions cover moving options between disconnected selects, single/multiple
transitions, attribute-triggered selection, default/current state separation,
unrelated mutation, same-owner reorder, first-value matching, :checked, external
form association, empty/multiple FormData entries, disabled options and reset.
Earlier isolated-state tests were adjusted to explicitly detach options before
testing dirty deselection without a select's automatic fallback; disabling an
already selected option is separately preserved rather than treated as deselection.

Verification: full binding suite 216 passed / three existing ignores; native
executable suite 158 passed / six existing ignores; targeted DOM/JS/native clippy
with warnings denied and diff checks passed. Native DOM tests cover 79 cases.

Required remaining work before claiming the constructor repair:

1. Native Option legacy factory with conversion order, defaults, UTF-16,
   exception, realm and prototype behavior.
2. Live HTMLOptionsCollection, select length/indexed access/add/remove and option
   text/index/form APIs; current select.options is still a sequence snapshot.
3. Broader parser/mutation/shadow/select regression coverage, including current
   option-list and display-size edge cases; renderer consumers beyond selector
   matching still need audit.
4. Frozen signed rebuild and complete constraints/regression reruns with current
   resource, automation and reference qualification. Prior evidence stays immutable.

Full Core and clean-source/security completion gates remain unchanged.
