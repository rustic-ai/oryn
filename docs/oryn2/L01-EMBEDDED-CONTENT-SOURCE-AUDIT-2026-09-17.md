# L-01 embedded resource source audit

Status: active source generation; no builds, tests, WPT or generators executed.

The pinned `html/infrastructure/urls/resolving-urls/query-encoding/resources/resolve-url.js` requires frame/iframe/object/embed SVG child Documents and object/embed raster loads. The initial audit found iframe-only creation and an independent object-image request route, which could not decide representation from the actual response MIME.

## Generated integration under independent review

- Native `EmbeddedContentKind` defines source attribute, destination and immediate blank eligibility. Parser discovery, snapshot discovery, dynamic mutation, host guards, child order and response ownership use it.
- Existing page-resource and standalone frame loading now accept object/embed destinations with object-src policy, object request cookies, no-cors Fetch Metadata, redirects, cancellation and response MIME. Actual child Documents inherit the existing CSP/embedding/COEP/permissions/worker policy custody; raster bytes do not allocate a child policy.
- One response owner selects SVG/XML/HTML Document or raster representation. The old ObjectImageStart/ObjectImageStreamStart route was removed. Binary response publication retains the ready response on preparation failure; native event publication owns callback retries after admission.
- Representation changes retire the old child navigable, preserve native response pixels without another request, clear obsolete pixels on Document commit and retain trusted load/error event delivery. Native `embedded_document_active` / `LayoutNode.document_replaced` distinguishes Document replacement from actual image intrinsics, suppressing object fallback without inventing an intrinsic ratio.
- Media ancestors suppress embedded requests. Descendants of resource-bearing objects remain inactive until fallback is selected; empty objects expose fallback directly.
- Added Frame/contentDocument/getSVGDocument IDL and native consumers. IDL generation remains deferred to the final generation/build gate.

## Generated regression sources (unrun)

DOM kind/discovery and ancestor/fallback admission; native one-request-per-resource raster/SVG representation switching; object-src versus frame-src with HTTP failure activating only fallback children; layout default Document sizing without a fabricated ratio; existing URL-encoding producer matrices, and updated binding tests ensuring object resources no longer enter the image request path.

## Remaining source reconciliation

Declared type and scheme cases; object name/contentWindow semantics; empty embed rendering; SVG natural dimensions and missing-component/default-size obligations; static/parser source routes; removal/stale completion and publication allocation fault coverage; layout/paint consumer consistency. These remain active implementation work, not a qualification-only waiver. WebVTT helper closure follows this group. Full L-01 and whole Core/five-family qualification remain open.

## Independent-review corrections (source generated, unrun)

- Embedded image/fallback publication now queues its event until a regular checkpoint, so native old-realm cleanup completes before authored callbacks can create newer ownership.
- Existing object/embed child-Window navigation uses the Document/frame destination, including response policy installation and raster image Documents. Attribute replacement continues to use object/embed representation selection. Transport failures preserve the existing child and settle the exact execution observer/joint-history failure; NoCommit uses the ordinary retained-incumbent navigation handler.
- Buffered raster child Documents seed their generated native image from the admitted response bytes before image preparation, avoiding a second request. Later src changes still use ordinary image selection. Added one-request/natural-width source assertions.
- Empty or failed embed representations have no formatting box or paint, including authored backgrounds and descendant top-layer projection. Added native projection regression.
- Added activation-failure injection and source regression preserving the same failed embedded completion and current identity at the queue front, followed by one retry/error event and no refetch. Root separately repaired the activation/requeue guard and generic ready-frame preparation hook.

Remaining items above still include SVG intrinsic metadata, declared type/scheme/static routing, and broader cancellation/publication source review; these corrections do not close the full group.

### Intrinsic sizing continuation

Native ImageIntrinsics now retains optional SVG width/height/ratio independently of concrete fallback dimensions. Image resources and committed SVG child Documents publish this descriptor; layout uses its actual natural ratio rather than inferring a ratio from 300×150 defaults. Child parser lifecycle and checkpoints refresh embedded SVG root dimensions before load publication. Added absent-width/height/ratio, invalid numeric values, CSS sizing and child root mutation regression sources. Full SVG CSS-derived root sizing and synchronous mutation/read reconciliation remain to review; this bounded source integration does not claim those algorithms are complete.

### CSS intrinsic and static-route continuation (source only)

- Native hosted SVG root identity now reaches LayoutNode separately from authored fallback children. Screen/subtree/print projection reads Stylo's computed root width/height before used percentage resolution; percentages/auto remain absent natural dimensions and viewBox supplies the preserved ratio. SVG root CSS changes therefore affect synchronous parent geometry without feeding the parent used viewport back into natural sizing. Generated regression sources cover absolute CSS overrides, percentage/auto dimensions and print parity.
- Both object and embed Document/image replacement suppress their hosted/fallback formatting descendants. Existing optional intrinsic metadata remains distinct from concrete defaults.
- Static/snapshot object/embed discoveries now enter the same single response representation selector. Registered navigation protocol handlers are restricted to actual frame Document navigation. Standalone Blob response capture carries strict native creator origin, policy and bounded source reservation; revoked URLs fail without a network request. Native frame lifetime checks retire embedded descendants suppressed by an outer representation.
- Added static blank/blob/revocation/name and dynamic representation/navigation/cancellation source fixtures. All are unrun. Full declared-type and source/cancellation reconciliation remains open rather than inferred from these examples.

