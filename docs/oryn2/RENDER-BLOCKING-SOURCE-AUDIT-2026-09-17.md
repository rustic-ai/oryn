# Native render-blocking source integration

Generation in progress. No builds, tests, formatters, generators or services run.

## Current source reconciliation ledger

The dated sections below record successive generation passes; their earlier
“remaining” lists are superseded by this ledger and later named deliveries.
Nothing in this document is an executed conformance result.

| Requirement group | Current source state | Remaining obligation |
| --- | --- | --- |
| Resource/expect blocker leases and parser closure | Generated across native parser, dynamic script/style, removal/adoption and encoding rollback | Full pinned lifecycle caller reconciliation and later execution |
| Rendering admission | Generated resource/expect, BFCache reveal, prerender, suspension and native visibility composition | Complete reentrant frame transition coverage |
| Paint/observer APIs and scenes | Generated immutable entries, retained observer delivery, native root/child scene acceptance and ancestor-effect coverage | General occlusion/visual fixture reconciliation and later execution |
| Request render-blocking status | Generated immutable admission transport and stylesheet import/preload handoff | Protocol-wide final reconciliation and runtime checks |
| Classic/style preload | Generated Unix parent and Broker/Sources producers, root headers, retained single-use claims, policy proofs and rollback | Non-Unix producer integration; independent policy/claim review underway |
| Full L-02 gate | Open | Remaining caller generation, then build/test/fix/conformance phase |

## Confirmed pinned requirements

Pinned WPT `719d5e38fdd0903a18ed9007aba816c98cc491e0`, `html/dom/render-blocking/`:

- Parser-blocking classics and parser stylesheet links are implicit blockers.
- Explicit `blocking=render` covers parser async/defer/module and dynamically
  inserted external classics, inline modules with imports and stylesheet loads.
- Removal/adoption must release original-document blockers; removing an explicit
  attribute does not remove an independently implicit stylesheet blocker.
- `link rel=expect` has a same-document fragment target, loading-only eligibility,
  head/potential-blocking and media rules. Its target must be fully parsed;
  later removal of a satisfied target's id must not start blocking again.
- Rendering admission must compose resource/expect blockers, BFCache pageshow /
  pagereveal order, prerender suppression and document suspension/retirement.
- PerformancePaintTiming timestamps must come from actual admitted native paint
  milestones, after resource completion where required. Timer fabrication is not
  an implementation of this requirement.

## Generated source so far

- `blocking` SameObject / PutForwards DOMTokenList IDL on script/style/link.
- Shared private token-list attribute identity now handles rel/blocking without
  authored property authority, preserving existing associated wrapper ownership.
- Script obsolete charset/event/htmlFor reflection and classic for/event
  eligibility are generated in the script preparation audit.

## Remaining generation in this group

1. Document-owned blocker leases across script/style admission, completion,
   failure, cancellation, migration and parser encoding rollback.
2. Native closed-element parser metadata and rel=expect processing on insertion,
   removal, attribute changes, parser pop/readiness and media changes, including
   permanently satisfied previous target identities.
3. Shared document rendering admission composed with the source-stable
   animation_frames history-reveal gate and traversal's document_prerendering.
4. Actual paint milestone entry storage/IDL/query/observer exposure and original
   Document identity guards; native root/frame and standalone callers.
5. Regression sources for every lifecycle boundary above and pinned caller map.

This document is an inventory, not completion evidence. Broad L-02 stays open.

## Stylesheet custody and native expect follow-up (source generation only)

- Native resource blocker tokens use `Arc<AtomicBool>` so captured parser script records remain transferable to native fetch producers; a source regression asserts `Send + Sync`.
- Parser/root/frame stylesheet admission now retains an explicit native blocker per `(node, revision)` from discovery through sheet/import publication and load/error task admission. Dynamic insertions routed through a preparing frame use explicit blocking semantics; parser discovery uses implicit semantics. Pure inline completion, denied inline policy, empty source, obsolete revision, owner cancellation and queue destruction release custody. Frame stylesheet IO retains the existing shared page byte budget.
- Dynamic stylesheet jobs retain completed source and a publication marker while event admission is retried. They do not refetch or repeat an already accepted stylesheet publication when the networking event queue is full.
- Stylesheet networking tasks now retain the actual trusted event and listener suffix across preparation/callback failures. The superseded one-shot `dispatch_stylesheet_event` route was removed. Queue ownership includes original Document generation, stream and authority plus a tracked target/revision; after dispatch begins, adoption does not erase an accepted propagation suffix. Native scheduler readiness uses a 10 ms retry deadline, and suspension contributes no runnable work.
- Native expect selection now checks nonempty href, case-insensitive rel keywords, literal fragment before percent decoding, ordinary document-tree ID/HTML anchor-name selection, actual parser open-element membership and the satisfied-link latch. Generated source regressions cover closing the target, later ID mutation, empty href, media/foreign URLs, implicit-vs-explicit resource membership, release and token transport.

