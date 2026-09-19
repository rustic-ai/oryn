# Form submission ownership source audit

Source generation only. No build, test or WPT execution.

## Algorithm evidence

Reviewed [HTML §4.10.22.3](https://html.spec.whatwg.org/multipage/form-control-infrastructure.html#form-submission-algorithm) on September 16, 2026, alongside the pinned WPT form-submission algorithm and planned-navigation reparenting sources.

- The algorithm retains a form document before submission callbacks.
- After submit and formdata events, it rechecks whether the form can navigate. Moving to another live document is not itself cancellation.
- Parameter attributes and form-owner fallback belong to the current submitter; the action URL is parsed relative to its node document.
- The target navigable is chosen before the planned task executes. Later detachment must not discard an accepted navigation.
- The planned task navigates using the form's node document at task execution. This differs from retaining the original document for earlier algorithm steps.

## Generated integration

- Form operation guards use tracked leases across adoption and shared binding-state reentrancy checks.
- Invalid-control snapshots retain wrappers across callbacks and resolve current owners for dispatch and reporting.
- Click and requestSubmit use the same submission continuation and follow form/submitter wrappers after submit callbacks.
- FormData serialization retains the state that owns its captured entry list, while live parameter collection follows moved form and submitter identities.
- Submission acceptance captures existing root/frame target identity independently from source admission. Frame targets retain parent generation and connection generation; the native consumer follows a later document commit in the same frame, rejects removal/reinsertion, and does not repeat name resolution after acceptance.
- Weak planned-task receipts follow tracked form identity across binding-state queues and host extraction. Supersession, stop and retirement cancel receipts before execution; starting a task consumes its receipt exactly once.
- The native host retains extracted default-action suffixes across errors, preserves their order ahead of newly queued callback actions, rejects retired generations, and schedules bounded retries. Started actions are not replayed. Source regressions cover failure after the first action and reentrant cancellation of an extracted submission.
- Action URL parsing shares the hyperlink document-base/encoding implementation, including detached submitter documents. Dialog submission preserves the existing return value if callbacks change the submitter into a non-submit control.
- The original document's completely-loaded state is captured for history handling. Current-generation/stream load and pageshow completion sets the lifecycle bit; open/replacement resets it. Native same-source-target requests propagate replacement to both NavigateEvent and scheduled navigation. Early/late/other-target and lifecycle-retry regression sources are generated and the bounded caller chain is source-reviewed.
- Regression sources cover canceled and noncanceled adoption, recursive callbacks, out-and-back adoption, separately moved submitters, captured POST entries, and child-document targeting.

### Task-time source document integration

The transaction exposes the tracked form's current DOM/node identity. The native host resolves its current live document realm before issuing form authority, target navigation authorization, or popup creation. NavigateEvent source construction uses that realm's Window while retaining the accepted source-element wrapper and FormData object. The accepted URL/body/target and original-document history state remain frozen. Regression sources cover extraction followed by adoption into another live frame, unchanged target/payload, and explicit failure to resolve an inert document as the original live authority.

This does not finish source provenance: a form adopted into an inert document has no live realm in the current authority model. The consumer reports that missing authority instead of borrowing the original document's privileges. Supporting that document's own origin/policy/referrer provenance remains required before generation completion.

## Remaining generation review

- Complete inert/retired-document source provenance and its parent-issued authority model; live adopted-document realm selection is generated. Audit detached documents, sandbox/referrer policy, origin visibility, source retirement and original-document history state together before closing this requirement.
- Complete acceptance-time creation/selection for new or remotely named popup targets; existing local root/frame target selection and lifetime regression sources are generated.

These are open requirements, not conformance waivers. The broader form and runtime generation gates remain open.

### Source provenance architecture evidence

Source inspection on September 16 confirms `oryn_dom::DocumentMetadata` stores URL, inherited base, encoding and document mode, but no document-owned origin/policy snapshot. DOMParser imports an inert subtree and records URL; DOMImplementation document constructors likewise create metadata without source authority. Parent `issue_form_authority_with_metadata` authenticates a live Window creator before sealing the policy/fetch source. Consequently, resolving a renderer arena to a current live Window cannot implement inert/retired source semantics. The remaining generation must retain the document's own provenance and authenticate its use without substituting a later Window document.

The public HTML navigation algorithm (`browsing-the-web.html#navigate` and `#snapshotting-source-snapshot-params`, source retrieved September 16) snapshots source-document sandbox flags, policy container and fetch client, then applies allowed-to-navigate checks and captures source origin/base. This confirms the need for document-owned provenance alongside the existing accepted-task receipt.

### Constructor provenance reconciliation (continuing source generation)

Public DOM and HTML dynamic-markup specifications retrieved September 16, 2026 were inspected directly (`/tmp/oryn-dom-standard.html`, `/tmp/oryn-dynamic-markup-insertion.html`). `new Document` uses the current global's associated document origin; DOMImplementation constructors use their associated document origin; DOMParser uses its relevant document origin and URL. Document cloning copies origin and URL, but the DOM clone algorithm does not copy the active document's policy container or sandbox state. A new inert document must therefore not simply inherit an active document's sealed form-policy token. Its authenticated origin/relevant-settings provenance and its own fresh policy/sandbox state are distinct inputs.

The shared detached parser previously copied the current URL for both DOMParser and static Document.parseHTMLUnsafe. Source now distinguishes the two: DOMParser retains the relevant document URL; the static parser starts at about:blank. `detached_document_source_url_tests::static_html_parser_uses_blank_url_while_domparser_keeps_relevant_document_url` covers both and clone URLs. This is generated source only.

The authority integration remains open. The implementation must install authenticated active-document snapshots before a realm is exposed, refresh document-local policy/referrer changes, preserve the last snapshot through retirement, and issue separately derived inert-document capabilities at construction/clone time. Retirement cannot be made dependent on an unhandled fallible request. Parent validation must authenticate inert origin and relevant-settings identity without reusing active-document sandbox flags; target navigation checks and NavigateEvent source visibility must consume that document provenance rather than activating an unrelated live realm. Existing live-source lookup and missing-authority rejection do not satisfy these requirements.

### Document provenance implementation generated

The earlier missing-authority fallback has been replaced for parent-backed and standalone native runtimes. `DocumentMetadata.provenance` owns an immutable origin/key/policy capability. Parent document tokens use a separate HMAC domain from form/script load tokens, validate bounded wire data and exposed metadata against signed payloads, and derive fresh inert policy/referrer state while preserving origin and relevant fetch-client provenance. Tokens are capabilities within their originating BrowserContext: DOM migration may consume them from another page in that context, but another context cannot validate them. Form load tokens are issued for the actual destination/caller page while retaining the original signed source.

DOMParser, static HTML parsing, `new Document`, DOMImplementation constructors and Document cloning generate provenance before returning wrappers. Ordinary node adoption follows the destination Document metadata. Retirement preserves the last immutable document snapshot and the ability to derive an inert document from it; it does not issue a new live-Window request. Initial frame binding now performs native policy/origin inheritance before publishing contentWindow/contentDocument, and loaded frames capture provenance after their native response policy is bound.

Active URL/referrer changes refresh provenance with rollback on failure. Parent meta-CSP changes prepare and sign the new snapshot before committing the policy. Standalone policy changes preflight a signed immutable record; isolate-owned HMAC keys and CSP records implement the same constructor/clone/retirement paths without a parent RPC. Parent and standalone token domains are distinct. Parent meta-policy and local style-policy mirrors now compose instead of replacing one another.

Form task consumption gets provenance from the tracked form's current Document, independently of whether that Document has a live execution realm. Parent form policy and target checks consume the document capability. Native NavigateEvent source visibility compares document origin keys and retains original element/FormData wrappers, so a retired or inert source does not borrow a later Window's privileges. Standalone consumers validate their isolate-signed form policy. OpenPage accepts a typed document source, derives popup origin/initial policy from it, and strips document/form capabilities at the authored JSON bridge. Parent selection-receipt integration is coordinated with the root agent; named-target re-resolution is being replaced there.

Generated regression sources:

- Parent active-versus-inert CSP/referrer, retired source derivation, cross-context/metadata/capability tampering rejection, and metadata preflight failure.
- V8 constructor/clone native identity, authored origin spoofing, URL/referrer rollback and derivation after retirement.
- Standalone HMAC signer isolation, late inert adoption with accepted POST/URL custody and sourceElement visibility, and fresh inert policy versus original form-action restrictions.
- Static parser versus DOMParser URL and cloned URL preservation.

No build, test, formatting or WPT execution has occurred. These are generation/source-review claims only. Broad conformance and source review gates remain pending independent review and combined execution. Popup target-selection receipt completion is owned by the root agent and must be reviewed together with this source-provenance group.

## Named selection from retained Document provenance — September 17

Parent named-target lookup now separates live Window sources from authenticated immutable Document sources. Live lookup still resolves and rechecks the native creator; Document lookup authenticates its capability, uses its frozen origin for browsing-group familiarity, and applies Document-owned target policy. Retired/inert source selection no longer tries to revive the creator or borrow the current Window's origin. Both routes share local/remote inventory traversal and exact destination authority validation. Existing selected-link provenance regression source now covers retired named selection, live-source rejection after retirement, absent local targets and forged source origin. Source-only; independent navigation review and global execution remain pending.

## Local POST target admission — September 17

Source review found the local consumer's POST authorization was guarded by `destination_realm == self.top_realm`, leaving other local roots/frames to reach destination NavigateEvent before the source form policy check. Removed that root-only guard: every accepted local POST now checks its captured form authority before destination admission and before authority retargeting. Added `post_to_auxiliary_target_checks_source_policy_before_destination_event`, with form-action denial and a destination handler that would reveal premature admission. Source-only; not executed.
