# Native document.domain generation audit

Status: named domain implementation group and independent source review generated. Build, regression execution and full inventory qualification remain pending; no builds or tests run. See [independent source audit](DOCUMENT-DOMAIN-SOURCE-AUDIT-2026-09-17.md).

## Evidence and scope

The pinned HTML setter test `html/browsers/origin/relaxing-the-same-origin-restriction/document_domain_setter.html` requires cross-subdomain Window/Location access after both documents relax their domains. It separately requires `document.open()` to retain strict same-origin checks. It rejects public-suffix, unrelated-host and inactive-document assignments. The initial native Rust/IDL search found missing setter/effective-domain state. The implementation and caller integration below now address that finding.

[Source inventory](DOCUMENT-DOMAIN-SOURCE-INVENTORY-2026-09-17.json) records hashes for 91 Core and 9 Necessary source/helper files containing literal document.domain in the listed HTML/DOM/security/navigation search roots. These are not WPT manifest-entry counts. Filename-only references, aliases and transitive helper closure require reconciliation. Tier policy and pinned manifest are unchanged.

## Complete implementation group

- [x] Native Document domain IDL/getter/setter and document-owned relaxed-domain state; no authored property shim.
- [x] URL host parsing/canonicalization, suffix/public-suffix validation, opaque/IP/inactive/sandbox rejection and exact setter exception order, reconciled against pinned tests and specification.
- [x] Native origin-keyed-agent-cluster and permissions-policy behavior, including required ignored assignments/diagnostics rather than incorrectly relaxing authority.
- [x] Distinct same-origin-domain access checks for WindowProxy/Location/frameElement/contentDocument and native V8 security tokens; retain strict origin identity for network, storage, messaging and document.open.
- [x] Both-side opt-in, port behavior and unrelaxed-versus-relaxed access distinctions; prevent origin serialization from becoming the relaxed domain.
- [x] Lifecycle integration for initial documents, replacement/navigation, document.open, detached/retired frames, adoption/clone and BFCache; retain/reset only the state required by each algorithm.
- [x] Parent named-navigation familiarity and NavigateEvent gates that explicitly use origin-domain semantics; do not weaken ordinary origin-bound signed capabilities.
- [x] Production caller audit and regression sources for cross-subdomain access, one-sided assignment, rejection/ignored cases, strict-origin exceptions and lifetime transitions; remove superseded paths if any are found.

- [ ] Execute generated regressions, reconcile all manifest/helper entries and qualify complete WPT/five-family behavior after global code generation. Checked rows mean source generated and reviewed, not runtime verified.

Generation ownership: root handles V8 publication and restoration; the resource agent handles parent admission, inherited aliases, cluster readback and standalone admission. This group does not replace the existing Core/five-family scope. Final build/test/conformance remain behind the global generation gate.

## Generated foundation

DocumentMetadata now owns a GC-managed shared optional domain cell independently of signed provenance. Inert/clone Documents share it; native inherited owner installation joins context-qualified alias cells through weak host records. Document.domain IDL/getter dispatch reads that value or the document's native strict origin host; opaque origins return an empty value. Added getter regression source verifying that domain reads and relaxed state do not change the serialized origin, origin capability key or location URL. The validation/access-identity primitive, setter authorization, token publication, policy and lifecycle integration are generated. No execution.

## Setter and parent admission generation

The Document setter converts its value once, rejects inert/retired receivers, and routes active documents through a native authorizer. The host prepares parent admission, allocates/caches the V8 access token, commits against the expected previous domain, then publishes the receiver's V8 token and normalized Document metadata. Ignored assignments leave both unchanged. The parent group owns sandbox/permissions/OAC admission and separate access-domain records; strict Origin remains unchanged. Context49/agentIPC18 carry the new native protocol.

Added setter regression source for value conversion, ignored/denied assignments, inactive Documents and unchanged URL. Native context installers register the callback. Standalone admission, same-Document stream/token preservation, strict-origin consumer audit and cross-realm/lifecycle regression sources are generated. No builds or tests have run.

## Strict document replacement guard

Explicit document.open and implicit write-triggered opening now compare authenticated strict origin keys before clearing nodes/listeners or advancing the parser stream. Relaxed V8 access tokens cannot authorize replacement. Bare binding embedders without provenance retain same-realm and unrelaxed inherited-token handling; this fallback is disabled once either Document has relaxed. Added two-realm regression source granting shared DOM-access tokens while retaining different strict identities, requiring visible DOM edits but SecurityError and unchanged tree on document.open. No execution.

## Alias publication and history restoration

Parent preparation returns the exact active alias recipient roster. The host resolves and retains every recipient context and Document metadata, and allocates each domain string and the shared V8 access token before commit. Commit compares both the previous domain and recipient roster; successful publication updates every retained alias context.

History restoration refreshes authenticated domain state for the complete suspended root/descendant forest after parent authority restoration and before releasing microtask custody. A refresh failure retains the suspended forest and the restored parent marker for retry. This handles aliases whose native domain changed while their contexts were parked. Added native popup regression source that injects stale cached metadata into root and descendant Documents and requires both to be refreshed before the queued checkpoint. The real tuple-origin alias-change-while-parked regression is generated below. No execution.

The real tuple-origin regression source is now generated: a parent-backed popup relaxes their shared domain while the opener is parked, resolves an opener promise, then restoration must refresh access tokens and metadata before that promise observes document.domain. Routed endpoints remain in the same actor. Independent source review is recorded in the linked audit; execution remains pending.

## Startup cluster publication

Native domain synchronization now publishes the document's committed secure-context state before requesting the parent-owned cluster decision. Previously this publication happened only when particular Worker/platform APIs were used, leaving the initial tuple-origin getter without an admitted cluster. Root/frame creation and restoration now share the explicit publication path. Generated parent-backed startup regression source covers site-keyed relaxation and an origin-keyed document ignoring an invalid assignment before any Worker API use, while retaining strict provenance.

## Navigation event domain admission

NativeNavigationSource now carries a separate captured domain-access key. Link/form/native source capture derives it from native Document metadata and authenticated provenance; remote script-navigation source readback consumes the parent-signed domain key. Navigation event admission and source-element/form-data visibility compare this identity against the target's current domain identity. Strict origin keys remain separate; an unrelaxed strict key does not match a relaxed target even if the original tuple was identical.

Added regression source for bilateral cross-host/port access, one-sided rejection, scheme separation and unchanged strict provenance. Parent signed producers now capture relaxed state and access key from one authenticated native identity. Form owner changes refresh the domain identity when the owner remains represented by a native binding; retired owner sources retain frozen signed navigation identity independently of live domain metadata. Full manifest-level runtime qualification remains pending. Native originAgentCluster readback is now installed during initial/domain restoration synchronization. No build or test execution.

## Shared inert-domain lifetime follow-up

A previously created inert Document now observes later changes to its inherited origin-domain cell, including changes made by a live alias after creator retirement. Native alias keys contain BrowserContext identity and monotonic alias identity, so different contexts and unrelated same-origin responses cannot merge. Weak host records do not own cells; the last retained Document releases the state and later synchronization prunes expired entries. Standalone origin slots share the same cells before inherited documents are exposed.

Signed navigation snapshots stay immutable. Native root blank handoffs retain the authenticated alias identifier even when the previous active owner retired, while data/blob/replacement responses remain separate. Added source regressions cover pre-relaxation inert construction, parked restoration, creator retirement followed by further relaxation, clone visibility, separate default identities and last-owner reclamation. No execution.