These are generated sources, not executed evidence. Remaining generation includes the actual native rendering opportunity / scene-publication producer, immutable PerformancePaintTiming entries and PerformanceObserver delivery, complete resource timing render-blocking status, further snapshot/standalone caller reconciliation and comprehensive rendering gate integration. The broad L-02/render gate remains open.

## Native presentation and Performance Timeline delivery (source generation only)

The earlier paint-producer/API TODO now has production source integration:

- Normal `poll_page_tasks` creates a headless native presentation at admitted rendering opportunities. It refreshes layout, assembles a scene from already available native resources, compiles the scene through `oryn_render_core::build_display_list`, retains the accepted scene/display list and then records paint timing. It does not fetch missing capture resources or report paint from a forced layout query. Original Document/realm/generation/stream/authority is checked around callback-capable layout work and before publication. Failures produce native rendering diagnostics and are retried at a later opportunity.
- The same producer traverses visible same-actor child-frame scopes through a private ready-scene mode. Existing capture requests retain their resource-waiting behavior. Native rendering gates are checked for each child, frame identity is revalidated and parent scope is restored.
- `PerformancePaintTiming` has native immutable `first-paint`/`first-contentful-paint` entries, inherited PerformanceEntry branding, filtering, sorted query snapshots and paint-specific JSON. Clearing resource timings leaves paint entries intact. Contentful publication currently recognizes actually available visible text/image scene content and excludes fully transparent ancestor text.
- `PerformanceObserver` / `PerformanceObserverEntryList` now have native constructor/IDL/brand, immutable batch lists, single-vs-multiple observation modes, buffered delivery, disconnect/takeRecords, filtering, supported type exposure and bounded FIFO callback delivery. Window supports the generated resource/paint types; Worker supports resource. Resource observation is independent of the public resource buffer capacity. Native preparation failure retains the batch and a retry deadline and surfaces a diagnostic; entered author callbacks are never replayed. Worker runtime already invokes the shared RawV8Host delivery loop, so it uses the same scheduler path.
- Stylesheet render tokens now transfer into the queued retained load/error task, so removing the producer does not release rendering before the accepted callback suffix completes. Legacy parser completion also retains already-published stylesheet state on event-admission failure. Disabled and nonmatching-media stylesheets do not acquire resource blockers.
- Regression sources cover actual normal-poll scene/timing generation without capture, document.open identity replacement, transparent ancestry, child-frame presentation, observer buffered snapshots/FIFO reentrancy/resource-buffer independence, stylesheet callback suffix/retry readiness and stale revision cancellation.

Still open for source reconciliation: complete resource timing `renderBlockingStatus` transport from captured request admission (it must not be inferred by matching URLs); broader FCP visual coverage (background image, clipping/mask and text visibility edge cases); full non-Unix/standalone entry-path matrix and native rendering gate behavior under reentrant frame transitions; comprehensive pinned-fixture requirement mapping. No generated sources in this section have been compiled or executed, and the broad L-02 acceptance gate remains open.

### Contentful paint refinement

Replaced the interim geometry/text-presence classifier with `oryn_render_core::scene_has_contentful_paint`. It retains actual content paint commands (including background images), preserves opacity/clip/mask/filter wrappers and uses the shared bounded renderer to check resulting content alpha. Solid backgrounds/borders and embedded child documents do not count as parent-document content. Input dimensions are validated against the renderer's pixel limit. Unchanged scenes avoid repeating publication/classification. Source regressions cover noncontent/transparent groups and preserved effect wrappers; runtime sources cover normal document and child-frame publication. This supersedes the preceding geometry-only FCP implementation note; broader pinned caller reconciliation and execution remain pending.


### Observer custody and request-status source review

PerformanceObserver callback preparation now allocates options before the native entry-list wrapper and removes failed wrapper records before retry. A generated regression injects preparation failure, proves no callback or orphan list publication, parks the original environment with its exact pending batch, and restores it for one delivery. Retry deadlines and diagnostics remain native; no test has run.

Ready-scene SVG capture was inspected through `capture_svg_paint_tree`: it creates bounded native SVG/foreignObject scene resources and records unresolved image descriptors; it does not invoke a resource request. Normal presentation consumes ready resources and never executes the capture image-waiting path.

