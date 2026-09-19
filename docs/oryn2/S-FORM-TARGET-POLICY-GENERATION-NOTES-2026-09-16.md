# Form participation, target and relationship generation — September 16

Source generation and read-only review only. No build, formatter, project generator,
test or conformance execution. Context protocol is now 45, coordinated with the
script-load typed-failure work; worker and agent protocols are independent.

## Generated source with callers and regression sources

- FACE participation uses native live validity flags and regular `:valid`/`:invalid`.
  Spec reconciliation corrected an earlier overreach: `:user-valid`/`:user-invalid`
  apply only to input/textarea/select, not FACE hosts. `element_user_validity` and
  submission marking now enforce that boundary. Regression:
  `custom_controls_validate_without_builtin_user_validity_selectors`.
- `SubmissionTransaction.noopener` snapshots native form relationship tokens after
  FormData callbacks. `_blank` defaults to no opener; explicit opener opts back in;
  noreferrer/noopener override opener. Regression:
  `native_rel_tokens_preserve_blank_default_and_explicit_opener_precedence`.
- `apply_default_navigation` carries relationship policy into typed `OpenPage`.
  Native live frame target names replace arena-wide name/id matching: changes via
  child `window.name` are honored, detached frame bindings are excluded, and a
  missing named target no longer silently navigates the source document.
- Popup admission separates creator authority from the opener relationship.
  No-opener pages are registered with no opener edge and a fresh browsing group
  from the start, while inheriting applicable creator policy and origin. Their
  sessionStorage is not copied. Named popup identity is published atomically with
  initial page registration, before an opened event can expose the new page.
- Parent target-name registry is part of the live page registry; lookups require
  the same browsing-context group, and updates validate exact native root
  generation/epoch. Target lookup precedes creation-only noopener; a newly isolated page preserves its nonblank name. Regression:
  `native_names_are_group_scoped_and_stale_document_cannot_rename`.
- Native Window name accessors read/write parent state for roots and shared native
  target state for child frames. Author replacement of the public property does
  not replace native target authority. The previously unreferenced native opener
  setter is now installed alongside its getter; actual `window.opener=null`
  reaches parent disown. Access checks precede name/set-opener callbacks.
  Regression: `author_name_and_null_opener_assignment_reach_parent_native_registry`.
- Regression `noopener_popup_has_fresh_group_and_no_opener_from_initial_registration`
  verifies native descriptor/group/opener-target state and stale creator rejection.

## S-03 producer → consumer → parent map

| Contract | Native producer / caller | Consumer / parent | Source assessment |
|---|---|---|---|
| Built-in/FACE entries | `form_data::construct_entry_list`; `custom_elements::submission_entries` | `submissions::queue` | Snapshot construction, disabled/name filtering and custom value snapshots exist; regression sources in form_data/custom_elements |
| Native file input | `MaterializeUpload` parent label grant → selected native bytes; `file_lists`/native drag | FormData Blob/File records → `form_data_body_encoded` | No generic filesystem path in context request; selected bytes survive body construction |
| Legacy encoding/body | `submissions::queue`, `name_value_pairs`, `form_data_body_encoded` | `navigation::PostResource` | GET URL encoding, text/plain and multipart bytes generated after callbacks |
| Validation/reentrancy | `validation::request_submit`, `form_activation::finish` | `SubmissionTransaction` queue | Invalid and submit callbacks, rooted identity, queue bound/supersession; source controls exist |
| Deferred default action | `PendingDefaultAction::Submit` | `apply_default_navigation` | Realm/generation/document/form connectivity rechecked before dispatch |
| Target and rel | native target/base/formtarget + transaction noopener | native live frame target or `OpenPage` | Current relationship/name changes described above |
| Root POST/reload/redirect | PageState resource navigation | `AuthorizeFormSubmission` / network broker | Signed original-source policy is retained across initial request, redirects and history resource replay |
| Frame POST/redirect | `FrameNavigationContext` → `LoadPageResource` | `load_page_resource` validates PostResource and reauthorizes redirects | Parent body checks plus signed original-source policy before initial fetch and each redirect |
| Popup POST | `OpenPage` native Creator, PostResource | parent authorization then page navigation | Exact Creator at admission; parent authenticates and rebinds original-source policy to destination page |

