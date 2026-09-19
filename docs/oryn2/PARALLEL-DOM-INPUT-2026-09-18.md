# DOM, editing and form continuation generation — September 18

Source generation and inspection only. No tests, builds, formatters, binding
generators, service launches or conformance commands were run. Existing dirty
source was preserved. This record does not close S-01/S-03/S-04 or qualification.

## Generated corrections

| Contract | Production source and callers | Regression source |
| --- | --- | --- |
| Undo preserves author control state | `editing_inverse::Segment::apply` now treats exposed value, pending input, dirty state and user-edited state as one ownership group. An intervening flag change prevents value restoration; selection retains its separate ownership predicate. This is shared by execCommand, keyboard, clipboard, composition and drag segment histories. | `inverse_control_flags_and_pending_input_share_value_ownership` covers each flag conflict while preserving the current exposed value. |
| Undo cannot remove a wrapper containing author-modified controls | `Segment::compact` retains control witnesses for subtrees whose edges undo/redo can remove, including controls otherwise unchanged by the native segment. Opaque nontext control witnesses also cover checkedness/selectedness/user-validity/indeterminate and file-selection identity. Initial and resumed detach check control value/flags/pending input/selection and sensitivity as well as DOM topology/text/attributes. | `inverse_keeps_inserted_wrapper_when_unchanged_control_acquires_author_state`; existing subtree/retry/author-child tests remain controls. |
| Compacted inverse admission includes unfinished input | Retained pending-input strings and selection direction contribute to `serialized_bytes`; later history admission cannot omit those allocations. | `compact_inverse_charges_retained_pending_input_bytes`. |
| Script and activation form reset share one algorithm | `v8_bindings::reset_form` delegates to `form_activation::reset`, which uses `retained_form_reset::Reset`. The old duplicated one-shot body is removed. Existing activation scheduling, readiness and retirement now hold an activation/reset job variant. Admission reserves capacity before author callbacks. | `script_form_reset_retains_event_and_control_suffix_without_replay`, `cancelled_script_reset_releases_guard_and_never_applies_control_suffix`, existing retained reset event/guard test. |
| Callback retirement cannot revive work | Script activation, direct reset and queued default polling recheck original generation/stream and stopped/retired execution after callbacks before retaining another attempt. A current job removed before retirement cannot be requeued after retirement has drained the queue. | `callback_retirement_cannot_requeue_an_interrupted_reset`. |

## Caller reconciliation

| Family | Source route inspected | Disposition |
| --- | --- | --- |
| Authored rich editing | `editing_commands::run` → `editing_continuation::Continuation` → owned `editing_algorithms`/`editing_range` operations → native inverse segments | Existing source uses retained operations and author-boundary baselines. Snapshot closure mutation APIs in `retained_editing` are test-only. Full topology/history publication qualification remains open. |
| Trusted/control/rich input | Keyboard, semantic replacement, clipboard, composition, same/cross-document drag → `retained_editing::Transaction::poll_owned` or `Compound::poll_future` | Existing production callers use owned operations. Host/Actions integration belongs to the automation/root lanes; this pass does not replace their authority or receipts. |
| Form reset | Script `HTMLFormElement.reset`, script click default and trusted native click | All now share Reset's original event, reset guard and retained per-control cursor. Native controls reset before queued FACE reactions. |
| Submission/file/FACE | `submissions`, retained submit request, `form_data`, custom-element entries, parent authorized file selection | Existing September 17 FACE audit has complete generated file snapshots and persisted fixture receipts. Parent authority/file transport and end-to-end family acceptance remain with their lane owners. No second implementation was introduced. |
| HTML sanitizer/serialization | `sanitizer::{parse_document,set_html,set_legacy_html}`, shared native fragment policy/context, scoped registry initialization and retained replacement preparation | Existing shared callers were inspected. A concrete UTF-16 parser-input gap is recorded below; full sink row remains open. |

## Cross-lane dependencies and exact open work