Pinned `resource-timing/render-blocking-status-script.html` requires request-time **potential** render-blocking status, including a dynamic `blocking=render` script inserted into head after body exists. This differs from actual admission into the Document render-blocking set (which can already be closed by body creation). Consequently status must not be inferred from a retained active blocker, `parser_inserted` alone, or a response URL. Required remaining generation is an immutable request-status field carried through parser/dynamic script preparation, root-only module graph requests (dependencies remain nonblocking), stylesheet admission, parent `resource_timing_scope` and standalone `ExternalTiming`, into the native timing entry. Existing getter remains nonblocking until that complete producer chain is generated. Resource entry `toJSON()` now includes its existing `renderBlockingStatus` member; this does not close status semantics.


The DOM now exposes `potentially_render_blocking(node, implicit)` as a preparation-time predicate separate from active membership; head-parent, connectivity, media/disabled state, and explicit token checks are shared with actual admission. A generated regression covers head/body distinction and body-era closed admission. This helper alone does **not** implement Resource Timing status. Pinned link status fixtures further require inherited CSS import status and already-started preload consumer upgrades (`link-style-preload-used-immediately`, versus later use nonblocking). These request/consumer transitions must be generated with exact request custody, while module imports remain nonblocking.


### Stylesheet replacement publication review

A retained old stylesheet completion could previously look up the current blocker by NodeId after queue-admission retry, then release a newer stylesheet load's blocker on cancellation. Registry resources now carry the captured native stylesheet revision; event-task handoff upgrades the lease only for that exact revision. A generated regression replaces source text, admits the replacement, and proves releasing/canceling old publication cannot unblock it. No execution performed.


### Request status transport generation

Native `RunScriptLoad` and `StylesheetRequest` now carry a serde-default preparation snapshot. Parser classic/module and dynamic classic/module root requests capture native potential blocking; module descendants remain nonblocking. Parent resource observations and standalone external timing queues carry status separately from Timing-Allow-Origin-redacted response details. Inline stylesheet imports now receive stylesheet request metadata, and descendant CSS stylesheet requests inherit the root status while image/font requests remain nonblocking. Standalone dynamic classic loading also publishes its real captured timing metadata, which that branch previously omitted.

A native shared request-status cell permits a consumer to promote an in-flight shared fetch and atomically freezes status at response completion. Completed preload reuse cannot change an existing timeline entry. This is wired through shared module fetch joining, parent local observation and standalone module timing guards; it neither changes authentication nor exposes an authored authority field. Regression sources cover freeze/promotion and timing publication without response details/TAO.

**Still generation-open:** general classic-script/style preloading is absent (current parent single-use preload cache is image-only); complete preload producer/consumer joining must be added. Existing module shared-fetch cache keys include parser-inserted policy context, so joining across distinct preparation policies must separately preserve CSP admission rather than removing the key discriminator. Protocol reconciliation must include the new request metadata fields. No execution has occurred.

### General preload generation in progress

The parent-backed classic-script/style path now has a separate prepared-resource preload store, native producer, parser/dynamic link admission, retained link completion events, and loader consumer interception. Single-use claims use the full signed native preparation request (Document identity, policy, URL, request mode, integrity and decoder inputs), with fragment/CORS-mode normalization and timing flags excluded from identity. Producers keep their original native load ticket after link removal; claimed consumers remain cancellation targets on Document teardown. A native transport regression source holds the preload response, drops the link owner, joins a blocking consumer, and checks one request, one link timing entry and cleanup accounting.

Stylesheet requests now capture signed preparation policy and parent loading validates it against the live native owner. CSS imports preserve the snapshot. Non-stylesheet CSS resource requests strip stylesheet-only options rather than being rejected by the destination guard. Dynamic script and stylesheet blocker captures now end the DOM borrow before subsequent native initializer/lease operations.

Still actively generation-open: standalone classic/style preloads, HTTP Link header preload admission, full cross-policy cache behavior, parser encoding-checkpoint rollback for new preload queues, and complete retained publication/lifecycle regression coverage. The new producer/cache code is source-generated and has not been built or executed; this section is not a broad completion claim.

### Admission and preload rollback review (source generation, not executed)

- Dynamic script preparation now captures fallible metadata and target leases and reserves its bounded queue before admitting a render blocker. A failed final native mark drops the prepared token. The generated real-capacity regression rejects entry 257 without starting the script or blocking rendering, retries it after queue release, and checks one admission and cancellation release.
- Parent-backed classic/style HTTP Link preloads now join the existing root/frame response-header producer hook. Header parsing preserves quoted delimiters, first parameters, destination, CORS, integrity, media and referrer policy; URI resolution uses response URL/UTF-8, separately from HTML element query encoding. Header producers have no synthetic element/event.
- Parser encoding rollback does not rotate Document/stream authority. The checkpoint now retains exact general-preload baseline sources, pauses baseline link publications, cancels tentative sources even after a parser consumer claimed them, restores baseline unclaimed lookup once, and restores element/header admission identities. Network response bytes remain owned by the original producer. Generated store regression covers claimed tentative cancellation, baseline reclaim and idempotent acknowledgement after reparsing.
- Store retirement follows the same entries-before-owners lock order as admission and performs transport cancellation after unlocking. Element `rel`/`as`/empty-`href`/media/MIME invalidation clears only the original environment's preparation identity while leaving its admitted source independently owned.