WebVTT's native loader/API dependency is tracked separately in `L01-WEBVTT-SOURCE-AUDIT-2026-09-17.md`.

## Declared type, source and cancellation reconciliation — September 18

This section supersedes the earlier open **source-generation** items for declared type/scheme/static routing and cancellation. All validation and conformance execution remain pending.

### Applicability and authority

The frozen cohort contains nine distinct Core query-encoding entry paths (including `attributes.sub.html`, `lone-surrogates.sub.html`, UTF-8/UTF-16 and Windows legacy-label matrices), whose `resources/resolve-url.js` enumerates frame/iframe/object/embed SVG and raster loads. The object-element and embed-element directories each contain sixteen distinct persisted `necessary` entries; those are not mislabeled as Core or discarded as unsupported. Normative reconciliation uses HTML `iframe-embed-object.html`, especially `#object-type-detection` and `#the-embed-element-setup-steps`, downloaded to `/tmp/oryn-embedded-elements-spec.html`.

Object unsupported declared-type rejection before fetch is explicitly optional (“may jump ... fallback without fetching”). The implementation continues fetching to examine real response metadata. Declared type is required when metadata is absent, and in the binary response branch only declared non-XML image types may select an image handler. Embed supported declaration selection precedes response metadata; no filename-pattern plugin handler is invented. The existing native document/image/media handlers determine support.

### Production source closure

- New `embedded_resource_type.rs` selects object/embed representation separately from supplied response headers. Parent streamed HTTP and data/blob responses and portable standalone responses use it. Text/plain binary discrimination is retained independently of charset parameters; original response metadata remains available for policy, timing and decoding provenance.
- `ResourceInitiator.embedded_type` is an optional defaulted wire field. Parser owner attributes, static snapshot discoveries and dynamic preparations capture it once. Ordinary child-Window navigation clears it, as do CSS subresource copies. Source/response byte accounting includes the captured string. Owner fingerprints still include type, so an old completion cannot publish after a type mutation.
- Object/embed requests now carry the specified `navigate` mode with their own destination and include credentials. COEP credentialless no-cors omission no longer applies to document navigation; separate response/frame COEP checks are retained. Actual no-cors images continue omitting credentials under credentialless.
- Object HTTP failure remains an error. Embed non-2xx responses can still select a native handler. Successful fetches with no handler return typed `EmbeddedResourceUnavailable`, rather than being misclassified as a network error. This survives parent wire conversion and parser retained results.
- `NativeEmbeddedNotification` separates notification from image/representation success: object network failure emits error; embed network failure emits load; unsupported-handler or malformed-image fallback does not manufacture a network-error event. Retained publication still runs after native old-child retirement and does not recapture responses on allocation retry.
- Parser lane's `EmbeddedContentKind::loads_source` distinguishes missing source, exact empty object data, whitespace object data, and present-empty embed src. HTML parser, snapshot and dynamic source producers share it; media/outer-object fallback suppression remains native.
- Static `BTreeMap<String,String>` resources are explicitly **already-decoded HTML fixture sources**, without HTTP status, Content-Type or binary-body provenance. That existing API does not claim real-response MIME conformance. No headers or binary metadata are fabricated. Real transport selection and all new declared-type/status tests use actual broker responses.
- HTTP/data/blob and unsupported schemes retain the same creator/CSP/mixed-content/admission route. Source removal, adoption, connection generation, navigation token, stream epoch and type mutation invalidate pending ownership. Dropping ready responses releases their original body/policy leases. Per-resource cancellation precedes stale publication.
- Parent frame cleanup now attaches the known request identity before transport start. The shared `network_identity::IdentityBroker::start` error branch retains the native identity in the existing cancellation-ACK retry worker, fixing registered-then-error leakage. No-entry/already-finished cancellation remains idempotent; route/native identity stays retained until acknowledgement.

### Added regression sources (unrun)

`object_declared_type_is_conditional_on_response_metadata`; `embed_supported_declaration_precedes_transport_type_without_url_suffix_plugins`; `embedded_type_wire_defaults_and_selected_metadata_remain_distinct`; `declared_embedded_type_survives_capture_and_does_not_replace_response_headers`; `empty_and_whitespace_sources_keep_element_specific_semantics`; `embedded_network_failure_notifications_are_distinct_from_handler_fallback`; updated activation-failure/no-refetch fixture; `credentialless_navigation_keeps_credentials_without_reclassifying_as_cors`; `document_destinations_keep_include_while_no_cors_images_omit`; and `rejected_start_keeps_registered_native_identity_until_cancel_ack`.

Earlier generated SVG CSS sizing, static/blob revocation, representation switching, fallback descendants, cancellation and callback-retry sources remain part of the matrix. There is no known remaining source obligation in this assigned embedded declared-type/source/cancellation scope after this reconciliation. Build/test/WPT execution and observed conformance are not claimed.