1. **HTML DOMString parser input integrated:** parser lane supplied explicit
   inert document/fragment DOMString APIs with reversible tokenizer transport.
   Safe JS HTML conversion now preserves units; native sanitizer fragments no
   longer scalarize them. Document safe/unsafe and legacy/sanitized fragments
   use the new APIs. Rich and plaintext editing use them too; root integrated
   DOMParser and the shared text-control execCommand caller. The source matrix
   `html_sink_domstrings_preserve_surrogates_and_storage_prefix_text` covers
   document safe/unsafe, Element/ShadowRoot safe/unsafe/legacy, attribute/text
   units, supplementary characters, literal storage-prefix collisions and
   numeric character references that produce transport markers.
   `editing_html_domstrings_preserve_units_for_rich_plaintext_and_controls`
   covers all three editing host kinds. These sources and the parser transport
   remain unbuilt/unexecuted; parser lane owns complete native decoder review.
2. **Target sanitization:** retain the specified conjunction for newline/'<'
   checks. The three pure target/referrer helpers now classify decoded UTF-16
   units and preserve the original target storage; regression source covers
   conjunction negatives, lone surrogates and rel tokens.
3. **Inverse breadth:** `EditingNodeSnapshot` now also captures an opaque
   nontext control witness: checkedness/selectedness and their dirty flags,
   indeterminate state, user-validity and file-selection allocation identity.
   File evidence is weak and does not copy or retain uploaded bytes. Both
   initial and retry removal checks preserve controls that changed without
   authored attributes. `inverse_subtree_witness_preserves_checkbox_option_and_file_state`
   covers checkbox, option and actual selected file bytes. Remaining topology/
   reorder conflicts, history publication failure paths, custom-element
   internals stored outside the native Node, and cross-document history
   cancellation remain explicit source-review gates.
4. **Sanitizer breadth:** the pre-existing policy-modifier, registry and script
   scheduling matrices need combined parser/resource integration review. No
   new claim is made for full XML/DTD compatibility or Core manifest closure.
5. **Verification:** all authored regression sources must be built and executed
   only after the global generation gate, followed by the complete pinned Core
   and five-family candidate qualification.

## Continued source closure

### ElementInternals and author-owned state

`custom_elements::Element` now carries a replacement identity token for accepted
internals mutations. Editing snapshots keep only a weak token, so they neither
root callbacks nor retain submission File/FormData values. Attach, form value,
validity, changed custom states, changed ARIA defaults/references and attached
submit-behavior properties publish a new token. Native CE migration preserves
its identity. Subtree inverse removal compares the captured token before initial
and resumed detachment. Unchanged CustomStateSet/ARIA writes and rejected validity
conversion keep the token. `inverse_preserves_author_internals_without_rooting_form_values`
covers those positive and negative cases.

CustomStateSet string conversion and scalar ARIA conversion now retain the host
wrapper and resolve its current native owner after coercion. The two-arena
`internals_scalar_and_custom_state_conversion_follow_adopted_host` source covers
add/has/delete and default ARIA after callback adoption. Reference conversion now
uses the shared iterator consumer and retains original wrappers until conversion
finishes, then resolves the receiver and every target in their current arena.
Foreign targets remain private JS reference edges; native accessibility receives
only same-arena projections. Existing migration reconciliation exposes those
original targets when subsequent adoption brings them into scope. The same
regression source covers foreign assignment, target adoption, receiver adoption
between yielded targets, later target adoption and IteratorClose atomicity.

### Scoped OIL dismissal

The previous global close/cancel/unnamed-button search is removed. Named dismiss
commands resolve their actual native container. Generic popup/modal/dialog
commands inspect only close-control ancestors using the native role and computed
position/z-index/viewport coverage from the same style revision. Candidate clicks
must be visible, enabled, interactable descendants. Unnamed icon candidates must
be small and near an ancestor container's top-right corner. Frozen child/container
references are revalidated through the native Inside resolver before Click.
Completed receipts still pass through the shared follow-up observation handler.