**Actual remaining generation:** standalone Broker/Sources classic/style preload admission and consumer sharing; cross-policy/parser-insertion reuse requires separate consumer authorization before broadening the exact request key; complete source matrix and native parser/header/stylesheet handoff regressions remain open. The new parent-backed paths and checkpoint changes have not been compiled or executed. Earlier sections describing header admission/rollback as absent are superseded only by this bounded source delivery.

### Standalone preload producer/consumer integration (source only)

The Unix standalone Broker/Sources paths now use a Document-owned typed preload store attached to the native script-report owner. HTML and HTTP Link producers invoke the existing classic ScriptFetch or root-only stylesheet loader; stylesheet imports remain consuming-job work. Parser classics, dynamic external classics and stylesheet consumers claim matching responses once. The duplicated dynamic standalone fetch implementation was removed in favor of the shared classic loader and retained asynchronous completion queue.

Native request keys retain policy, decoding, referrer, integrity, CORS and insertion inputs. The report retirement callback revokes producers; parser encoding checkpoints preserve baseline entries and cancel claimed tentative entries. Header jobs have explicit host polling/deadline ownership despite having no event target. Repeated preload admission after a consumer claims an in-flight source reuses that producer without enabling a second consumer claim or resetting its render-blocking status. Both stores retain bounded response bytes; the parent response budget includes headers/raw-body transport as well as decoded source. Native capacity/cancellation/infrastructure failures remain operation failures instead of fabricated link load/error deliveries.

Generated regressions cover parser classic/style/import single-fetch handoff, dynamic link removal before classic consumption, native policy/encoding mismatches, original response retirement, repeated preload after claim, and checkpoint cancellation/idempotence. **No execution has occurred.** Remaining source reconciliation includes non-Unix compatibility producers, reuse across differing captured policy/parser metadata with independent consuming authorization, and complete pinned caller/integration review. The previous entry's statement that all standalone producers were absent is superseded for the Unix native paths only.

### Consuming-policy proof and parser-mode reuse (source generation)

The previous exact-policy/parser-key restriction is superseded by native consuming authorization. Parent preload loading uses appended native-only `PreloadResource`/`PreloadedResource` protocol variants. A bounded parent registry records the authenticated Document owner, request inputs excluding only the captured CSP token/parser mode, and the complete actual redirect URL sequence. `AuthorizePreloadConsumption` validates that opaque proof against the live consuming owner and its independently captured policy. Every original hop is checked; changed initial or redirect upgrade behavior requires an ordinary fetch. Nonce, integrity, credentials, decoding, referrer and all other request inputs remain match requirements. Author JSON dispatch does not expose either operation.

Standalone ScriptFetch and root stylesheet loading retain equivalent private native redirect evidence. A cache candidate reserves its single-use claim during the wait/check, releases it on denial or interrupted authorization, and consumes it only on successful reuse. Denied speculative authorization does not emit duplicate CSP reports; the ordinary fetching path produces the actual blocked-load disposition. Accepted report-only checks retain their normal reporting path. Failed preparation under a different original policy is not reused without proof.

Generated sources cover actual parent redirect capture, intermediate-hop denial despite allowed start/final URLs, parser-mode changes, forged request rejection, owner retirement, foreign registry rejection, standalone rejected-claim release, and static-preload-to-dynamic-classic single-fetch handoff. This protocol extension preserves existing script and PageResource constructors through traced internal wrappers. Protocol version constants are reserved for the coordinator's final combined reconciliation. No build, test or generator has run.

Remaining generation/review scope: non-Unix compatibility producers and exhaustive native caller/lifecycle reconciliation against the full L-02 map; independent review of the newly generated preload proof/cache/rollback group is still required. Broader rendering and L-02 acceptance gates remain open until the full generation phase and subsequent execution are complete.


## Composed native presentation custody (source generation, not executed)

Native root scene acceptance now owns child paint milestone publication. Child
scenes retain their original Document identity and pending milestone until the
accepted ancestor display list presents their embedded-document coverage through
native opacity, clipping, projection and masks. Rejected root scene construction
cannot expose child first-paint/FCP; a hidden child retains its pending milestone
for a later visible scene. Source regressions cover both cases.

The entire presentation opportunity retains its recursion guard. Native child
capture avoids ordinary before/after page-task polling and duplicate layout/rAF
entry: the root animation phase already visits the realm tree. Geometry observer
callbacks remain part of rendering, with original identity/gates rechecked, while
layout installation during scene construction avoids the unrelated page-wide task
checkpoint. Nested frame captures restore the actual prior automation scope on
success and early errors, rather than incorrectly selecting the top document.