## Original-source policy and native navigation permissions generated

- `FormNavigationAuthority` carries a size-bounded sealed immutable native snapshot.
  Parent `issue_form_authority` validates the exact source Creator and captures its
  origin/CSP; HMAC authenticates context, source and destination. Only parent
  `retarget_form_authority` can bind an accepted snapshot to another page. This is
  policy provenance; consequential action approval remains separately enforced.
- `apply_default_navigation` issues the snapshot once; `LocationNavigation`,
  `FrameNavigationContext`, `OpenPage`, `LoadPageResource`, `GotoResource`, parent
  queued navigation and history resource replay retain it. Initial GET/POST and
  redirects use original source form-action and sandbox policy, including after
  accepted source retirement and POST-to-GET redirects. Worker protocol is 14;
  context protocol 45 includes typed policy requests and OpenPage fields.
- Regressions: `frame_source_policy_is_frozen_authenticated_and_rebound_only_by_parent`
  and the HMAC known-vector source cover exact frame policy, destination/context
  mismatch, altered signature, parent rebinding and retired-source issuance.
- `native_target_navigation_allowed` checks exact source/target documents, native
  ancestry, sandbox top-navigation flags and transient activation. Related top
  targets use `native_top_target_navigation_allowed`; new sandboxed auxiliary
  pages capture their one permitted navigator only when an opener exists.
  `sandbox_navigation_uses_native_ancestry_activation_and_permitted_auxiliary`
  covers self, descendant, non-top ancestor, activation-gated top, denied sibling,
  permitted auxiliary and stale source.
- `native_named_navigation_realm` now searches current subtree then top subtree in
  native tree order, including named siblings/ancestors/nested frames. Dispatch
  schedules the selected native realm through the same target authorization path.
  Detached frames and author-overridden properties do not supply authority.
  Regression: `native_named_navigation_searches_current_then_top_live_subtrees`.
- `PageOpened.created` prevents a reused named page from generating a false
  page-created effect; new no-opener effects contain null opener identity.

## Source fetch metadata, remote descendants and popup isolation generated

- Sealed `FormNavigationSource` now retains original native current URL, effective
  binding referrer policy, top-level cookie site, source/ancestor site chain and
  opaque origin. Native history URLs are checked against the committed origin;
  caller-supplied foreign origins are rejected. `IssueFormNavigationAuthority`
  obtains current URL/referrer state through native bindings, never public getters.
- `CheckFormNavigationPolicy` returns authenticated metadata. Root and frame
  network requests consume it after destination changes; redirect responses update
  referrer policy, stripped referrers stay stripped, and cross-site redirect state
  survives returning to the first site. Explicit opaque cookie sites no longer
  become an implicit same-site default. Source metadata remains valid for already
  accepted navigation after source retirement.
- Regressions: `frame_source_policy_is_frozen_authenticated_and_rebound_only_by_parent`
  includes native history URL/foreign-origin rejection and metadata rebinding;
  `frame_form_fetch_keeps_original_referrer_cookie_source_across_destination_and_redirect`
  observes actual broker requests with forged initiator input, retired source,
  referrer-policy redirect and source cookie metadata;
  `opaque_or_cross_site_form_source_does_not_become_destination_same_site`.
- `named_familiar_navigation_target` now searches native inventories on other live
  related pages in stable page/tree order, including descendant frames. Familiarity
  uses native origins and ancestry; sandbox checks use exact source/target Creator.
  Inventories have a 1 MiB bound and a shared five-second lookup deadline; an
  incomplete inventory is an explicit error, not a fabricated missing-name result.