Generated regression sources cover named competing containers, missing targets
without outside clicks, native small-overlay geometry, no-overlay rejection and
scoped unnamed icons. The existing real login-modal outcome regression remains
in place. This is source implementation, not a passing popup-fixture claim.

### Target policy correction

The earlier suspicion about `normalize_target` using conjunction was withdrawn:
the pinned dangling-markup cases require the existing conjunction. The source
review instead identified actual DOMString-storage classification risks for
newline/'<' target checks and rel token parsing. With navigation-lane ownership
confirmed, `normalize_target`, `target_noopener` and `link_referrer_policy` now
classify decoded UTF-16 units. The target name itself remains byte-for-byte
unchanged unless both required delimiter categories occur. The standalone
`target_and_rel_policy_classify_utf16_units_without_altering_names` source covers
surrogates, conjunction negatives, ASCII rel tokens and partial-token negatives.


### Native ARIA custody under failure

`set_accessibility_references` validates the receiver and targets before native
custody acquisition. Failure to retain a later target now releases the successful
prefix. Publication cleanup attempts every independent previous-target reclaim
and every protected-root release, preserving the first failure. Releases consume
each acquired root once because reclamation can fail after its accounting
committed. The JavaScript setter compares both native revision and association
when deciding whether to restore its private references after a native error;
committed errors still invalidate cached arrays and mark changed internals.

Generated native regression sources cover a second-target retain overflow with
unchanged association/revision and a postcommit release failure that still cleans
the independent suffix without replaying decrements. These are source fixtures;
no tests have run. All full-plan qualification gates remain open.


## Complete sink dependency reconciliation continuation

The former generic combined sink matrix is replaced by the exact hashed caller
maps in `S01-HTML-SINK-CALLER-MATRIX-2026-09-18.md` and
`S01-DECLARATIVE-STYLESHEET-CALLER-MATRIX-2026-09-18.md`. New Core obligations
identified there include all five safe/unsafe nonstreaming HTML insertion pairs,
streamed HTML, template-for patching and declarative adopted CSS modules.
Nonstreaming insertion, post-filter patch callers, customization filtering,
registry defaults, declarative stylesheet binding/provenance/placeholder and
native cleanup sources/regressions have been generated. Parser owns native
PI/template-for/incremental semantics; loading owns stream WritableStream and
typed CSS module adapter integration. No source-only row implies qualification.


### Final sink integration review

Loading has generated the shared CSS module adapter, parser preload admission,
module-map identity publication and async/failure/parked regression sources.
Clone/import/Range clones initialize declarative sheets before CE upgrade.
Active DSD roots now initialize their registry from their node Document; the
parser lane supplied a live scoped-host/global-shadow constructor fixture.
Template.buffer now has authored boolean IDL and presence-based reflection,
including coercion and parsed-template regression sources. Fragment-parent
sibling insertions disable DSD attachment on their synthetic parsing context;
all three sibling methods retain such templates as inert light nodes.
No builds, generators or tests have executed in this source generation lane.


### R04 target/name continuation

Confirmed and removed lossy generated/native Window.name, named property and
remote named-child boundaries. HTML target/name setters now retain their receiver
through conversion-time adoption. Replaced the obsolete author Window.open JSON
bootstrap with native conversion, parent-authorized typed selection and native
Window results. Exact caller/evidence map and regression inventory are in
`R04-TARGET-NAME-CALLER-MATRIX-2026-09-18.md`; all assertions remain unexecuted.

R04 pinned continuation also generated parent keyword ancestry and source-local
duplicate-name ordering, plus durable existing-frame opener publication and
remote/native getter integration across document replacement. Frame disown,
source/target destruction and page cleanup preserve exact native relationship
custody; retrying destruction retains the closed frame until native cleanup
acknowledges. Native Window.open is now the only author open producer; the old
OpenPage JSON capability and response projection were removed.