These are bounded source claims. Runtime rendering order, general occlusion and
full pinned caller coverage remain subject to the generation inventory and later
execution phase.

### Optional preload proof capacity — independent source review, September 17

The parent preload path previously propagated optional authorization-proof capacity exhaustion after successfully fetching the resource, converting a successful script/style response into a load failure. It now returns the successful `PreloadedResource` with no proof only for the bounded evidence-storage limit. Owner/trace/protocol errors remain errors. Exact-key consumers can use that response; differing-policy consumers without proof use the ordinary fetch path. Expanded the parent redirect/authorization regression source to fill all 1024 evidence slots, load again successfully without proof and verify the earlier proof remains retained. No build/test execution; broader preload and L02 qualification remain open.


### Native visibility at presentation admission

The host scene gate now calls `document_presentation_allowed`, composing native
Window visibility with the existing resource/reveal/lifetime gate. Previously
rAF suppressed hidden/minimized windows but normal paint publication could still
proceed. Regression source extends the native platform transition test: an
authored `document.hidden=false` override cannot admit a minimized scene, while
authenticated restoration admits it. Transient backend unavailability remains
fail-closed and does not fabricate a paint entry. No execution performed.

### Exact preload selection — independent source review, September 17

Parent-backed and standalone stores now prefer an available exact normalized request before considering a policy/parser-mode variant requiring reuse authorization. Previously an older variant could hide a ready exact response and cause unnecessary fallback fetching. Added source regressions with an older variant and later exact response, checking identity, no promotion of the unrelated producer, claim release/reselection, successful consumption and subsequent variant availability. Cross-policy proof checks and single-consumer claims remain in place. No execution.


### Selected-frame native style ownership

Composed presentation review found the layout computation, animation revision
and active-animation helpers always read the top realm's LiveLayoutProvider.
They now resolve the current validated automation realm, so child styles and
animation state stay document-owned. A native scene regression uses conflicting
root/child paragraph widths and checks each accepted scene's actual geometry.

Remaining concrete rendering review: geometry-observer mutation stabilization
before scene acceptance; general content occlusion; complete non-Unix preload
producer wiring. Existing root/frame identity checks prevent replacement-document
publication but do not by themselves prove resize-observer convergence. No source
claim in this audit treats those unresolved algorithms as complete.


## Native geometry observer replacement — active generation

Source review found `install_layout` invoked an author-visible legacy bootstrap
function and no native ResizeObserver interface existed. The production shim
call is removed. Native ResizeObserver/Entry/Size IDL and private records now
provide observation options, target re-resolution after conversion/adoption,
content/border/device sizes, writing modes, depth-limited gather/broadcast,
retained preparation failure, loop-error receipt custody and document identity
checks. Native presentation recomputes geometry after each broadcast before
accepting a scene. Forced layout does not itself broadcast observers.

Generated regression sources cover private slot forgery, pre-callback allocation
failure without lost delivery, immutable branded entry values, disconnect/reuse,
same-depth loop deferral, and callback-driven geometry in the first scene.
Required remaining source work includes page-wide cross-document observation
ordering, SVG/fragment size reconciliation, native IntersectionObserver replacement
and full pinned observer dependency inventory. This subsection does not claim
that the entire geometry observer group is complete or executed.

## Portable preload producer integration — active source reconciliation

General local preload state, admission/header jobs, polling/deadlines and parser
encoding rollback now compile through platform-independent source branches.
Only parent transport variants remain Unix-gated. The standalone stylesheet
loader and dynamic stylesheet local consumer likewise replace the non-Unix
“broker unavailable” branch. Active mixed-content admission is shared between
parent and standalone script/style loaders; `script_fetch` no longer references
the Unix-only context service module for this predicate.

Remaining portable caller review: CSS dependency publication and parser fallback
handoff, plus regression sources directly exercising non-Unix branches. No build,
test, formatter, generator or service execution has occurred.

## Native IntersectionObserver integration — source generation under review

The native observer now has private callback/target/entry records, Web IDL dictionary
conversion in member order, native entry construction, threshold transitions,
`takeRecords`, disconnect/unobserve, bounded pending entries and deferred callback
tasks. Callback preparation errors retain the batch and a 10 ms retry deadline;
acceptance removes the batch before author entry, preventing callback replay.
Suspended documents retain pending work without advertising an immediately runnable
task. Sampling runs after composed-tree ResizeObserver stabilization, and delivery
runs through the host task queue. The superseded authored ResizeObserver and
IntersectionObserver classes and geometry-dispatch shim were removed.