- `NamedInventory` and `NavigateFormWindow` are typed worker operations. Parent
  `queue_target_resource_navigation` retains target snapshot, body, policy and
  metadata custody. The receiving runtime validates the exact original target,
  waits for committed frame identity and returns an explicit failure/timeout.
  Terminal outcomes include the target frame token and committed generation.
- Custody supersession is now per target: independent frame requests coexist;
  a root navigation supersedes pending descendant requests. Source regressions:
  `frame_supersession_is_target_specific_and_root_supersedes_descendants` and
  `remote_named_frame_navigation_has_exact_target_and_terminal_custody`.
- `native_popup_relationship_policy` enforces cross-origin nested creation under
  native same-origin COOP (no opener and blank name) and native blob-entry origin
  versus top-level site. It never derives blob authority from an authored URL's
  apparent origin. `PageOpened` reports actual opener identity for effect output.
  Regression: `popup_relationship_policy_uses_native_coop_and_blob_origin`.

## Remaining integration and validation obligations

1. General `LoadPageResource` target-embedder frame-src authority still uses the
   page-level resource-policy path; original **form-action** source authority is
   corrected above. This broader frame resource caller reconciliation is recorded
   with the P/R owner, alongside native frame-origin publication.
2. Reconcile history/reload treatment of retained form provenance and the complete
   S-03 manifest/recovery/receipt mapping with R/G. Accepted provenance retention is
   generated; full history policy semantics and receipt consumption are not proved.
3. In-flight navigation supersession, COOP transition ordering and document commit
   task ordering remain under R. Named inventory has bounded failure behavior;
   it is not proof of the entire concurrent browsing-context lifecycle.
4. All builds, regression execution and WPT validation remain gated on completing
   the full source generation pass. No whole S/R/P/V package is claimed complete.

## Parent queued-navigation custody integration — root source generation

The former detached per-navigation thread and ignored goto_resource result are
replaced by queued_navigation process-owned fixed lanes and page-owned tickets.
Admission is bounded globally/per page; context, POST body and authenticated form
authority survive until completion. A retired or already-superseded queued ticket
skips execution. Terminal generation/error is retained in bounded native history
with an explicit pruned count, exposed through queued_navigation_status. Close and
crash seal admission; close drains outstanding jobs and reports incomplete custody.

Generated source regressions cover retirement, captured resource release on
rejected submission, admission/history caps, queued supersession, and actual
queued native success/failure/closed admission. In-flight supersession and commit
ordering still need full R reconciliation; worker-start checks alone are not that
proof. External automation consumption of native outcome history also remains
under G review. No builds/tests/formatters/generators ran.

Queued-navigation custody also retains a process-wide 64 MiB reservation for URL,
body, content type, referrer and sealed authority bytes across queued/running work.
Terminal errors are bounded to 4,096 UTF-8 bytes with error_truncated; per-page
history has 32 records with pruned_outcomes. Overflow cannot silently claim full
evidence. Added oversized-admission and UTF-8 truncation regression sources.

## Final source reconciliation additions (unexecuted)

- Unnamed required radio controls validate independently; readonly still bars
  validation even where the input editing UI does not apply readonly, matching
  upstream constraint-validation WPT applicability. Regression:
  `readonly_bars_even_noneditable_inputs_and_unnamed_required_radio_validates`.
- Fieldset validity excludes shadow descendants while including participating
  FACE hosts (`fieldset_validity_excludes_shadow_controls_but_includes_custom_host`).
- FACE individual validity flags, native disabled-fieldset state, and accessibility
  disabled projection reach OIL through `apply_native_control_state`;
  `custom_validity_flags_and_fieldset_disability_reach_oil_and_accessibility`.
- Read-only frame operations recheck exact scope after the final page-task
  checkpoint. The remote named-frame regression also checks stale observation and
  extraction rejection after replacement; accepted action receipts stay separate.
- Native number editing and media/stylesheet applicability are detailed in the V
  notes. Target-embedder frame-src/current Creator reconciliation is owned by P/W.