`PaintClip` now transports serde-default native owner identity and independent
scroll-container axes. Layout populates these from the actual clipping element;
paint-only and print clips retain no native element authority. Explicit roots
exclude ancestor clips outside the root, and scrollMargin cannot expand an
`overflow:clip` boundary. Native frame traversal validates the embedding connection,
projects through content origins, clips embedded viewports, suppresses cross-origin
margin/root information, and maps intersection snapshots back to target coordinates.

Visibility tracking shares native permission paint coverage, including physical
pointer-transparent occluders; its entry point removes permission-only inertness
and fully-unclipped eligibility restrictions. Frame ancestors are conservatively
checked. The common rendering gate also suppresses opportunities while native
unload is retained, including after its rendering timeout.

Regression sources cover threshold transitions, edge-adjacent zero-area
intersection, percentage margin basis, single-axis clips, singular transforms,
scroll-versus-clip margin semantics, dictionary order, signed-long conversion,
private entry brands, failed preparation recovery, takeRecords, disconnect with an
already queued snapshot, and suspension/resumption. No regression has executed.

Remaining generation/review obligations: exact transformed/fragmented clipping
against pinned fixtures; region-specific ancestor-frame visibility (the current
whole-frame conservative proof can reject an otherwise unobscured subregion);
callback realm/retirement and document.open reconciliation; complete direct and
transitive pinned caller matrix; native composed-scene observer integration
regressions; and the portable parser/style/load barrier review. Broad L-02/rendering
closure remains open. This section records generated source, not conformance.

Follow-up source review replaced whole-frame visibility rejection with the actual
observed polygon transformed at each native embedding boundary; unrelated portions
of the iframe no longer determine the target's occlusion. Native implicit-root
selection now follows the observer's own top-document chain, preventing a target
in an unrelated auxiliary window from replacing that root. Explicit-root targets
remain confined to the root document. An independent-arena, two-window source
regression covers the implicit-root isolation. These fixes supersede the
whole-frame-proof residual above; transformed clipping, complete lifecycle and
pinned caller reconciliation still remain under review.

Pinned IO source reconciliation additionally covered `empty-root-margin.html`,
`observer-attributes.html`, `root-margin-scroll-margin-units.html`,
`cross-document-root.html`, and `display-none.html`. Empty/whitespace margin is
zero; absolute CSS units normalize to pixel lengths; invalid cross-document roots
and targets without layout boxes publish the specified empty nonintersecting
snapshot. Even these inactive observations retain native revision dependencies so
later insertion/display changes can schedule a new observation.

## Shared CSS basic-shape clip generation — active

Pinned `clip-path.html` exposed missing native HTML basic-shape clipping. Added
shared used geometry for inset/rounded inset, circle, ellipse and polygon fill
rules; native PaintClip transports that shape and original owner. Layout emits it
for the element and descendants. Raster projection and native hit testing consume
analytic shape membership; vector print falls back to native raster for shaped
clips, while extracted print text respects the same clips. Intersection geometry
uses analytic curve/rectangle crossings and polygon edges rather than a sampled
visibility grid. Continuous polygon-only visibility rejects curved/complex clips
when it cannot prove them, leaving physical raster coverage authoritative.

Source regression coverage includes collapsed insets, rounded corners, percentage
ellipses and nonconvex polygons. Still in generation: remaining shape functions
(path/shape/rect/xywh), URL-reference/SVG geometry integration, all geometry-box
rules and shared paint/IO integration regressions. Unresolved native shapes remain
explicit diagnostics; this is not a completed CSS clipping/conformance claim.

Clipping follow-up generated rect/xywh and SVG `path()` shape geometry. A direct
pinned `svgtypes = 0.15.3` source dependency parses paths; bounded adaptive native
contours handle cubic/quadratic/smooth curves, elliptical arcs, relative commands,
closure and compound fill rules. Tessellation has a 0.001 CSS-pixel local tolerance
and a 65,536-point admission bound; exceeding bounds produces an explicit geometry
error. Compound-hole, curve-extrema and arc regression sources were added. Global
lock/profile reconciliation remains deferred with the source-generation barrier.
Remaining shared clipping implementation includes `shape()` command grammar,
SVG URL-reference clipping, exhaustive geometry-box selection, and transformed
curve precision policy. No execution or broad clipping completion is claimed.

The shared parser now also resolves computed `shape()` commands against native
reference-box dimensions: absolute/relative move/line/hline/vline, quadratic/cubic
curve and smooth commands, explicit control origins, elliptical arc flags/rotation,
closure and compound fill. It reuses the same bounded SVG contour admission;
percentage/control-origin regression sources are generated. SVG URL-reference
clipping and exhaustive reference-box/transform fidelity remain open.

Same-document SVG URL clipping now has a native resolver. LayoutNode carries
serde-default tree-scope and original Document URL, filled by the native DOM
projection before style/layout; authored attributes cannot substitute either.
Fragment lookup remains in that native scope, and absolute references must match
the original Document URL. The resolver supports clipPath units, unioned primitive
children, path/use, SVG and planar CSS transforms, and nested intersecting clips
with cycle/depth bounds. Scope, objectBoundingBox union and foreign-source rejection
regression sources are generated. PaintClip shape and LayoutNode metadata changes
must enter the final transport/profile compatibility reconciliation.

Outstanding source work in this group: admitted external SVG clip resources;
SVG text clipping/shaping; complete SVG fill/stroke/view-box selection; exact
intersection geometry across transformed curved/compound clipping; scaling-aware
path tessellation precision; complete IO retirement/document.open caller review;
and portable preload/parser load-fence source review. These are implementation or
source-review obligations, distinct from the globally deferred build/tests/WPT.


## Shared clipping continuation — external SVG and text (source only)

Generated source now carries successful native CSS SVG response bytes through the
original Document metadata into `LayoutInput.svg_clip_resources`. The existing
bounded CSS image owner retains request/response aliases and cancellation; there
is no second clip fetcher. External clip extraction validates requested and final
response origin, resolves the fragment in a separate bounded XML tree, and matches
that resource's native Stylo cascade independently of embedding-page selectors.
Resource removal clears the geometry input; late canceled completion cannot
republish it. External text/element interleaving is preserved in native content.

SVG text clipping now uses the pinned, already vendored usvg text-to-outline
implementation with native captured font bytes and disabled image URL resolvers.
Text/tspan positioning and textPath references are projected from the original
native scope. Glyph paths, counters and affine transforms feed the same Shape
representation consumed by paint, hit testing and IntersectionObserver. Generated
regressions cover actual glyph holes/position, native text interleaving, isolated
external resource cascade, missing/foreign final response, DTD rejection, and
resource publication/removal. Direct layout dependencies reuse pinned usvg 0.45.1
and roxmltree 0.20.0; lock/profile reconciliation remains deferred.

The transformed region engine retains union/intersection operands and path holes
until final bounds, instead of repeatedly clipping axis-aligned boxes. Retained
path source is refined using a homography derivative bound; raster tolerance is
1/256 device pixel and observation contour tolerance is 0.0001 CSS px. These are
bounded numerical approximations, not exact algebraic curves. Membership work is
charged for crossing candidates as well as endpoint candidates. Rotated-corner
and compound-hole regression sources exist; execution has not occurred.

Remaining generation includes full SVG geometry-box/viewBox/stroke-box mapping,
external SVG stylesheet/font/subresource and non-UTF8 XML dependency closure,
complete SVG text style/font alias and textPath reference coverage, projective
horizon clipping rather than explicit precision failure, observer Document-open/
retirement lifecycle reconciliation, and portable parser/load-fence callers.
Broad rendering/L02 source and all acceptance gates remain open. This section
supersedes only the earlier statements that no external/text clip route or
transform-aware path refinement had yet been generated.


### Observer idle-root source reconciliation

IO removes its scheduling-map/self-wrapper root only when both targets and queued
records are empty; disconnect preserves an already queued callback, whose prepared
local wrapper remains live while its final native self-root is released. A later
observe call restores scheduling custody on the same object. RO likewise releases
its self-wrapper on disconnect/unobserve-last. A regression exercises disconnect
with pending delivery, exact callback count, release, and re-observation. This is
source evidence only; it does not assert general collection of JS/native callback
cycles or complete retired-document cleanup.


### Portable parser barrier continuation

Confirmed `document_lifecycle_publication::document_load_blocked` includes retained
native stylesheet event queues on every platform, as well as dynamic stylesheet
and parser jobs. Load therefore remains fenced after CSS publication while a
load/error listener suffix is retained. The actual non-Unix error gap was the
bare `?` on `portable_styles_block_scripts`: it discarded the popped parser owner.
Both platforms now share the same retained-error branch and 25ms retry deadline.
A source regression injects barrier failure before either ordered script executes,
checks exact owner identity/jobs/load state, then checks once-only ordered retry.

Parser readiness, retries and polling now preserve parked realm jobs without
running them. Dynamic stylesheet and CSS dependency polling/readiness likewise
park live jobs rather than dropping or publishing them. Existing font-display
readiness/poll already used the authoritative parked registry and was preserved.
This does not constitute full portable execution qualification or complete
observer retirement coverage; no builds, tests or generators were run.


### Retired observer and actual native park coverage

Both native observer modules now release their retired scheduler/self-wrapper
roots through `V8BindingState::retire_execution`. IO entry snapshots remain in the
script-held observer record, so takeRecords/immutable entry access survives; new
observe calls on that retired owner update private lists without recreating a
runnable scheduler. RO uses the same separation. Generated retirement regressions
assert no callback, no self-root, readable IO snapshot and no scheduler resurrection.
BFCache suspension remains distinct and retains pending delivery for restoration.

A parent-authenticated native root/frame regression now queues a frame parser job,
parks the actual root authority, asserts no runnable parser work and intact job,
restores authority, and checks exactly one script execution. The existing portable
stylesheet event/load fence remains shared with Unix. These are generated source
checks, not executed results; broader callback-cycle collection and full portable
platform qualification remain open.


### Pinned Core top-layer clipping producer review

Reviewed top-layer-containing-block.html, top-layer-parent-transform.html and
 top-layer-parent-clip.html against the complete layout producer. The existing
`compute_with_scrollbar_gutters` native top-layer projection already reparents
modal/popover/fullscreen elements into independent layout roots before sizing and
paint projection, preserving inheritance from the earlier matched cascade. No
additional escape algorithm was necessary. New regression sources exercise native
top-layer metadata under a positioned/transformed/clipped ancestor, initial-block
percentage width, and preservation of the promoted element's own shaped clip for
itself and descendants. This is source review/coverage, not a passed reftest.

Added document.open observer coverage preserving the queued old-target entry and
resampling the now detached target rather than borrowing replacement-node geometry.

### SVG descendant geometry and presentation cascade source reconciliation

Native SVG descendants previously retained generic CSS layout boxes. The new
`svg_geometry` producer derives viewBox/unit/transform bounds from the same pinned
usvg normalization used by SVG paint, preserves unclipped target bounds, and
carries native user-space clip planes into observer projections. Nested SVG
viewport clips remain separate retained shapes. Images use admitted intrinsic
metadata and geometry placeholders, with all geometry-pass resource resolvers
disabled. Budgets bound source size, nodes, fonts, and accumulated clip storage.

The review also found missing SVG presentation hints: computed `clip-path:none`
could hide a native attribute reference. SVG declarations now enter Stylo at
presentation-hint origin, with native owner URL and SVG transform grammar
normalization. Author declarations including explicit `none` win normally;
computed SVG geometry/text properties are exported to consumers. Clip extraction
no longer reinstates transform/clip attributes after an authored `none`.

Regression sources cover viewBox-scaled rectangle/circle clipping, author geometry
and `none` overrides, SVG transform syntax, hidden geometry, nested viewport
clips, image bounds without network/decoder access, and escaped fragment IDs.
These sources have not been compiled or executed. Remaining source reconciliation
includes foreignObject descendant observer geometry (paint already owns an
independent local formatting pass), text-element versus glyph bounds, nested
viewport percentage references without viewBox, and the previously listed
external SVG/font/dependency cases. This is bounded source evidence, not closure
of the full geometry observer or L-02 acceptance gates.

Follow-up source integration uses the existing foreignObject local formatting
algorithm for observer descendant geometry, then composes SVG transforms and
ancestor clips. Promoted top-layer descendants keep their independent viewport;
hidden foreignObject subtrees clear stale projections. Native text parent bounds
already use usvg glyph metrics rather than outline bounds (see pinned
`usvg/src/text/mod.rs::convert`); per-tspan identity projection remains open.
Nested no-viewBox percentage references now walk the native SVG viewport chain.

### CSS dependency admission and binary publication retention

A popped dependency batch previously marked itself evaluated after admission
failure, and binary publication consumed the only body/byte lease before fallible
cache/font installation. Both paths now retain their original owner. Successful
binary cache/native-font publication advances a committed marker before the
fallible invalidation tail, preventing reinstallation on retry. Cache replacement
preflights capacity/allocation before removing the prior live entry. Retry
readiness and the executable delay use 25 ms deadlines and exclude parked owners.
Source regressions inject dependency admission, precommit, and postcommit failure;
none have run. Remaining host batch-transfer admission and complete font-demand
caller inventory are still source-review obligations.

The host-to-executable CSS batch transfer now restores failed owner admissions
into the original native queue with the original byte lease and a 25 ms deadline.
The admission section invokes no author code; drained queue capacity remains
available, and retired/changed stream batches are discarded. Existing source
replacement is staged after all fallible owner admission work. A source regression
covers queue transfer failure, no immediate runnable drain, and eventual original
batch admission. No test execution was performed.

Per-span native geometry is now source-generated through a documented pinned
usvg extension: parsed spans retain source ancestor IDs and layouted spans expose
the existing glyph-metric rectangle. The native consumer unions only each
original element's runs, preserving positioned tspan/textPath identity and
inherited viewport clipping. Added a tspan metric-extent source regression.
External SVG resource/font dependencies, exact transformed/percentage edge-case
qualification, full SVG input/selection projection and observer lifecycle/GC
qualification remain open; this does not promote the broad parent to complete.
