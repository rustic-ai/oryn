# Loading integrity generation — source only

The full-plan source audit found an explicit L-02 gap: import-map resolution
rejects every resolved module with nonempty integrity metadata. Valid hashes
therefore cannot load. This is not a runner prerequisite issue.

Generated `oryn-native/src/integrity.rs` verifies fetched representation bytes
using SHA-256/SHA-384/SHA-512, strongest supported metadata, multiple alternatives,
response eligibility, and supported base64 forms. Regression source covers known
hashes, byte changes, strength downgrade rejection, wrong-length digests,
unsupported/malformed metadata and ineligible responses. ImportMap now exposes
metadata by resolved request URL for the native fetch handoff.

The blanket resolver rejection has NOT been removed: the verifier still needs
LoadScript/raw-response integration, cache provenance, redirect/request URL
binding, and host/static/dynamic module consumers. Hashing decoded JavaScript
strings is not an acceptable substitute for raw-byte verification. Existing
CORS/security checks must remain in force.

No builds, tests, generators or conformance runs have executed. L generation and
full-plan acceptance remain open.

## Native fetch integration generated

LoadScript now carries optional integrity metadata. Parent data/network script
loads verify representation bytes before decoding and retain no-CORS eligibility
loss across redirect hops. Module fetch keeps its CORS checks before integrity
verification. Context protocol advanced to 22 to prevent an older parent from
silently ignoring the metadata.

Import-map resolution now returns its URL; native module fetch obtains metadata
for the original resolved request, carries it through the parent request and
verifies direct-broker response bytes. Integrity-bearing module resolution does
not trust source-only cache entries. Speculative graph loading and parent-backed
parser script loading carry import-map/element metadata. Generated regression
source checks raw versus decoded bytes, classic no-CORS eligibility, module CORS,
data URLs, and integrity-preserving resolution.

This supersedes the earlier blanket-rejection status. Still pending: full caller
and cache-provenance reconciliation, positive end-to-end static/dynamic module
regression sources, classic crossorigin modes and broader resource SRI consumers.
All source is unbuilt/unrun; no conformance claim is made.

## Scheduled and dynamic script source integration

Dynamic script preparation now retains the integrity attribute in its immutable
native preparation record. Scheduled parser fetch contexts retain the attribute
and send it to parent raw-byte verification. Dynamic parent requests use that
snapshot, with module-map metadata fallback; broker-backed dynamic scripts bypass
decoded resource entries when integrity is present, verify before text decoding,
and validate cross-origin module CORS with same-origin credentials. Regression
source covers byte encoding, poisoned decoded cache, digest rejection, opaque
classic rejection and CORS module acceptance. No execution has occurred.

Remaining obligations include classic crossorigin modes, fallback redirects,
parser module-map metadata at scheduling time, prepared parser metadata reuse,
and full raw-resource cache provenance. These changes do not close the L gate.

## Cross-origin classic script integration

Protocol 23 adds the captured crossorigin attribute to LoadScript. Parser,
scheduled parser and prepared dynamic callers pass the native attribute; module
dependency defaults remain same-origin credentials. Parent and dynamic broker
loaders now validate CORS for classic opt-in requests and implement anonymous
versus use-credentials cookie modes. Credentialed cross-origin responses reject
wildcard ACAO, missing ACAC and duplicate ACAC. Added parent regression source
for empty/invalid/anonymous defaults, case-insensitive use-credentials, credential
selection and response rejection. These sources have not been built or run.

Remaining integration includes module graph credential inheritance, fallback
redirects and full prepared parser/cache proof handling.

Scheduled module fetches now also snapshot import-map integrity fallback before
starting asynchronous work. The native prepared-script lifetime regression now
asserts integrity and crossorigin attributes remain their preparation-time
values after author mutation/removal. No tests have run.

## Shared broker loader and redirect generation

Replaced the duplicated dynamic-script and module-import fallback fetch bodies
with script_fetch.rs. Both now share bounded redirects, one overall deadline,
CORS at each response, opaque redirect origin tracking, persistent no-CORS
integrity ineligibility, reduced referrer propagation, cookie modes, MIME
validation and raw-byte integrity before character decoding. Data URL scripts
use the same final verification. Module imports reserve unique bounded request
ID ranges before broker calls.

Regression source covers a cross-origin chain returning to its original origin,
null Origin and suppressed same-origin credentials after taint, redirect CORS
failure before the next request, and opaque classic integrity rejection after
a redirect home. No build/test execution. Parent CSP authority remains in the
parent loader; remaining L obligations include prepared parser cache provenance
and module graph credential inheritance.

## Parser preparation and completed-source ownership

Parser records now retain immutable nonce, integrity, crossorigin and element
referrer policy. Scheduled fetch success is stored on its exact parser record;
deferred execution restores those completed bytes instead of accepting a shared
URL-map overwrite or re-reading edited integrity attributes. Native standalone
parser fetches use the shared broker verifier when integrity/CORS requires fresh
proof or no decoded resource exists. Added regression source for valid/invalid
parser integrity with a poisoned decoded entry, and extended deferred-identity
coverage to mutate integrity and overwrite the resource map before execution.

No build or tests have run. Whole-plan generation and qualification remain open.

## Module graph request policy

Generated per-realm module fetch credential inheritance from parser/dynamic root
scripts through cached, fetched and redirected dependencies. Native parent
preloading and broker module imports now use the document's request initiator
for CORS, while resolving URLs relative to the referencing module. This prevents
a CDN-hosted module from treating its CDN dependency as same-origin with the
document. Added three-level CDN module regression source for anonymous and
use-credentials modes, checking Origin and credential flags at every request.
No tests/builds have run.

## Native module fetch result reuse

Generated a document-context-owned module fetch result map, retaining verified
source/final URL or failure by requested module URL. Repeated dynamic imports
reuse it; compiled module entries are consulted before resource-map fallback.
Source retained in this map is restored when isolate-wide decoded source maps
are replaced. Added regression source for redirected valid imports and repeated
integrity failures, including resource-map substitution and exact broker request
counts. This is source generation only; no test results are claimed.

Successful parent module preloads now publish requested URL, final URL and source
to the same per-document verified result map. Speculative failures remain outside
that map until an actual import requests the module. This removes the second
fetch previously required when an integrity-bearing preloaded module linked.

## CORS classic script error visibility

Classic parser and dynamic execution now preserve exception details after a
successful crossorigin fetch. Dynamic crossorigin loads no longer accept an
unverified decoded resource entry. Added native parser/dynamic onerror regression
source checking exact resource URLs and thrown objects; existing opaque classic
error-muting coverage remains. Redirect response tainting still needs to travel
with loaded classic source so a no-CORS cross-origin detour returning home stays
muted. This remaining requirement is explicit; no qualification claim is made.

Script IDL now declares nullable crossOrigin and integrity. Native reflection
returns null for absent crossorigin, canonical anonymous/use-credentials for
present values, removes the attribute on null assignment, and reflects integrity
verbatim. Added native property/attribute round-trip regression source. Binding
generation is still deferred along with all builds and tests.

## Response taint through classic execution

Protocol 25 ScriptSource carries native muted_errors. Parent and shared broker
loaders preserve no-CORS cross-origin taint across a redirect home; parser
completion records and dynamic script result tuples retain it through execution.
Classic execution consumes the response flag rather than inferring it solely
from the final URL. Added parent wire-result regression source plus parser and
dynamic redirect-home onerror coverage. This supersedes the previously noted
redirect-taint propagation gap; validation remains deferred.

## Dynamic script base versus request identity

Broker dynamic scripts now resolve src against the prepared base while deriving
CORS/cookie/error-taint identity from the active native document URL. A base
element pointing at a CDN no longer makes that CDN same-origin for dynamic
fetches. Added regression source for relative dynamic CORS and opaque scripts
under a cross-origin base, checking error visibility, Origin, credentials and
native request initiator. No execution.

Dynamic inline modules now use their preparation-time document base for relative
imports and bypass external root-module caching. Added two-inline-module
regression source with a relative dependency under a base element, checking both
bodies execute independently. No builds/tests have run.

Parser module records now capture the document base at preparation and use it
for inline module resolution after deferral. Added regression source where a
later classic script mutates base before the earlier inline module executes;
the module must retain its original dependency URL. Per-inline-module credential
metadata still needs independent native Script/Module identity rather than
solely URL-keyed inheritance when multiple inline modules share a base.

## Per-compiled-script credentials

Each compiled module now retains its credential setting by native module
identity. ScriptOrigin host-defined options carry the same immutable boolean
to later dynamic-import callbacks; static linking reads the referring module's
record. Inline preparation may update its shared base URL's staging metadata
without changing earlier compiled scripts. Added two-inline-module regression
source covering independent static imports and delayed dynamic imports after
both modules initialize. This supersedes the per-inline identity gap above.
No builds, binding generation or tests have run.

## Native dependency discovery

Removed regex-based module dependency discovery. Parent graph preloading now
uses V8 module requests from parse-only compilation under the native execution
deadline. Comments, strings and unused dynamic imports are not prefetched;
multiline static imports and re-exports use parser-produced specifiers. Added
regression source for those cases and recovery after syntax errors. No build or
test execution has occurred.

## Module evaluation promise ownership

Dynamic import now resolves its namespace only after the native evaluation
promise fulfills and forwards the original rejection value. Evaluation publishes
a retained promise before entering author code, so reentrant self-import shares
in-progress evaluation instead of trying to evaluate again. Document module
execution uses the same evaluation record. Added regression source for externally
resumed top-level await, delayed rejection identity, and dynamic self-import
with exactly one execution. No tests/builds have run; document event ordering
remains under audit rather than being changed by assumption.

## Module environment retirement

Compiled module records now carry the exact native environment owner. Retirement
removes only that owner's compiled cache entries, evaluation promises, names and
credentials, and clears its context-owned fetch/import-map data through weak
context references. Evaluation/import callbacks reject retired execution rather
than running author reactions. Added regression source retiring a frame with
never-settling top-level await, checking root records remain and the retired
context becomes collectible. BFCache suspension does not call retirement and
therefore retains module state. No builds/tests have run.

## Separate module referrer source

Protocol 26 LoadScript and the shared broker fetch contract now carry optional
referrer_source separately from the native document initiator. Module preloading
and actual imports supply their importing module URL for referrer computation
while CORS/cookies continue to use document identity. Added direct broker
regression source and extended the three-level CDN graph regression to check
referrer headers for child and leaf requests. All checks remain unrun.

## Native script scheduling properties and completion ordering

Added script async/defer/noModule IDL and native boolean reflection. The DOM
tracks force-async separately from attributes: dynamically created scripts start
true, parser-created scripts start false, and async assignment/attribute addition
clears the force flag. Prepared scripts snapshot asynchronous scheduling state.
Dynamic parent fetch completions now buffer async=false scripts per realm in
preparation order, including failed/stale entries; async scripts and sibling
realm queues remain independent. Added source regressions for force-async
transitions/boolean reflection and reversed completion with failure plus async
and sibling progress. No builds, generators or tests have run.

## Script started-state ownership

Removed the per-realm prepared_scripts set. The native DOM node now owns
already-started state, and clone/import copy it while adoption naturally retains
it. Parser execution, dynamic preparation, error queuing and inert fragment
marking share that state. Added regression source for clone/import/adopt of an
executed script, clones of inert fragment scripts, and independent execution of
an unstarted script and its clone. No tests or builds have run.

## Script feature detection and import metadata

Added native HTMLScriptElement.supports for classic/module/importmap with
case-sensitive matching and WebIDL conversion. import.meta.resolve captures the
module's native base, resolves through the realm's import map, records successful
resolution and returns URLs without fetching. Author changes to import.meta.url
or this binding do not change that captured base. Added regression source for
relative/mapped/unavailable URLs, null blocks, unmapped and Symbol errors,
coercion count and required argument behavior. No builds/tests have run.

## Interrupted module evaluation ownership

Native module evaluation now removes its provisional promise-cache record if
Evaluate or forwarding-reaction setup fails. The caller retains V8's original
exception/termination state. Reentrant imports still share the promise published
before evaluation, while later requests cannot hit an orphaned pending record.
Generated regression source covers watchdog interruption, cache release and
subsequent successful script/module execution. The frame-retirement regression
now resides with the native private-state tests it inspects. No tests or builds
have run; whole-plan generation remains incomplete.

## Per-script referrer policy across module graphs

The parent ScriptSource response now carries effective script referrer policy
(context protocol 28). The shared broker loader provides equivalent metadata.
Final response headers update script options independently of redirect request
policy; data scripts inherit their options. Parser completions, deferred records,
dynamic completions and speculative graph loading preserve this metadata.

Compiled module records and native ScriptOrigin options retain each module's
policy, including separate inline modules with the same base URL. Static imports
and delayed dynamic imports use that captured policy; the mutable document policy
is only the fallback for a script without captured metadata. Realm retirement
clears the per-document policy map. Generated regression sources cover response
headers, data inheritance, redirect/options separation and delayed inline imports.
No builds/tests ran. Import attributes, classic-script dynamic import metadata
and the broader R/P/L reconciliation remain open.

## Classic-script dynamic import metadata

Classic and module compilation now use one native host-defined option layout
for credentials, referrer policy, resolution base and muted errors. Prepared
parser/dynamic classic scripts pass their captured metadata; synchronous inline
classic insertion captures native attributes and base before author execution.
Delayed dynamic import reads the originating compiled script's options, so later
base/meta changes and other scripts sharing its URL cannot replace them. Imports
from muted cross-origin classic scripts reject before resolution or fetching.

Generated regression source covers inline base mutation, delayed callbacks,
credentialed CORS imports and muted-script rejection without a network request.
This supersedes the preceding classic-metadata open item. Import attributes and
whole-plan generation reconciliation remain open. No build or test ran.

## Typed import request generation in progress

Added native JavaScript/JSON/CSS module-type parsing, strict case-sensitive import
attribute validation and MIME rules, with regression sources. The V8 adapter
distinguishes static attribute triples (including source locations) from dynamic
attribute pairs. Speculative JavaScript graph loading now leaves JSON/CSS requests
to the typed loader rather than fetching them with JavaScript MIME requirements.

This is foundational source, not completed JSON/CSS module support. Typed fetch
transport, type-aware module caching, synthetic exports, CSS sheet construction,
worker integration and runtime callback dispatch still require generation. No
builds/tests have run; the whole-plan gate remains open.

## Typed module transport

Context protocol 29 adds LoadTypedModule with a serialized native ModuleType.
The parent reuses script authorization, CSP, CORS, cookies, redirects, integrity,
referrer policy and byte limits while selecting strict MIME validation by type.
The shared broker loader exposes the corresponding typed entry point. JSON/CSS
module source uses UTF-8 decoding regardless of a legacy response charset. Data
URLs follow the same type/MIME validation and raw-byte integrity checks.

Added source regressions for typed parent data requests, MIME mismatches and
UTF-8 text preservation. This closes the transport-generation item above, not
module implementation: native synthetic exports, type-aware caching, CSS native
construction, callback dispatch and worker integration remain in progress.
No builds or tests have run.

## Native typed module records and exports

Window static/dynamic import callbacks now validate attributes and dispatch
JSON/CSS requests through the typed loader. JSON uses native V8 JSON parsing;
CSS creates a native constructed stylesheet and parses without imports, without
calling author constructors or replacement methods. Synthetic modules export a
single default binding. Cache keys include environment, requested URL and type;
failed loads retain their original exception independently of JavaScript entries.
The registry caps per-environment entries and aggregate decoded source bytes.
Environment retirement releases cached modules, exported values and accounting.

Generated regression source covers static/dynamic export identity, JSON mutation
and __proto__ preservation, author constructor replacement, CSS rules, MIME and
syntax errors, repeated failure identity, type-separated fetches and frame
retirement. CSS module base URLs reach native CSSOM parsing; propagation to
adopted-sheet layout/resource loading, worker module integration and further
callback/lifetime reconciliation remain open. No builds or tests ran.

## CSS module base URL through layout

Native stylesheet base URLs now live on the DOM stylesheet record, replacing the
binding-only module map so document snapshots retain them. Constructed sheets
capture their native creation base. LayoutInput projects a base per stylesheet;
main Stylo parsing and print-rule parsing consume it through the native adapter.
CSSOM reads the retained base rather than author-overridable global location.
Existing LayoutInput constructors were updated for the new serialized field.

Generated regression source covers independent stylesheet origins, LayoutInput
round trips, adopted CSS modules and replaceSync after document-base changes.
Worker module integration and remaining lifecycle/policy reconciliation remain
open. No builds or tests have run.

## Worker module evaluation and redirect identity

Source inspection confirmed worker dynamic imports already waited for evaluation
promises. The actual gaps were repeated Evaluate calls, fetching before cache
lookup, and discarding parent-returned redirect URLs. Worker startup and dynamic
imports now share the native published evaluation promise; module fetch results
retain final URLs and response referrer metadata, and compiled records register
native lifetime ownership. Shared live-environment completion callbacks replace
the duplicate worker callbacks.

Generated regression source covers reentrant worker self-import without another
fetch and redirect-relative dependencies with namespace/fetch reuse. Worker typed
imports and parent propagation of per-import referrer metadata remain open.
No builds or tests have run.

## Worker typed imports and referrer authority

Context protocol 30 / agent IPC 9 carry ImportModule with a native wire type and
importing script referrer metadata. Worker static/dynamic JSON imports now use
the same synthetic-export and type-aware cache machinery as documents, through
the route-bound parent worker loader. CSS imports reject in worker globals;
service-worker dynamic import rejects before transport. Classic worker scripts
also retain native import metadata in their compiled ScriptOrigin.

The parent retains worker origin, credentials, partition and cookie initiator
authority separately from the importing script's referrer URL. JSON data, blob
and network loads share MIME/UTF-8 rules. Redirects retain reduced referrers and
opaque Origin state. Added worker JSON identity/type rejection, service dynamic
import rejection, parent MIME/UTF-8 and initiator/referrer regression sources.

Durable service imported-script maps still lack full type/MIME/referrer
provenance; JSON bypasses their untyped reuse path pending that integration.
Further source reconciliation and all execution/qualification remain open.
No builds or tests have run.

### Durable service import response provenance (source only)

- Native bootstrap graphs now retain import kind, final response URL, MIME, effective response referrer policy, and importing request referrer source/policy alongside source and raw-byte digest. Context protocol 31 / agent protocol 10 carry the new schema.
- Restored service imports are admitted through the parent registry, including JSON; requested module MIME is checked against stored provenance. Installed resource maps cannot be rewritten by a new import.
- Registration updates refetch each saved resource using its own kind and request options; provenance changes participate in update equality. Legacy resources without provenance explicitly require an update to obtain a validated response rather than inventing MIME evidence.
- Generated JSON redirect/update, immutable installed graph, MIME mismatch, and serialization regression sources. Existing restart fixture is being integrated by worker ownership agent. No builds or tests run; all generation gates remain open.

### Recursive dynamic-script admission (source only)

The scheduler no longer creates independent fetch batches. A single per-realm ordered completion queue admits scripts inserted by each completed script before awaiting the next network response; force-async successors and sibling realms can progress while an earlier ordered fetch is pending. Added regression source for recursive admission and predecessor ordering. Prepared native leases remain retained until this drain completes, and stale completions still use exact preparation validation.

This does not yet complete asynchronous scheduling: the drain still waits on its next network response and needs reconciliation with the page task pump, timer/message progress, cancellation, and load barriers. Whole-plan generation remains open; no build or tests have run.

Restored classic worker imports now enforce the same strict JavaScript MIME requirement as the network worker loader. Added negative regression source for missing MIME, HTML, JSON, and plain text.

### Persistent dynamic-script task integration (source only)

The per-page scheduler now retains native preparation records between checkpoints, uses nonblocking completion polling, and exposes pending work to the page task pump. Timers/messages/input can run while a parent script fetch is pending. Load barriers account for queued and fetched-but-unexecuted dynamic scripts in the owning document; stale preparation records are removed after document replacement and late responses cannot execute. Stop drops the scheduler receiver and its native preparation leases. Added regression sources for timer progress with an unresolved script, delayed load completion, and replacement followed by a late response.

This supersedes the prior blocking-drain limitation. Parent fetch transport cancellation and bounded outstanding request ownership still need source reconciliation; synchronous fallback and module graph loading also require the same task-lifecycle audit. No builds/tests have run.

### Native scheduler readiness audit (source only)

The page loop uses zero delay for runnable work. Treating unresolved dynamic scripts or their document-load barriers as runnable would spin continuously while waiting for the network. Added an atomic completed-result count: publication makes work runnable, consuming the result clears readiness, and unresolved network work alone uses the existing bounded idle poll. The timer/load regression now distinguishes pending work from runnable work. Parent-owned fetch cancellation remains the next loading integration obligation. No build/tests run.

### Parent-owned cancellable script fetches (source only)

Added a parent script-load registry with single-use admission tokens, a 128-operation bound, cancellation before/after claim, and RAII cleanup. Cancelled active operations retain their capacity until actual completion; parent teardown cancels all leases. Context protocol 32 adds same-page reserve/run/cancel messages, and the run wrapper accepts only classic or typed script loading while preserving the existing URL/CSP/CORS/SRI/referrer path.

The dynamic scheduler now retains a parent lease for every production script fetch. Replacement, stop, scheduler destruction, and discarded completions release/cancel the lease. The loader passes the lease cancellation flag to bounded network collection. Generated registry capacity/single-use tests and a transport regression for live network cancellation, foreign-page denial, cancelled admission, and continued service availability.

Outstanding source reconciliation includes cancellation through module dependency loading and the synchronous fallback; whole-plan generation remains open and no builds/tests have run.

### Queued script-source ownership (source only)

Completed dynamic-script responses now carry a native RAII byte reservation from publication through delivery and any ordered predecessor wait. The aggregate queued-source limit is 32 MiB; overflow follows the script error path, and execution, stale-result discard, or scheduler destruction returns the reservation. Both parent-response and synchronous-fallback results use this accounting. Added capacity/discard regression source and zero-retained-byte checks after successful and stale completions. This bounds retained completed source; transient loader/IPC allocations and module graph memory remain part of broader L memory reconciliation. No builds/tests run.

### Asynchronous broker fallback integration (source only)

Broker-backed dynamic scripts now capture native request inputs on the page task and fetch on a native background operation, returning through the same completion ordering and source-byte ownership as parent-backed loads. Redirect request IDs are reserved in nonoverlapping ranges. The shared loader accepts cancellation, including before a data URL decode or the first socket request. Local admission uses the same 128-operation registry and retains cancelled capacity until the worker completes; stop preserves admission/source budgets for subsequent work. Cached fixture resources remain synchronous and do not initiate network work.

Added a stalled broker regression source proving timer progress and native cancellation on stop, plus pre-cancelled network/data request negatives. Module dependency linking/evaluation is still subject to the remaining L task-lifecycle reconciliation. No builds or tests run.

### Module graph fetch failure identity (source only)

The preloader previously discarded failed responses and let linking retry the same URL. It now records the failed fetch in the owning document module map and skips previously attempted resources across graphs. Verified responses cannot replace an existing failure; another realm keeps its own independent module map. Generated regression source for failure persistence, fixture-source bypass denial, and sibling-realm isolation. Module graph asynchronous task integration and authoritative top-level-await event-order review remain open. No builds/tests run.

### Pinned top-level-await source review

Read upstream WPT at commit `719d5e38fdd0903a18ed9007aba816c98cc491e0`, including [sibling-imports-not-blocked.any.js](https://github.com/web-platform-tests/wpt/blob/719d5e38fdd0903a18ed9007aba816c98cc491e0/html/semantics/scripting-1/the-script-element/module/top-level-await/sibling-imports-not-blocked.any.js) and its task helper modules. These require a sibling to capture its value while another module is suspended, before that module resumes on a microtask or task. Generated matching native regression source for both suspension types, plus independent sibling import progress and single evaluation for repeated imports of a suspended module.

The inspected tests do not establish script-element load-event ordering relative to TLA settlement; that ordering was not changed based on an assumption. Module graph asynchronous loading remains open. No test execution occurred.

### Module graph admission and retained-source bounds (source only)

Consolidated worker-side parent script ownership in `ContextScriptLoad`; typed-module and linking fetch paths now use parent reserve/run/cancel admission. Module preloading owns all outstanding handles on its calling stack, limits concurrent requests to eight, and cancels unfinished requests when the graph loader exits. The graph source total is capped at 32 MiB before publishing a new response to module/source maps. Added source regressions for failed-response retention and over-budget publication denial.

The preloader still waits for a batch on the calling page task; converting graph discovery/link preparation into checkpoint-driven work remains open. No builds or tests have run.

## Native transient activation dependency for picker generation

Added exact-creator transient query/consume to `ActivationBackend`, native context
RPC (protocol 34), and `V8BindingState`. The parent validates document generation
and stream epoch before accessing its existing native input grants; consumption
removes grants in the same top-level page under the grant lock. The author context
bridge explicitly rejects this RPC. Missing authority, retired execution, and
backend errors do not grant transient activation; sticky activation remains a
separate value. The layout owner is consuming this API for showPicker.

Regression source covers five-second expiry, non-consuming queries, one-time
consumption, page-wide consumption without affecting another page, stale document
identity, synthetic events, and retired V8 state. These tests are generated only.
Full activation notification propagation to ancestors and same-origin descendants,
and navigator.userActivation exposure, still need integration; this dependency
change does not establish completion of the activation API family.


Window `navigator.userActivation` source now installs a native same-object
UserActivation instance with branded prototype getters and an illegal public
constructor. Getters consult the owning document's native state, distinguish
sticky from consumed transient activation, and deny retired state. Generated
regressions cover same-object/prototype identity, forged receivers, synthetic
input, consumption, and retirement. Ancestor/same-origin descendant notification
propagation remains open; no build or test has run.


Activation propagation source now records exact native frame-parent identities at
both fetched and inherited document-policy commit. Notification visits all
ancestors and same-origin descendants of the source window; siblings and foreign
origin descendants do not receive a new grant. Sticky grants are parent-owned as
well, so propagated activation is visible through native UserActivation getters.
Consumption clears transient grants across the top-level page while preserving
sticky grants. Parent cycles/rebinding are rejected, retired identities are pruned
without discarding retained BFCache documents, and registries remain bounded.
Generated cross-origin-tree regression source covers an intervening foreign-origin
frame, unactivated siblings, consumption, sticky preservation, and cycle denial.
The new native parent fields were introduced at context protocol 35; subsequent
loader changes may advance that version. No build or test has run.

## Cached module graph discovery reconciliation

Source audit found the preloader skipped already-fetched and resource-cached
modules without discovering their descendants. Added a realm-owned module-map
snapshot API distinguishing unattempted, failed, and verified redirected source.
Graph traversal now walks cached sources, preserves cached failures, seeds the
root as visited to avoid cycle refetches, and applies node/source-byte bounds to
cached as well as newly fetched entries. Redirected source retains its final URL
as the base for descendants. Added source regressions for cache result identity
and failure immutability.

This removes a discovery gap but does not complete checkpoint-driven graph
scheduling: the existing batch receiver still blocks and parser/dynamic callers
must migrate together with cancellation and load barriers. No builds or tests ran.

## Persistent dynamic module graph scheduler

Generated `module_graph_load.rs`: retained discovery/fetch queues, exact native
script-load leases, eight concurrent fetches, bounded graph/source accounting,
completion readiness, cached redirected-source discovery, and nonblocking poll.
Dynamic script scheduler now retains these graphs alongside each preparation,
polls them at checkpoints, keeps the existing ordered/async completion queue,
checks realm/preparation liveness, and advertises ready graph work to the page
pump. Dynamic completion no longer invokes the blocking preloader. Dropping a
graph releases its native cancellation leases. The old standalone batch-consume
path was removed; its budget/failure regressions were migrated to the graph
scheduler with cached-redirect/cycle traversal coverage.

Still open: parser callers use the transitional blocking adapter; typed dependency
and dynamic import loader integration, aggregate concurrent graph memory bounds,
load-barrier/cancellation races, and full native timer/input integration coverage
must be completed before generation can close. No build or test was run.

### Aggregate graph source admission

Dynamic module graphs now share the document's queued-script source budget.
Pending root/cached source owns a reservation; network responses reserve before
channel publication and transfer that reservation through discovery. Completion
publication failure or graph cancellation drops the reservation. Budget failure
is a graph limit error rather than a cached network failure. Added concurrent
admission/cancellation and late-completion cleanup regression sources. Retained
module-map/resource cache accounting remains a distinct follow-up obligation;
this bound covers queued discovery and response payloads. No tests/builds ran.

### Shared module fetch concurrency

Dynamic graphs now share eight network admission slots across the document rather
than multiplying that limit per script. The network task owns its slot until it
terminates, even if graph cancellation drops its parent load lease. Stopping and
replacing the script scheduler preserves the slot counter while old requests
finish. Queued graphs remain pending without issuing a request or reporting
runnable work when all slots are occupied, then become runnable when capacity is
released. Added saturation/cancellation regression source. Parser continuation
migration and whole-plan generation remain open; no tests or builds ran.

### Typed dependency graph loading

Generated native V8 dependency discovery retaining import type attributes, while
preserving the existing JavaScript-only query for callers that explicitly need it.
The persistent graph now keys discovery/in-flight identity by URL and type, sends
JSON/CSS dependencies through LoadTypedModule under the same cancellation lease,
source budget and fetch-slot controls, and caches native verified typed responses
before linking. JSON/CSS bodies are not parsed as JavaScript dependencies. Typed
response caches are context-owned, bounded, preserve failures/redirect identity,
and clear on environment retirement. The existing native typed linker consumes
these responses before its fallback fetch path. Added same-URL/different-type
queued-fetch and cache-backed JSON linking regression sources.

The parser adapter and dynamic-import path still need continuation integration;
this source change does not claim all module loading is nonblocking or qualified.
No build or test ran.

### Concurrent completion cache consistency

Module graph completion now checks the realm/type-specific retained result before
publishing a late response. It drops superseded payload/reservation ownership and
uses the first successful JavaScript source for descendant discovery; retained
failures remain failures. Late bodies cannot overwrite the resource map while the
module map preserves a different result. Added a regression with invalid late
bodies after an earlier success and failure. Typed response caches now explicitly
register their owning contexts so environment retirement clears them even when no
module was linked. Shared in-flight request coalescing remains to be generated;
this consistency fix does not claim duplicate requests are eliminated. No runs.

### Shared in-flight response ownership

Generated a weak request registry shared by dynamic module graphs, keyed by exact
realm, document generation, stream epoch, URL and module type. Followers retain
one native request lease and response envelope; dropping one graph keeps the
request alive for other followers. The final follower releases the lease and any
queued response reservation. Separate document lifetimes cannot share authority.
The network task retains its admission slot until it terminates.

Source reconciliation removed the obsolete per-graph completion channel. The
transitional parser adapter now consumes the actual shared-request receiver and
keeps its completion available to followers; it reports an absent active request
instead of blocking on a channel with no producer. Added regression sources for
parser completion delivery, missing/disconnected producers, late delivery after
cancellation, and response-budget retention until the final follower drops.

These sources remain unbuilt and unrun. Parser and dynamic-import continuation
migration, disconnected asynchronous request handling, registry lifetime/type
separation coverage, and retained cache accounting remain open alongside the
full-plan generation gates. This entry supersedes the earlier statement that
in-flight coalescing had not yet been generated.

### Shared request disconnection and queue scheduling

Shared response readiness now polls the receiver itself and records producer
disconnection. An asynchronous graph whose producer disappears becomes runnable
and retains the native module failure instead of waiting indefinitely for a
ready counter that can never change. Removed that superseded response counter;
the request channel owns delivery and detects producer lifetime directly.

Graphs capture their exact document identity on first poll and reject subsequent
polls under another identity. Queued requests can wake when an exact matching
shared request appears even with all eight network slots occupied. Admission
visits every queued request once, allowing a later shared request to proceed
while an earlier request still needs capacity. Added regression sources for
producer loss, shared admission behind a blocked request, and generation/epoch/
module-type separation. The new sources are unbuilt/unrun; parser and dynamic
import continuation migration and retained cache accounting remain open.

### Parser request cancellation ownership

Parser script scheduling now reserves a native parent load token before launching
the request and runs LoadScript through RunScriptLoad. The parser scheduler owns
the cancellation lease by preparation order; consuming that script's completion
releases its lease, and dropping the scheduler cancels outstanding owned loads.
Duplicate live preparation orders reject before replacing another request's
ownership. Added a transport-backed regression source checking completion release
and parser-drop cancellation without releasing unrelated script leases early.

Source inspection confirms the broader parser lifecycle still waits for resources
inside initial parsing and executes deferred scripts before page construction.
Those callers require persistent parser continuation and readiness-barrier
integration; cancellation ownership is a prerequisite, not completion of that
migration. Stylesheet/frame cancellation and parser source-memory accounting also
remain open. No builds or tests have run.

### Parser completed-source custody

Parser script network completions now reserve from a 32 MiB scheduler source
budget before channel publication. The verified URL/body/error-mode/referrer
policy and reservation travel together in one immutable shared owner, which
queued preparation records retain through execution or discard. Cloning a
preparation record shares that verified source rather than copying its body.
Failed channel delivery and stale-document completion release their reservations;
capacity rejection carries a script-load failure without publishing the body.

Added source regressions for aggregate admission, shared preparation ownership,
failed delivery cleanup, and stale-completion budget release. This bounds queued
completed parser-script payloads; it does not yet unify parser and dynamic-script
budgets or account for retained resource caches, inline preparation strings,
stylesheets, or frame documents. Parser continuation migration and all global
generation gates remain open. No builds or tests were executed.

The transitional parser module graph also receives its verified root's source
budget, so dependency discovery and queued responses compete with other completed
scripts from that parser scheduler. Inline/unscheduled roots still establish a
local graph budget until the persistent parser owner is integrated. This changes
source ownership only; the transitional graph wait remains to be removed.

### Direct typed-import cache reconciliation

Direct JSON/CSS module loading now uses the same bounded native response cache as
graph prefetching. It reserves an in-progress/failure identity before entering
worker, context-service, or broker fetch paths, retains successful redirected
source only within cache limits, and rejects publication after an environment
change. Failed fetches remain document-scoped failures; a later graph cannot
replace them with a different result. The duplicated prefetch-only insertion
logic was removed in favor of one cache retention implementation.

Added cache regression sources covering failed-entry capacity, sticky results,
pending-to-success transitions, URL/body byte boundaries and type separation.
These bounds cover the typed native response cache. JavaScript response-cache
and shared resource-cache accounting still require reconciliation; direct import
network continuation also remains open. No builds or tests ran.

### JavaScript response-cache bounds

JavaScript module prefetch and direct fetch now share a bounded response-cache
insertion path (256 requested identities and 32 MiB of retained requested URLs,
redirect URLs and bodies per context). Failure identities consume capacity.
Successful cache entries remain immutable; only the native fetch completing its
pending entry may replace that placeholder. Admission checks precede source
copies. Public prefetch retention returns capacity errors to the graph, preventing
overflow bodies from reaching the resource map. Direct fetch also checks exact
environment identity again before publishing a response.

Added regression sources for pending-entry byte boundaries, immutable success,
and full-cache graph rejection before shared resource publication. Updated cache
callers for explicit errors. JavaScript and typed response limits are currently
separate; whole-document resource/compiled-module accounting and asynchronous
direct-import continuation remain open. No builds or tests ran.

### Graph lifetime boundary and regression fixtures

Graph polling now explicitly rejects retired execution realms before cache or
request work. Added a cross-realm continuation regression requiring no source
publication or native request after the active realm changes. Source review also
found that graph regression fixtures used bare V8 hosts without native document
bindings; those fixtures now install actual document bindings and stream identity
before exercising the document-scoped scheduler. The lifetime guard is preserved
in production rather than weakened to accommodate incomplete fixtures. These
regressions have not been built or run.

Typed response retention now borrows incoming URL/body data for its capacity
check and copies only an admitted new value. Direct typed imports no longer clone
the response before discovering that the cache is full or already authoritative.
Prefetch uses the same borrowing boundary; byte-boundary regression sources were
updated accordingly. No execution occurred.

### Module request admission outside graph polling

Module graph polling now captures native request authority and schedules token
reservation on its existing bounded fetch thread. Shared graphs own the eventual
lease through a synchronized slot; the fetch thread retains only a weak owner
while awaiting admission. If all consumers disappear, a late lease is cancelled
on that thread and no graph accepts it. Duplicate publication cannot replace an
already owned token. Fetch slots remain occupied until admission/network work
terminates, including after graph cancellation.

Added transport-backed regression source for retained followers, duplicate token
publication, final-owner cancellation and admission arriving after every owner
drops. This removes the synchronous ReserveScriptLoad call from graph polling.
Cancellation acknowledgements can still block existing lease destruction, and
parser/direct-import continuations remain open. No builds or tests ran.

### Queued dependency cache handoff

Queued dependency admission now rechecks the exact realm/type module response
cache before claiming a network slot. A graph that waited for capacity adopts a
result published in the meantime even if the completing graph has already
released its shared in-flight request. Successful JavaScript responses retain
redirect-relative descendant discovery; failures and typed responses do not
start duplicate requests. Added a regression source with delayed cache
publication, redirected grandchildren, a retained failure and a JSON dependency,
requiring no transport writes. No builds or tests ran.

### Cache publication wakes queued graphs

Native JavaScript and typed module response caches now expose context-owned
revision cells. Graphs retain those cells after binding their exact document
identity and compare the last observed revisions when deciding whether queued
work is runnable. Another loader's cache publication can therefore unblock a
queued dependency without waiting for unrelated occupied network slots. A
completed recheck records the current revisions, preventing repeated wakeups
from one unrelated cache change. Other realms do not share these revision cells.

Extended regression sources to require cache-driven wakeup under saturation,
one-shot rechecking after unrelated updates, and cross-realm revision isolation.
All sources remain unbuilt and unrun; full loading continuation and global
generation gates remain open.

### Requested module identity and redirect response base

JavaScript static/dynamic dependency loading now keeps the requested URL as its
module-cache identity while compiling with the final response URL. Native module
names, import.meta.url, credentials and relative descendant resolution use that
response URL. Two requested URLs that redirect to the same response can retain
distinct module instances; repeated imports of one requested URL reuse its
instance. Parser and dynamic external script execution now pass the immutable
prepared request URL separately from the fetched response URL into root caching.

Added regression sources for distinct redirected dependencies, repeated static
and dynamic imports, final-response-relative grandchildren, import.meta.url, and
external script-root cache identity. Shared decoded-resource provenance and
root-fetch admission/coalescing still require integration; this source change
does not prove redirect request counts or complete network conformance. No builds
or tests ran.

### Graph root request identity

Parser and dynamic module graphs now receive the prepared external request URL
separately from the final response/base URL. Discovery marks the requested root
as visited, preventing a redirected self-import from refetching that root while
still allowing a distinct import of its final response URL. Inline roots supply
no fetched URL identity, so importing the inline script's base URL is a real
dependency request rather than an incorrectly suppressed cycle.

Added regression sources for redirected original/final URL separation and inline
base-URL dependencies under saturated fetch admission. Root-response sharing
across graphs and full parser/direct-import continuation remain open. No builds
or tests ran.

### Consistent graph dependency and cached-source limits

Module graphs now count newly discovered dependencies explicitly, with the same
256-dependency limit for inline and external roots. The initial external root
identity does not consume a dependency slot. Cached JSON and CSS source bodies
consume the graph byte budget at discovery, queued cache adoption, and completion
cache adoption, just as freshly fetched typed sources do.

Regression sources cover inline/external roots at 256 and 257 dependencies and
cached typed-source adoption at the graph byte limit. These sources remain
unrun under the global generation-first gate; this is not a conformance result.

### Retained dynamic-script dispatch

Ordered completions now transfer into a scheduler-owned dispatch queue. A host
error dispatching one script leaves subsequent results queued, with their pending
preparations and source reservations intact. The next page task drains that queue
before admitting more work; queued dispatch itself marks the page runnable. Each
item still passes the existing document-identity check before execution, and
stopping the document drops the scheduler and its retained results together.

Added an unrun regression source for an interrupted ordered batch, successor
retention, ordered resumption, and prevention of duplicate dispatch. Full parser
continuations and native admission/cancellation integration remain open. No
builds or tests ran.

### Graph admission provenance and worker startup failure

Graph discovery now uses only retained native module responses for cached
dependencies. Decoded resources cannot suppress a required fetch or supply an
unverified dependency body for recursive discovery. Updated the cached-cycle
fixture to retain verified child responses, and added a saturated-admission
regression proving a decoded redirect target still queues its own request.

Module fetch workers use fallible thread creation. An unstarted worker releases
its captured fetch permit and publishes a shared error for waiting graphs rather
than panicking the page actor. Added regression source for abandoned closure
ownership, returned capacity, and startup-error visibility to followers after
the producer channel closes. All regression sources remain unrun.

### Parser resource terminal completion ownership

Parser scripts, external/inline stylesheets, and frame resource workers now use
one completion recipe retaining the original document identity and preparation.
Fallible thread creation and caught worker panics both publish terminal failures
through that recipe, allowing the existing scheduler to release pending load
counts and script leases. Unstarted workers no longer panic the page actor;
panicked producers no longer leave the parser waiting on its still-open channel.

Added regression sources for ordinary failure and producer panic, exact identity
preservation, single terminal delivery, and source-reservation release. Sources
remain unrun. Bounded parser worker admission, persistent parser continuation,
and nonblocking cancellation acknowledgement still require integration.

### Bounded parser IO execution

Parser script, frame, and stylesheet jobs now share eight process-owned IO lanes
with a 1,024-job cap covering queued and running work. Admission does not wait
for network completion. Rejected jobs release their captures and use the same
terminal completion recipe; the previous per-resource thread creation path is
removed. Partial executor startup failure stops and joins the lanes already
created, while normal process ownership retains their join handles.

Queued jobs hold a weak scheduler lifetime. If the parser scheduler is dropped
before a job begins, it releases its unexecuted capture and reports retirement
without calling the network loader. Existing document checks still govern result
publication, and native script leases still own already-started cancellation.

Added regression sources for running-plus-queued capacity, rejected-job capture
release, panic recovery, scheduler retirement, and exact terminal completion.
No builds or tests ran. Persistent parser continuation and cancellation of
already-running frame/style requests remain open, as does nonblocking native
script cancellation acknowledgement.

### Parser script admission off the page task

Parser script scheduling now captures native authority and a weak lease owner,
then performs ReserveScriptLoad and RunScriptLoad on the bounded IO executor.
The page task no longer waits for the admission RPC. A parser retired while that
RPC is pending drops its owner; the late token is cancelled on the IO lane and
cannot start the resource request. Parser and module-graph loading share the
same token-publication helper, with existing duplicate/late-token source tests
retained against the shared implementation.

Added a transport-backed regression source delaying admission until after parser
scheduler retirement, checking an empty pending token slot, late cancellation,
and zero RunScriptLoad calls. Updated scheduler lease ownership regressions.
No builds or tests ran. Cancellation of already-published tokens still waits for
its acknowledgement; persistent parser continuation and dynamic-root admission
remain open.

### Bounded dynamic root admission and completion

Dynamic script roots now share the bounded resource IO executor with parser
resources; the executor module was renamed to reflect its integrated scope.
Native ReserveScriptLoad and RunScriptLoad execute on an IO lane using the
shared weak admission owner. Retirement before admission skips the RPC, and
retirement during admission cancels its late token. Broker-only dynamic fetches
also use the executor instead of creating a thread for every script.

One dynamic completion publisher now reserves source bytes, marks runnable work,
and reverses both ownership and ready accounting when delivery is rejected.
Executor rejection and worker panic publish normal terminal failures. Added
regression sources for panic delivery and rejected-completion cleanup. No builds
or tests ran; root-fetch coalescing, persistent parser/direct-import continuation,
and nonblocking cancellation acknowledgement remain open.

### Integrated module IO admission

Module-graph fetch jobs now use the shared bounded resource IO executor rather
than creating per-fetch threads. Their existing eight-request graph admission
limit remains in force, and rejected executor jobs return captured graph permits
and publish a shared terminal failure. The pool therefore covers parser, dynamic
root, and module dependency IO under one process-level job bound.

All three native script paths now call one admission helper: an already-retired
weak owner skips the broker RPC, and retirement during the RPC cancels the late
token before RunScriptLoad. Added a regression source asserting that retired
admission writes no broker request; existing late-token and follower ownership
regressions target the same shared implementation. No builds or tests ran.

### Nonblocking script cancellation custody

ContextScriptLoad now reserves cleanup capacity before requesting a parent token.
Dropping its owner marks abandonment and wakes dedicated cleanup IO lanes; it
does not perform a broker request on the page thread. The process-owned bounded
registry retains the client, route, page, and token until CancelScriptLoad is
acknowledged. Failed or panicked cleanup retries on a bounded cadence, while its
occupied capacity prevents unbounded new admissions. Cleanup lanes are separate
from resource lanes, so saturated fetches cannot consume all cancellation workers.

Added an explicit cleanup drain returning an error while acknowledgements remain
pending, and regression source covering delayed cancellation replies and retry
after failure. Updated existing token-owner regressions to drain acknowledgements
before closing their fixture sockets. Response-only graph fixtures no longer
invent parent tokens. The parent response-custody work separately rolls back
unacknowledged ScriptLoadReserved responses against the original registry.

All sources remain unrun. Transport/route teardown integration for pending cleanup,
persistent parser/direct-import continuation, and root-fetch coalescing remain
open; this source change does not establish shutdown qualification.

### Exact retired-route cancellation authority

Worker routing now retains cancellation-only grants mapped to the original
script registry and page. CancelScriptLoad and its response-scope check accept
that exact grant during draining and after rotation, independently of revoked
network admission. Missing/forged grants and page mismatches fail. Cleanup never
selects the current environment's replacement registry. Retained grants keep no
BrowserContext or network authority and are bounded to 4,096 per router; reused
grants and excess rotation fail admission explicitly.

Added regression sources for cancellation during a failed drain, forged/page
mismatched cleanup, and equal numeric tokens across old/replacement registries.
These remain unrun. Transport-exit cleanup draining and the broader loader
continuation work remain open.

### Worker lifecycle cleanup barriers

Reset, environment release, and explicit worker exit now close the page actor
and drain exact-route script cleanup before publishing their success response.
Timeout returns an error and preserves the session for retry. Reset no longer
takes the session before close/cleanup succeeds. End-of-input cleanup reports
page-close or script-drain failures rather than silently returning success.

The cleanup drain now counts pending-admission tickets and live token owners as
well as abandoned cancellations. Added source regressions for those barrier
states and reset timeout followed by a successful retry after the token owner
is dropped. Sources remain unrun. Resource-job quiescence across transport exit,
persistent parser/direct-import continuation, and root-fetch coalescing still
require integration and qualification.

### Per-route resource IO teardown ownership

SharedContextClient now shares a bounded registry of exact-route resource scopes.
Parser script/style/frame jobs, dynamic native script roots, and module fetch
jobs acquire ownership before executor submission; rejected or discarded work
releases that ownership. Route clones share the same scope, while sibling and
replacement grants have independent scopes.

Worker reset/release/exit and end-of-input teardown now seal the route, wait for
its queued/running resource work, then drain script cleanup under one timeout.
A failed drain keeps the route sealed and its job owners retained. Added source
regressions for pending closures, timeout/retry, sibling/replacement isolation,
and combined resource-job/token reset barriers. No builds or tests ran.

Persistent parser/direct-import continuation and root-fetch coalescing remain
open, along with broader qualification of transport failure and resource cleanup.

### Resource ownership through completion publication

Parser and dynamic-script wrappers now own their route job guard through the
terminal completion publication, including panic recovery. Fetch return alone
does not release teardown ownership. The submitting caller retains a guard owner
through rejected-executor failure publication as well. Module-graph jobs already
hold their guard around both fetching and channel publication.

Added a regression source that pauses the parser completion recipe after fetch
return and verifies that ownership remains held until publication finishes.
No builds or tests ran; this does not close the remaining continuation or
conformance gates.

### External module root response identity

External parser and dynamic module roots now adopt responses through the same
document module map used by dependency fetches. Existing successful bodies and
response policies win over later success/failure; an earlier fetch failure stays
failed. Resource-limit failures do not manufacture a permanent fetch failure.
Dynamic completion reaccounts its adopted source before graph/dispatch ownership.

Parser module authorization no longer treats a decoded resource entry alone as
a verified module response. Added regression sources for first-response body,
redirect URL, credentials/referrer policy retention, sticky failure, and resource
limit distinction. All remain unrun. Concurrent root-fetch coalescing and the
persistent parser/direct-import continuation remain open.

### Completed module-root fetch reuse

Parser scheduling and dynamic-script admission now check the current document
module map before scheduling external module roots. Retained success supplies
the first response URL/body/referrer policy; retained failure supplies a normal
failed completion. Both paths preserve existing source-budget and completion
ordering ownership without admitting a duplicate broker operation.

Added parser regression source for cached success/failure, redirected response
metadata, source-budget release, and absence of broker traffic or script leases.
No builds or tests ran. Simultaneous in-flight roots still require shared-fetch
coalescing; completed-response reuse alone does not close that obligation.

### Parser in-flight module-root coalescing

Parser roots with the same realm, document generation, stream epoch, and URL now
join a single in-flight module request. Followers retain their own script node,
order, asynchronous flag, and document identity; they acquire no extra broker
lease. Completion expands into separate records sharing one Arc-owned verified
source and source-budget reservation. Fetch failures fan out through the same
completion path. Draining either polling or waiting callers preserves pending
counts and removes the in-flight lookup when the leader completes.

Added source regression for follower admission without broker traffic, distinct
script records and scheduling flags, shared source ownership, and budget release.
Parser script admission is bounded including followers. No builds or tests ran.
Dynamic-root/dependency coalescing with parser roots, completed deferred-root
publication, and persistent parser/direct-import continuation remain open.

### Deferred-root module map publication

After the parser completion validates its document lifetime, successful module
roots now publish into the native module map before entering the deferred queue.
Subsequently discovered roots can reuse the completed response without waiting
for script execution. If another root/dependency already published a result,
that first body and response policy are adopted before shared resources or the
deferred record are updated. Matching shared responses retain their original Arc;
replacement adoption acquires its own source reservation.

Added a source regression applying two different responses for one deferred root,
checking first-response preservation, publication before any execution, unchanged
deferred source/referrer policy, and reservation release. No builds or tests ran.
Cross-path in-flight coalescing and persistent parser/direct-import continuation
remain open.

### Module resource provenance

Network-backed hosts now resolve JavaScript dependencies through the native
module map or a real module fetch. A decoded page-resource entry alone no longer
authorizes compilation of a distinct requested URL, including the final URL of
another request's redirect. Explicit dependency maps remain available to hosts
without native network services.

Added a regression source distinguishing a retained redirected request from a
direct request for its final URL, including subsequent dynamic-import reuse.
Moved accidentally nested module regression functions into the enclosing test
module so the later test phase can discover them. No builds or tests ran; direct
import continuation and root-fetch coalescing remain open.

## Typed local parser failures and sticky publication — September 16

Source generation only; no builds/tests/formatters/generators ran. Parser script
completion now distinguishes local source capacity, infrastructure and established
fetch failures. Executor rejection/panic/retirement remains infrastructure; the
source reservation reports typed capacity. Publication first checks document
identity, preserves an existing native result, and only retains an established
fetch failure in the module map. A late fetch failure can adopt the first native
success using the original completion budget.

Regression sources cover local failures leaving the map untouched, sticky fetched
failure, first-success adoption with capacity exhaustion, and stale failures not
poisoning a replacement. The wire still flattens remote errors into strings, so
those remain conservatively infrastructure failures. Typed parent fetch errors,
graph/direct-import error propagation and full cross-path unification remain
required before L-02.i closes. No semantic failure classification uses substrings.

## Standalone fetch and module graph failure provenance — September 16

Generated ScriptLoadFailure variants for fetched failure, infrastructure, cancellation
and capacity, retaining native NetworkError variants without message heuristics.
Standalone metadata loaders return the typed error; host/parser/dynamic consumers
preserve it. Actual network errors remain fetch failures, while unavailable
transport, host policy/replay, cancellation and configured capacity stay distinct.
Decoded source size is bounded in addition to response bytes, and cancellation is
checked before decoding/publication. Runtime capacity reporting preserves limits.

Module graph startup/disconnection/admission and untyped remote RPC errors now
remain infrastructure failures and do not enter the sticky fetched-failure map.
Regression sources updated for disconnected producers, first-result cache
semantics, network variant classification and legacy-decoding expansion capacity.

Parent script RPC still needs explicit typed error publication. Direct/typed import
loading still erases some typed errors and must migrate with persistent async
continuations; this does not close L-02.i or L. No builds/tests/generators ran.

## Parent script failure provenance and consumers — September 16

Context contract 45 now includes ScriptLoadFailed with typed fetch/cancellation/
capacity failures emitted at the native script fetch boundary. Source authority
and admission remain independently checked; their failures remain RPC errors,
which loaders treat as infrastructure rather than sticky module failures.
Parser, dynamic script and module graph consumers preserve the new reply.
Legacy untyped convenience load wrappers are test-only. Added real parent IPC
regression source for fetch/cancellation/capacity, unavailable broker and stale
Creator; updated cancellation/MIME regression sources for typed replies.

Direct and typed-import source loaders still use pending map placeholders and
erase some classification; migrate their ownership and async continuations before
L-02.i closure. No builds/tests/formatters/generators ran.

## Direct JavaScript pending ownership follow-up

`host.rs::PendingModuleFetch` now retains separate document-cache pending identity with RAII release. Pending URL bytes and request counts share the existing 32 MiB/256 cache bounds. Direct native/standalone fetch records only classified established fetch failures; cancellation, capacity, admission, unavailable state and stale environments release pending ownership without failed-map publication. Completed responses are first-wins, including an earlier failure; late successful bodies/policies cannot replace the first response. A late fetch failure reuses an already-published success. Regression sources cover admission/reentry, drop versus first publication, mixed pending/completed capacity and decoded body admission.

Source only, unrun. This is not asynchronous import completion: synchronous host continuation, separate graph/parser ownership, typed JSON/CSS pending state, compiled-module error classification and exhaustive native caller propagation remain open under L-02.g/h/i.

## Typed JSON/CSS pending ownership and error propagation

Typed native response caches now keep pending (requested URL, module type) identities separate from sticky failed fetches. RAII release and combined pending/completed count/byte bounds mirror direct JavaScript ownership. Parent ScriptLoadFailed and standalone ScriptLoadFailure retain their classification; local failure does not publish a failed response or compiled-record error. First response remains authoritative. Compiled typed records retain actual JSON parse errors, release local failures before fallible exception allocation, and charge source bytes only after successful module creation. Added type-separated ownership, first-failure persistence, release and mixed-capacity regression sources; updated superseded pending-as-failure controls.

Unrun. Worker ImportModule still lacks a classified fetch-failure wire response; worker transport errors remain infrastructure rather than guessing by message. Complete asynchronous continuations, unified loader ownership, worker provenance and integration regression coverage remain open.

## Worker import provenance and bounded ownership

WorkerError now carries optional native ScriptFailureKind metadata; constructor/default legacy errors are unclassified infrastructure. Agent script loader construction sites classify fetch/security response rejection, cancellation and capacity, while native network and decoder variants retain original classes. JavaScript and typed worker consumers use the metadata rather than exception name/message. Worker JavaScript no longer inserts directly into the response map: pending admission, combined cache bounds, first publication and post-fetch environment validation use the shared native ownership path. Added wire roundtrip and real agent import retry-versus-sticky-failure regression sources.

Unrun, same pending wire generation. Service interception/blob failure producers and full entry/bootstrap caller mapping still require reconciliation; this does not close L02 or asynchronous continuation work.

## Worker redirect compilation identity

Worker compilation now receives requested and final URLs separately. ModuleCache and ModuleOwners use the requested URL; ScriptOrigin, ModuleNames and import.meta use the response URL. Redirect aliases and a direct final-URL import therefore retain independent module identity while repeated identical requests reuse one module. Worker referrer policy is retained per requested response and copied into each module's credential record, avoiding final-URL collisions between aliases. Added `worker_redirect_aliases_keep_requested_identity_and_final_import_meta` regression source alongside existing relative redirect resolution coverage. No tests executed.

## Blob module lookup provenance

Native blob lookup tags established missing/revoked and foreign-resource rejection as fetch failure only after caller authority validation. A stale caller remains an unclassified authority/infrastructure error; successful lookup revalidates the caller before returning owned bytes. Blob script CSP rejection carries explicit fetch provenance. Added stale-caller versus missing/foreign resource regression source. Resource-creator retirement currently combines lookup and infrastructure failures and remains unclassified pending a more precise native authority result; service-event response errors also need producer classification. No tests run.

## Service-event loader error boundaries

Native rejected respondWith/body promises now emit explicit fetch provenance; explicit service-event cancellation emits Cancelled. Non-streaming body overflow is Capacity, separated from invalid body values. Existing real service-event regression sources now assert fetch and cancellation provenance. Broad extraction/stream errors remain unclassified because the current V8BindingError paths combine local allocation/admission with author response errors; they are not guessed from text. No tests executed.

## Dynamic producer integration before parser continuation

Source inspection of the parser/task boundary found remaining generic dynamic-producer panic, executor rejection and local admission errors that could still become sticky module failures. Those producers now emit typed infrastructure failures. Extended the existing panic/one-completion regression to pass its actual completion through native module adoption and assert no failed module-map entry. Parser construction still consumes a blocking DocumentStream iterator and waits for resources before returning the executable; completing R06 requires changing that constructor/task ownership boundary, not merely replacing `recv` with polling. No builds/tests run.

## Owned live parser continuation foundation

The former monolithic parser locals now live in `LiveParserState`: tokenizer, input iterator/suffixes, encoding, native script leases, parser events, deferred records and resource scheduler. `resume` validates document/realm/generation/epoch and yields after one input slice; EOF with outstanding resources returns an owned Waiting state without calling recv. Resource waiting remains in the explicit transitional `parse_live_document` constructor adapter. Generated regressions cover state preservation between input slices, author execution between steps, pending-resource return and scheduler custody on drop.

R06 remains open: DocumentStream::next can still block; parser-blocking scripts and module preload still use synchronous paths; executable construction still drives the adapter before publishing the document. These must all move to the native task owner together before claiming asynchronous parser completion. No tests/builds run.

## Pollable navigation input

DocumentStream now owns a DocumentInput producer whose poll distinguishes Pending, a decoded chunk, and EOF. NavigationBodyChunks uses try_recv without internal sleeps, preserves one outstanding body demand across repeated empty polls, and bounds decoder-only progress per poll. The live parser yields InputWaiting without marking parsing complete; finite compatibility inputs use ReadyDocumentInput. Added native demand/partial-decode and parser pending-versus-EOF regression sources.

The constructor adapter still polls with a wait before returning the executable. Parser-blocking scripts/module preloading remain synchronous. Navigation cancellation/Drop still retains its prior explicit broker cancellation until the service-intercepted producer has equivalent receiver-closure ACK custody; the authority lane is implementing that boundary. These remaining items prevent R06 closure. No builds/tests executed.

## Pending parser-blocking classic scripts

LiveParserState now retains the exact scheduled external classic script order and source suffix, returns while its native fetch is pending, and resumes the original insertion point on completion. Success uses the prepared response; failure reports the existing script error and releases the parser without re-fetching. Shared insertion-point continuation preserves document.write ordering. Generated regression source injects success/failure through the actual scheduler completion path and checks that suffix/network input remain unavailable until resume. Continuation publication rechecks native document lifetime after other resource callbacks.

The production constructor still synchronously drives these steps. Standalone broker adapters, module preloading, full stylesheet-blocking rules and native executable/task ownership remain open; this source change does not close R06.

NavigationBodyChunks no longer calls broker cancellation from poll or Drop. Original receiver closure now transfers cleanup to the native identity owner or the service-stream custody generated by the authority lane, retaining cancellation/terminal acknowledgement responsibility. No tests/builds executed.

## Parser stylesheet custody and shared admission

ParserResourceScheduler now retains each pending stylesheet owner/revision with a reference count until its completion is consumed. Duplicate work for one revision and obsolete revisions cannot silently erase another pending owner. Style, inline-style, frame and script admission now share the existing 1024 pending-resource bound; inline-style admission failures propagate rather than silently admitting excess work. Added regression source for duplicate/revision completion ordering and shared capacity. Native stylesheet blocking eligibility is being generated with the layout lane; the tracking alone is not a script-blocking decision. No tests/builds run.


## Native stylesheet gating and parser retirement checks

The parser now uses `RawV8Host::native_stylesheet_blocks_scripts` for each current pending owner/revision. The native predicate checks connected document/light-tree ownership, CSS type, disabled/alternate state and parsed native media. Inline classic scripts retain their prepared record and insertion-point suffix; ready external classic scripts use the same gate. A nonblocking owner stops delaying script execution without abandoning its outstanding fetch custody.

Generated `parser_inline_script_waits_for_style_and_resumes_after_native_applicability_changes`: actual parser script preparation, pending stylesheet wait, success/failure completion or media/disabled/removal release, exactly-once execution and document.write insertion-point continuation. Generated retired-child-realm input regression proves the continuation rejects retirement before polling or publishing and drops its input owner. Lifetime is rechecked after resource callbacks and microtask checkpoints before further parser publication.

Source inspection only; none of these regressions have run. R06 remains open for persistent executable/task ownership, module preload waiting and standalone synchronous paths. Named stylesheet set selection and media change events are continuing in the S/V lane. This updates earlier notes that described stylesheet eligibility and navigation cancellation custody as pending.


## Non-HTML navigation input reconciliation

Source inspection found the XML/resource-document branch still iterating `DocumentStream.chunks` after its migration to `DocumentInput`. Replaced that superseded caller with `BufferedNavigationInput`, which owns the stream, accumulated decoded source and response URL. Each poll consumes at most one chunk; pending input cannot be mistaken for EOF. Completion parses by the original MIME type and applies the final decoder encoding. Error/completion drops the input producer exactly once and discards buffered source; subsequent polls cannot repoll a terminal producer.

Generated `buffered_xml_navigation_preserves_pending_encoding_and_terminal_input_custody` for split XML with intervening Pending states, late encoding metadata, cancellation propagation, source release and no terminal repoll. The non-HTML constructor now uses the same explicit transitional polling adapter as the HTML continuation. It still needs the native pending-load task owner; this is caller reconciliation, not R06 completion. No builds/tests/formatters executed.


## Executable-owned parser and shared completion publication

Moved live parser ownership into `RustExecutableDocument.pending_parser`. The constructor creates the executable before driving parser slices; `resume_pending_parser` retains waiting state and publishes partial DOM snapshots, then uses shared `finish_parser_resources` and `finish_parser_lifecycle` on completion. Removed the old standalone `parse_live_document` wait adapter. Pending parsing participates in pending-work and load-complete queries; stop drops the parser owner. Parser errors release the input and keep document load incomplete. Completion publication rechecks parent lifetime across prepared frame/resource and deferred script callbacks.

Generated executable-owner regressions for pending input with unrelated author work, partial DOM publication, EOF/load completion, and failed-input drop without false load completion. Initial DOM installation now takes the host's native root generation rather than a hard-coded caller value, aligning with the authority lane's parent-issued generation.

R06 is still open: `load_with_options` drives the executable synchronously; PageState network response capture/auxiliary-resource bookkeeping still requires a completed response before returning. Native page-task admission/readiness scheduling, XML pending input placement, and synchronous module/frame completion consumers must be reconciled before claiming asynchronous loading. No builds/tests run.


## Native page-task parser slices and deferred interactive publication

`drain_tasks` and `poll_context_events` now resume one executable-owned parser slice after their existing task/checkpoint work. Synchronous DOM checkpoints do not independently consume parser input. Generated `native_page_task_polls_resume_parser_after_other_tasks_without_draining_all_input` exercises pending input, a due timer, one chunk per task poll and an unrelated action between chunks. The executable's native `document_interactive` query excludes pending/stopped parsing and checks native readiness; PageState initial and later lifecycle publication now use that query so a pending parser cannot produce an early Interactive milestone.

The page actor already calls these task entry points, but production construction still waits before returning the executable. The next integration boundary is deferring completed-response capture, resource/font discovery and navigation-result bookkeeping while the actor retains the partially loaded executable. This has not been claimed complete. No tests/builds executed.


## Controlled navigation returns an executable with pending parsing

Native stream constructors now accept scheduled loading; PageState selects it for controlled navigation. HTML/text construction returns the executable without polling the input. Existing synchronous source/legacy navigation callers retain their explicit compatibility drive. Native page task entry points resume input, while `PendingNavigationResponse` retains HTTP/data source metadata bound to the PageState generation and exact native root realm/generation/stream epoch. Completed response accounting, challenge/referrer publication and auxiliary resource/font discovery move into a shared completion consumer. It takes the capture out of its mutex before calling further services. Replacement, document.open identity changes and stop cannot publish that old response.

PageState keeps lifecycle milestones pending until response completion bookkeeping settles, then publishes native interactive/load state. Page-task errors stop the active navigation rather than only logging a diagnostic. Root form-history restoration requested during pending parsing is retained for the matching native document and applied after prepared scripts before load lifecycle. Added source regressions for constructor return without input polling, scheduled stop releasing input, deferred capture retention and generation/stream replacement rejection.

Remaining R06 generation: XML/resource-document preprocessing still drives its collector before construction; compatibility/direct navigation paths still block; headers/redirect acquisition, module graph preload, prepared frame completion and auxiliary network consumers still have synchronous work; pending BFCache/history and all wakeup/order/error cases need full reconciliation. No builds/tests run and no whole R06 completion claim.


## Pending navigation and BFCache eligibility

Unfinished executables no longer enter the back/forward cache with a live parser whose response record/emitter belongs to the replaced navigation. Suspension dispatches a nonpersisted pagehide, captures bounded form history, stops the pending load and releases its input/executable; the history entry remains and a back traversal must reload it. Generated `replacing_unfinished_document_releases_input_and_forces_history_reload` verifies no extra input poll, source drop, no cached executable and retained history. Queued root form restoration now validates snapshot bounds before cloning; stop also drops pending child form restoration.

XML publication review: importing parsed XML must retain the existing document wrapper/arena and remap every parser-event node; swapping the complete arena could alias author-retained nodes. That integration remains open. Cached-restore rollback with an ineligible incumbent also needs reconciliation before broad history closure. Source inspection only, no execution.


## Executable-owned XML/resource input and stable document publication

Removed the non-HTML pre-construction collector wait. Buffered inputs now belong to `RustExecutableDocument.pending_buffered` with the native realm/generation/stream/document identity captured before input polling. Controlled XML/resource construction returns without reading input; the same native task resume path collects one chunk at a time, checks lifetime again at completion, publishes, and uses shared script/frame/lifecycle completion. Pending queries, stop/error cleanup and delayed form restoration cover both HTML and buffered input.

`publish_buffered_navigation_document` imports nodes into the existing arena, remaps every node-bearing parser-event variant, retains the original document node and native URL/base metadata, and detaches old children without reusing their identities. XML MIME/document kind is installed before exposing the document. Added scheduled XML regression source for no constructor polling, retained document/node wrappers, pending chunks, namespace/MIME/URL preservation and completion; added parser-event import identity regression.

Remaining: XML parsing itself remains a whole-source parse at EOF; full XML parser-script timing, malformed/import failure atomicity and non-HTML resource edge cases need reconciliation. Compatibility/direct loading, headers/redirect acquisition, module/frame/auxiliary consumers and ineligible-incumbent cache rollback remain tracked R06/R04 gaps. No builds/tests/generators executed.


## Cache restoration rollback retains an unfinished incumbent transactionally

Cache entries now distinguish reusable documents from temporary rollback-only ownership. A restore attempt can stop and suspend an unfinished incumbent without making it available to normal history traversal. Parent CommitPage failure now enters the same rollback path as native authority restoration failure (previously it returned early and stranded the incumbent). Rollback restores the exact retained document/authority and uses nonpersisted pageshow for an ineligible document; successful destination restoration removes the temporary incumbent. Removed the duplicate parent commit after native destination activation.

Generated regression sources exercise a real IPC parent-commit failure followed by successful rollback, exact document/history recovery without reviving input, and successful cached restoration discarding its temporary incumbent. Existing ordinary unfinished-load replacement still discards the executable and reloads on history traversal. The broad cache/security/rollback matrix remains unverified; this closes the specific generated-path hole recorded above. No tests/builds run.

## Retained deferred parser-module completion phase

- Generated `PendingParserCompletion`: deferred script queue, native script leases, initial frame lifecycle ownership, and a retained `ModuleGraphLoad`. Prepared parent modules poll dependencies on native tasks; following deferred scripts remain ordered and document completion waits for the queue. Stop/error paths release retained graph and script custody.
- Module preparation uses native inline authorization, immutable captured fetch policy, first-response module adoption and shared graph admission/source limits. Successful preloading is recorded privately so execution does not repeat the synchronous graph wait.
- Source review found dependency admission preceded prepared-script ownership validation. Factored `parser_script_identity_live` and applied it before graph preparation/poll and before execution; adoption away discards the retained graph and skips the obsolete record.
- Generated `deferred_parser_module_graph_yields_and_preserves_following_script_order`: saturated admission yields; unrelated author work runs; a retained dependency response releases ordered execution; an adopted-away module causes neither graph admission nor a script-error diagnostic.
- Source-only evidence. No build or test execution. Frame/snapshot/unprepared module waits, frame installation, async-fetch lifecycle barriers and broader task scheduling remain open. This does not close R-06 or L package gates.

## Lifecycle fetch barriers use native pending-load ownership

- Removed remaining `drain_async_fetches(true)` calls from `rust_executable.rs`: initial snapshot completion, retained parser completion, initial frame/top lifecycle, and dynamic child-frame lifecycle now deliver only ready fetch completions. Ordinary fetch promises are not document-load blockers.
- Source-inspected `flush_document_loads`: exact realm/generation/epoch checks and explicit pending dynamic scripts, images, frame navigation, stylesheet events and descendant loads retain their load barriers. Task checkpoints keep polling completions.
- Extended `broker_dynamic_script_does_not_block_page_and_stop_cancels_native_operation` regression source to enter lifecycle scheduling with a stalled native script operation, assert the explicit document-load barrier remains, run an unrelated timer and cancel through native custody.
- No execution. Synchronous graph/frame preparation and other network consumers remain open; this change removes lifecycle fetch waits, not every blocking network path.

## Lifecycle replacement identity and truthful load completion

- Source review found that ready callbacks/timers inside `finish_parser_lifecycle` can call `document.open()`, after which the old completion path queued lifecycle against the replacement stream. Snapshot native root and initial child identities before callbacks; recheck root and child identity before child publication and root identity after lifecycle checkpoints before top publication. Obsolete scheduling returns without completing the replacement.
- Removed the redundant pre-lifecycle fetch drain in retained parser completion so lifecycle owns its callback boundary and identity capture.
- `document_load_complete` now requires native Complete readiness as well as no parser/pending-load/stopped state. An unclosed document.open stream has no queued barrier and must still report incomplete.
- Generated `parser_lifecycle_timer_replacement_does_not_complete_unclosed_new_stream`: timer opens/writes without close, old completion leaves loading and no events, explicit close subsequently emits exactly one DOMContentLoaded/load pair. No execution. Capturing initial frame identity at preparation (rather than only lifecycle entry), full replacement/task matrices and synchronous frame graph preparation remain open.

## Prepared frame lifecycle retains original stream identity

- Replaced initial frame realm-only lists with `PreparedDocumentLoad`, capturing native document root/generation/stream epoch/liveness before child scripts execute. Retained parser completion carries these identities through deferred parent execution; lifecycle no longer recaptures authority from a replacement stream.
- Dynamic child preparation captures identity before imported script execution. Its mapping is retained, but obsolete load/refresh work is discarded after callbacks replace the stream; load handlers also cannot transfer old refresh actions to a replacement.
- Generated regression sources `parent_script_replacing_prepared_child_does_not_complete_its_new_stream` and `dynamic_child_replacing_its_stream_does_not_receive_obsolete_load`, each checking an unclosed replacement remains loading without old events and closes with one lifecycle pair.
- Source-only changes; no build/tests run. This resolves the preparation-vs-completion identity gap noted above, while asynchronous frame parsing/module graphs and broader nested-frame scheduling remain open.

## Unclosed replacement child retains parent load barrier

- `flush_document_loads` now considers live frame documents whose native readiness is not Complete, including replacement streams that have not called document.close and therefore have no queued completion entry. It checks connected ancestor frames and bounds ancestry traversal with visited realms.
- Extended parent-driven replacement regression to require parent incomplete before child close and complete afterward. Added `removing_unclosed_child_releases_parent_load_barrier` to require removal releases the barrier and parent load dispatches only once.
- Source-only evidence; no execution. Full nested-document stop/navigation ordering remains part of the open R-06/L integration audit.

## Stopped descendant load state

- Added native `BindingState.document_load_stopped`, set after loader cancellation and cleared when document stream epoch advances. Host exposes the native flag; parent readiness barriers exclude stopped descendants, and stale queued lifecycle entries are removed without events. `queue_document_load` rejects stopped-stream publication.
- Generated `stopping_unclosed_child_releases_parent_without_completing_child`: stopping a Loading child releases its parent, does not synthesize child completion, and a new open/write/close stream can complete normally. Extended binding-level cancellation regression with stopped/reset state assertions.
- No build/test execution. Window.stop API coverage, nested cancellation propagation, pending frame-network teardown and broader lifecycle qualification remain open.

## Native Window.stop binding and parser cancellation

- Added native Window.stop with Window receiver branding, access checks, non-constructible method shape and native loader cancellation. No author-visible shadow state controls cancellation.
- Live parser identity checkpoints and prepared parser-script ownership checks reject stopped streams. Executable parser cancellation caused by stop releases pending state and returns the still-live document rather than a navigation error. Ordinary checkpoints transfer root stop to executable-owned queues.
- Generated `native_window_stop_aborts_parser_suffix_and_preserves_script_execution` covering suffix suppression, ongoing script execution, illegal receiver, and fresh document.open/write/close recovery. Child-stop regression now uses the actual Window method.
- Source-only; no execution. Full child executable-owned queue cancellation, nested window.stop propagation, and all WPT stop/reentrancy edge cases remain open.

## Dynamic-script stop cancellation epoch

- Added checked native load cancellation epoch to BindingState and immutable PreparedDynamicScript captures. Stop invalidates all already-prepared dynamic scripts for that document, including preparations owned by the executable scheduler rather than the binding queue. New scripts prepared after stop capture the new epoch and remain executable.
- Existing scheduler currentness checks now release obsolete script leases, graphs and source reservations without poisoning module fetch caches. Sibling documents retain their independent counters.
- Generated `child_stop_invalidates_old_dynamic_preparations_but_accepts_new_ones`: old child preparation becomes stale, fresh preparation survives, a late old response is ignored, fresh response executes and queued source bytes return to zero. No execution.

## Observe root stop without repeating cancellation

- Source review found that checkpoint/parser cancellation handoff called the public executable stop routine after native Window.stop had already advanced cancellation ownership. The second call incorrectly invalidated dynamic scripts inserted after stop in the same author callback.
- Added `observe_native_root_stop` to release parser/buffer/completion/history ownership and old root load publication without issuing cancellation again or resetting the dynamic scheduler. Existing preparation-currentness checks retire pre-stop work individually. Explicit external executable stop retains its immediate cancellation behavior.
- Generated `root_stop_observation_preserves_scripts_prepared_after_native_stop`: old preparation retires, fresh preparation remains current and executes, and the stopped document does not falsely report load completion. No execution.

## Expected parser cancellation after stream replacement

- `resume_pending_parser` captures the pending parser/buffer/completion identity before polling. A Cancelled result with a confirmed live same-document/generation but different stream epoch drops obsolete parser ownership without marking the replacement as failed. Unrelated cancellation/errors remain failures; native stop still uses its own stopped path.
- Replacement close/lifecycle work runs through the existing checkpoint, preserving open replacement streams and exactly-once close behavior.
- Generated `replacing_pending_parser_stream_discards_input_without_failing_new_document`, covering both open and already-closed replacement streams, no obsolete input poll, input drop, preserved DOM, truthful readiness and exactly one load event. Source only; no execution.

## Snapshot constructor stop reconciliation

- Source inspection found snapshot `finish_parser_resources` propagated intentional native stop as Cancelled while live parser ownership handled it. Added stopped-parent checks before identity checkpoints in frame/script preparation loops, ending obsolete preparation normally.
- Lifecycle entry observes an already-stopped root without synthesizing completion, then checkpoints current work without issuing another stop.
- Generated `snapshot_window_stop_skips_remaining_scripts_without_constructor_failure`: stop skips subsequent scripts, retains pre-parsed DOM, preserves ongoing author execution and allows a replacement stream to complete. No execution. Synchronous snapshot/frame module preparation remains R-06.j.

## Deferred module preparation commits before yielding

- Source review found retained parser-module graph loading could yield before execute_script_record marked the node started. Now graph admission marks native started state first, preventing mutation-driven duplicate preparation while dependencies are pending. Empty inline module records bypass graph admission, matching normal empty-script preparation behavior.
- Extended `deferred_parser_module_graph_yields_and_preserves_following_script_order` to assert native started state and mutate the pending script type/text; the captured module source and following script order remain authoritative, with no dynamic re-preparation queued. No execution.

## Snapshot deferred script integration

- Snapshot constructor now partitions captured deferred records from immediate scripts, runs immediate preparation first and transfers deferred records/leases/initial frame lifecycle identities into PendingParserCompletion. Deferred inline parent modules therefore use the retained pollable graph path when context-backed, instead of the old direct snapshot module wait.
- Stopped snapshots discard deferred records without publishing Interactive. Non-stopped deferred snapshots publish Interactive before the deferred queue, then use shared lifecycle completion.
- Generated `snapshot_deferred_scripts_follow_classics_in_preparation_order` covering module + external defer + later classic + DOMContentLoaded ordering. Source only; no execution. Snapshot external sources lacking prepared responses, async scripts, frame script scheduling and direct compatibility constructor waiting remain open.

## Deferred external source continuation

- PendingParserCompletion now retains an optional ParserResourceScheduler for context-backed external records lacking prepared source. Source admission returns to page tasks; ready responses flow through existing verified parser completion handling before ordered execution/module graph polling. Captured base/referrer policy override mutable current attributes at admission.
- Stop/replacement/adoption drops retained source scheduler custody. Parser source publication now also checks the prepared script's exact ownership before adopting module results or publishing resource entries. Pending source work does not unconditionally advertise itself as runnable; task readiness/wakeup integration remains under the broader R-06 audit.
- Extended deferred graph regression source with a pending external source scheduler, explicit verified completion and ordered execution. No build/tests run. Frame/async module paths, direct compatibility waiting, and complete wakeup/admission/failure regression matrices remain open.

## Native parser completion readiness

- Added `ParserResourceScheduler::has_ready_completion`, which polls at most one channel item into retained completion custody. Readiness queries do not decrement pending counts, release leases or consume source reservations; `try_next` remains the consumption boundary.
- Executable runnable detection now includes ready live-parser resources and retained deferred-source responses. A source still waiting on I/O remains non-runnable; graph and existing page work keep their independent readiness paths.
- Extended deferred-source regression source with false-before-response, true-after-response and repeated readiness observations retaining pending count. No execution. Poll-loop notification latency and complete fairness/consumer integration remain R-06.j/k/l.

## Ordering admission and snapshot preparation reconciliation

- Parser script order allocation now uses checked addition and propagates a typed limit before reuse. Generated `parser_script_order_exhaustion_rejects_without_reusing_identity` covers the terminal boundary and repeated rejection without admitted work.
- Snapshot deferred executable scripts commit native started state before immediate classics run; mutation of a captured deferred module cannot dynamically prepare it again. Extended snapshot ordering regression with a later classic mutating the deferred module source.
- Deferred source admission now filters executable module/classic types and nomodule suppression before external fetch, preserving inert script-data behavior. No execution.

### Cross-package semantic correction underway

Pinned opening-the-input-stream WPT sources establish that document.open preserves queued tasks/global identity. P/W is separating stable native document capability ownership from parser-stream epoch; prior whole-stream retirement assumptions require caller/test reconciliation. Existing parser-stream checks remain applicable to obsolete parser/load publication, but cannot prove persistent capability retirement is correct. Whole R/P/W gates remain open.

## Stable document authority integration in native host

- Host history Register/Park/Restore and parent/child frame policy publication now use document_authority_epoch; parser, lifecycle and DOM-bound automation freshness retain document_stream_epoch. Automation realm inventory exposes both identities for service-client ownership versus command freshness.
- Pending WebDriver script callbacks now retain stable document authority, so document.open alone does not invalidate their completion. Replaced the previous incorrect stream-retirement regression expectation. Node/AX/print/command boundary reconciliation remains separate.
- Generated `document_open_preserves_queued_global_tasks_and_message_port_listeners` based on pinned opening-the-input-stream/tasks.window.js: timer, toBlob, port callback, window messages queued before/after open, and unchanged Window/Document identity. No execution. P/W owns binding/IPC stable authority publication and remaining consumer separation.

## WebDriver reference lifetime across document.open

- Cached Window and element/shadow references now capture stable document authority rather than parser-stream epoch. Incoming command handle maps and DOM-bound operation snapshots still require current stream freshness; cached nodes still require the same live owner/document and native connectivity.
- Generated `pending_script_window_result_survives_document_open` and `retained_element_reference_tracks_connectivity_across_document_open`: queued Window serialization preserves the native handle; detached old node is stale and reinsertion into the same document restores the same element reference. No execution.
- P/W completed the source ownership split documented in DOCUMENT-OPEN-CAPABILITY-LIFETIME-2026-09-16.md; broad caller and runtime qualification remain pending.

## Buffered navigation source allocation bounds

- BufferedNavigationInput now receives the constructor's configured max_heap_bytes as its decoded-source byte ceiling. Checked length addition and try_reserve reject oversized/allocation-failed appends with a typed limit error before source publication. This is a source-buffer ceiling, not a claim that total native plus V8 memory is accounted together.
- Error and terminal parsing paths replace the accumulated String, releasing its capacity; overflow drops the owned input immediately.
- Generated `buffered_navigation_limit_releases_source_and_input_at_overflow` covering partial accumulation, rejected next chunk, native input drop, zero retained source capacity and terminal repoll rejection. No execution.

## Buffered publication preflight

- Source and destination document metadata are now required before buffered publication imports nodes or detaches the existing tree. Previously a missing destination metadata record could be detected after removal; missing source metadata was silently skipped.
- Generated `buffered_publication_rejects_missing_metadata_before_changing_live_tree` checks document identity, retained node identity and connectivity after malformed source rejection. No execution. General import/allocation rollback and complete XML parse timing remain open.

## Buffered import edge fidelity

- Source inspection of DomRuntime::import_subtree showed separate inert template-content edges and intentional exclusion of hosted frame Document children. Buffered publication remapping now mirrors those edges instead of assuming the raw child vectors describe the complete imported tree.
- Template descendants now participate in metadata remapping (including stylesheet overrides) while retaining their inert owner document; malformed template-edge mismatches fail before live tree detachment.
- Generated `buffered_publication_preserves_inert_template_metadata` checks stylesheet override preservation, disconnected template content and its distinct owner document. No execution. General partial-import allocation cleanup/rollback remains open.

## Iterative subtree import and owner-document caller reconciliation

- Replaced recursive DomRuntime::import_subtree with an explicit copy/expand/attach task stack. Template contents are still copied before children; attachment remains postorder so existing adoption and form reconciliation run in the same order. Hosted Document child edges remain excluded.
- Iterative preflight rejects stale source nodes and cyclic/repeated source tree identities before destination allocation. Generated deep-tree (4096 levels) and malformed cyclic-template regression sources; existing template/form/frame import cases remain applicable. This does not provide general rollback for later allocation/attachment failures or eliminate repeated subtree adoption costs.
- Reconciled nine image/script/stylesheet ownership guards in v8_bindings.rs and rust_executable.rs with native node_document's Result<Option<NodeId>> API: flatten the successful optional owner before comparing native document identity. Missing/stale owners still fail closed.
- Source inspection only; all regression sources remain unexecuted. Full generation, build, tests, conformance and qualification remain open.

## Buffered forest and event preflight

- Buffered publication now validates all source roots, ordinary/template descendants, attribute identities and parser-event references before importing the first subtree. Repeated tree identities, non-Document source roots, nested root Documents and unimported event targets reject before live-arena allocation.
- Generated `buffered_publication_rejects_late_invalid_sources_without_allocating` covers a duplicate later root and an event pointing to an unattached source node; both require unchanged arena allocation count, original connected node identity and document identity.
- Corrected the inert-template regression's optional owner-document comparison to the native API shape. Source-only review; no tests run. General failure rollback after successful preflight and destination-side allocation/attachment still remains open.

## Iterative native reclamation

- The deep-import review exposed a second recursive path during native tree release. Replaced recursive tree destruction and internal-edge reclamation with explicit check/expand/remove tasks; reachability selection is now a separate read-only operation with ancestry/descendant cycle checks.
- Cleanup still removes ordinary/attribute/hosted descendants before parents, clears native bookkeeping, and rechecks template, stylesheet and accessibility targets against live roots. Already reclaimed pending targets are skipped by generational identity.
- Extended the 4096-level import regression to release mutation roots and the final native lease. Added a 512-level template graph regression that retains a deep leaf wrapper, requires ancestor survival, then releases the wrapper and requires every unreachable template/content node to be reclaimed.
- Source inspection only; no tests/builds run. Remaining synchronous module callers were inspected in parser completion, frame preparation and compatibility entry paths; R-06.j remains open and no blocking adapter was falsely marked removed.

## Async parser module dependency continuations

- ParserResourceScheduler now retains async module records and ModuleGraphLoad instances after common source publication/owner validation. Inline async modules join the same queue; network source completions no longer call the synchronous module preloader on the live-parser path.
- Each parser resume polls dependency graphs, discards adopted/retired preparations before dependency work, and marks completed graphs preloaded before execution. Pending graphs participate in runnable detection and retain parser completion custody; capacity includes graph entries and graph source reservations remain owned.
- Extended the parser module regression source with an async external completion while all dependency lanes are occupied, unrelated author work, delayed dependency cache publication, exactly-once module execution and adoption cancellation before dependency admission.
- Reconciled parser preparation/fetch identity constructors with optional native owner documents; missing owners now return explicit errors instead of constructing invalid identities.
- Still open: frame/compatibility synchronous callers, shared page-wide admission reconciliation, and separating async load blockers from parser/DOMContentLoaded completion. The existing parser waits for its resource set at EOF; this change yields network waits but does not claim that final ordering algorithm complete. No builds or tests run.

## Async graph handoff at parser EOF

- LiveParseResult now transfers pending async module graphs and explicit native script leases into executable-owned PendingParserAsyncModules. Graph completion no longer holds parser_pending or deferred/DOMContentLoaded completion after EOF.
- Page checkpoints poll the detached graph owner; pending-work/runnable detection, document-load completion and per-realm load barriers account for exact live script ownership. Stop drops the transferred scheduler and leases; stale/adopted preparations are filtered before further graph work.
- Extended the module regression source with scheduled HTML containing an inline async module, saturated dependency lanes, parsed following content and DOMContentLoaded before graph completion. Cache publication must produce dependency → module → load; an alternate stop path must drop ownership without completing load.
- Remaining: external async root network responses are still part of the parser resource EOF wait, plus frame/compatibility source consumers and complete scheduling/fairness reconciliation. This handoff closes the generated dependency-graph wait path, not the entire R-06 algorithm. No execution.

## External async source handoff at EOF

- ParserResourceScheduler now tracks async root preparations separately from the total resource count, including cached module roots and module-map followers. EOF waits only for non-async resources, then transfers unresolved async sources together with graphs and native node leases to page work.
- Ready async sources still pass through common exact-owner validation/adoption/error dispatch before classic execution or module graph admission. Source tracking is removed only on completion consumption; admission ownership stays in the transferred scheduler. Page load checks and readiness now include live pending roots.
- Page polling releases a wholly retired source/graph batch without waiting for network results; stop drops its receiver and native custody. Mixed batches retain bounded pending custody without treating a stale source as perpetually runnable.
- Added scheduled regression sources for an unresolved external async classic through EOF/DOMContentLoaded, later success and load ordering, source failure and load release, and stop rejecting late response delivery. Existing graph handoff/adoption sources remain applicable.
- This supersedes the preceding note's remaining external-async-root EOF wait. Frame/compatibility consumers, page-wide admission reconciliation, complete ordering/fairness and qualification remain open. No builds/tests run.

## Shared parser/dynamic page limits

- Production live-parser construction now installs ParserResourceScheduler::with_page_limits before the first input poll. Parser source responses, async graphs and dynamic scripts share the 32 MiB queued-source accounting, module dependency permits and native in-flight graph registry. Deferred root source schedulers use the same page limits.
- Explicit stop now preserves the graph registry as well as permit/source accounting when replacing the dynamic scheduler. Already retained reservations still count until their owners release them; new work cannot obtain a fresh allowance through stop.
- Generated `parser_and_dynamic_sources_share_page_limits_across_stop`: a parser-owned response fills remaining capacity, dynamic source retention fails, stop preserves common identities/capacity, and releasing the old response permits new work. No build/test execution.
- Remaining frame/compatibility preloading and full operation-count/fairness reconciliation are still R-06.j/R-06.k. This closes common budget wiring for the generated live-parser/deferred/dynamic paths, not every caller.

## Capture pixels and DOM copying

- Reviewed the media agent's clone/import concern against current source: both paths allocate fresh records and explicitly copy fields, so media_frame/image_dimensions already remain empty. Native rendering snapshots deliberately preserve those fields for the original node.
- Added `cloning_and_importing_media_do_not_copy_live_capture_pixels` for shallow/deep clone, cross-arena import, preserved authored width and preserved original snapshot pixels. No production reset was necessary; regression source remains unrun.

## Bounded async graph/source polling

- Async graph polling now advances a persistent round-robin cursor through at most 64 entries per call. A stalled prefix cannot indefinitely hide a ready later graph; source completion delivery after EOF is separately limited to 64 ready responses per poll.
- The original active realm is restored on both successful and failing graph-poll paths. Pending records/graphs remain retained between slices.
- Extended the parser module regression with 64 dependency-stalled graphs ahead of a ready graph, requiring the latter to execute on the next slice while the stalled graphs remain owned. No tests/builds run.
- Delegated the full frame-script continuation generation to the former S/V lane with explicit source-region coordination; root retains parser scheduler ownership. Frame/compatibility removal remains open until its production consumers and regression sources are reconciled.

## XML event slices after input EOF

- Source review of pinned xml 1.4.0 ParserConfig documents incomplete premature-EOF recovery (including PCDATA). That option remains disabled; arbitrary network splits must not silently change XML parsing semantics.
- Extracted owned XmlDocumentParser/XmlParseStep state retaining reader, namespace/parser state, tree sink and element stack. Compatibility parse uses the same event projection; buffered navigation moves its source into the owned reader at EOF and advances at most 256 events per page poll.
- Buffered task readiness includes remaining XML events. Existing stop/error ownership drops the parser, reader and unpublished tree; source encoding is captured at EOF and applied only on completed publication.
- Generated XML multi-slice namespace/CDATA and late-malformed-error regressions, plus executable page-work/stop and 600-element completion coverage. Updated the earlier small XML terminal regression for the explicit EOF-to-parser yield. No builds/tests run.
- Still open: network-time XML/script execution, token-level scanning fairness (one large token can consume work within a single event), and full parser algorithm coverage. Event slices are not a wall-time guarantee or a claim of streaming XML conformance.

## Ready resources while HTML input is pending

- Moved a bounded ready-resource drain ahead of the next HTML input poll. Previously, a parser without a blocking script returned InputWaiting before consuming async source completions, coupling independent resource progress to the next HTML network chunk.
- Blocking-script readiness still uses the same consumed completion identity and stylesheet barriers. Each completion rechecks parser lifetime before touching the old input; async module graphs retain their page-task scheduling.
- Generated `ready_async_sources_run_while_html_input_is_pending` for ready classic execution during a stalled body, adoption suppression and document.open invalidation before another old-input poll. No builds/tests run; full task-source ordering remains open.

## One ready-response budget per parser resume

- A parser resume now shares a 64-response budget across its pre-input, post-input and EOF drains. Previously two later drains could still process the entire ready set after the bounded initial drain. Remaining non-async EOF resources retain LiveParserState and yield instead of being transferred as async work.
- Added the missing exact parser identity recheck after each EOF completion. Shared MAX_SCRIPT_COMPLETIONS_PER_SLICE also names graph and detached source slice bounds.
- Generated `ready_source_bursts_keep_unconsumed_responses_between_page_slices`: 130 ready external scripts execute in 64/64/2 batches while input remains pending, with pending counts/source ownership retained until each batch is consumed. No execution; full task-source/inline-parser fairness remains open.

## Window indexed-property boundary review

- Reviewed window_proxy.rs local indexed callbacks, v8_context_bridge false-returning write hooks and structural engine patch alongside pinned window-indexed-properties{,-strict,-delete-no-cache}.html. Indexed writes/definitions are already rejected, including absent indexes; the local deleter's absent-index success is intentional. No production change was warranted by this review.
- Extended `native_context_window_structural_exotics_match_window_proxy_contract` with 2^32−2 index rejection, 2^32−1/negative/noncanonical string-key ordinary operations, strict deletion/write TypeErrors and repeated deletion followed by child removal/reinsertion.
- Regression source only; native engine patch and callback interaction remain unexecuted. Full cross-origin descriptor/prototype/rebinding matrix and R-02 caller closure remain open.

## Window descriptor conversion and author prototypes

- Source inspection of pinned V8 150.4.0 `GetPropertyDescriptorWithInterceptor` confirmed it passes callback objects to `PropertyDescriptor::ToPropertyDescriptor`, whose inherited-field reads can invoke author Object.prototype getters or combine accessor/data fields. Existing create_data_property avoided inherited setters during construction but did not prevent these later reads.
- Added one null-prototype descriptor-record constructor shared by named accessors and data descriptors (methods, fallback names and both indexed paths). This isolates internal conversion while V8 still constructs ordinary public descriptor objects.
- Extended the structural WindowProxy regression to poison all six descriptor field names with throwing inherited getters/setters, verify cross-origin window/close/then/0 descriptors and same-origin index descriptors, assert no poison callbacks and preserve ordinary public descriptor prototypes. Poisoned properties are restored in finally.
- Source generation/review only. No builds or tests ran; R-02.a remains open for its full operation/prototype/rebinding matrix and removal audit.

## Window enumeration and named-child reconciliation

- Replaced ordinary indexed assignment into native named/indexed enumerator arrays with V8 Array::new_with_elements. The old set_index path could invoke author Array.prototype accessors while collecting native keys.
- Generated a structural regression with throwing Array.prototype accessors at indexes 0 and 14. It checks Reflect.ownKeys index/string/symbol ordering, getOwnPropertyNames, getOwnPropertySymbols, Object.keys and for-in without invoking those accessors. Source inspection confirmed the pinned V8 array-construction API; no execution occurred.
- Pinned `html/browsers/origin/cross-origin-objects/cross-origin-objects.html` also explicitly requires named child-frame reads/descriptors/hasOwnProperty (nonenumerable, nonwritable, configurable). Current WindowProxy named callbacks permit only the fixed safelist/fallback keys, while child target-name resolution exists only in window_named.rs's same-origin resolver. This is a concrete remaining R-02 implementation obligation; generic descriptor/enumerator fixes do not close it.

## Cross-origin named child-frame generation

- Resolved the preceding missing callback path by extracting native target-name lookup from window_named.rs into named_child, shared by same-origin named access and cross-origin WindowProxy reads/descriptors/queries. Lookup follows live document traversal and native frame target-name state; it does not read author Window properties or element IDs.
- Fixed safelist names retain precedence. Named children precede the fallback for `then`; their descriptors are configurable, nonwritable and nonenumerable. Enumerators intentionally retain only indices/fixed keys/fallback symbols, preserving hidden named-child behavior. Existing cross-origin write/define/delete denial remains intact.
- Generated `cross_origin_named_children_are_live_and_hidden_from_keys`: two children with duplicate names, first-child precedence, descriptor/hasOwn/in checks, omission from keys, rejected DOM-ID/author-property access, live renaming, safelist/fallback collisions and removal. The fixture changes the parent-frame V8 security token after capturing references so the actual native cross-origin handlers are exercised when tests are eventually run.
- No execution. Full R-02 operation/prototype/rebinding reconciliation remains open, including navigation and caller lifetime matrices.

## Native cross-origin Location property surface

- Confirmed Location's original template installed only an ordinary no-op named getter. Its URL accessors checked origin when invoked, but property reads/descriptors/keys themselves lacked the pinned cross-origin projection.
- Added location_cross_origin.rs and integrated template configuration with native security-token access checks. It projects cached replace and href-setter functions in the calling realm, preserves original target context via traced callback data, exposes only href/replace/then and the three fallback symbols, and rejects other named/indexed access and cross-origin mutation/definition/deletion. The href read is denied while its descriptor exposes only a setter.
- Reused private per-caller/per-target WeakMap caches with distinct Location keys, null-prototype descriptor records and direct array element construction. URL conversion/navigation routes through existing native Location operations; required-argument validation and nonconstructible function names/lengths are generated.
- Generated `cross_origin_location_projects_only_native_allowed_properties` covering saved-object identity, cached functions/descriptors, property/index/symbol rejection, descriptors/keys, mutation denial, URL syntax/coercion exceptions and inherited descriptor pollution.
- Source only, not compiled or executed. Remaining Location obligations include structural get/set-prototype and extensibility behavior (the existing engine opt-in explicitly handles only WindowProxy), cross-realm receiver validation, navigation permission/retirement reconciliation and exhaustive caller-cache lifetime coverage. These are required follow-through, not exclusions from R-02.

## Location structural engine integration

- Extended the existing template opt-in's engine predicate from global proxies to explicitly marked API instances. Location alone receives that opt-in through a dedicated native bridge; ordinary objects and objects merely inheriting Location.prototype do not. The existing public API name and bit encoding are preserved, with its contract documenting both Window and Location.
- Location now participates in the same engine rejection of preventExtensions/seal/freeze and cross-origin immutable-prototype rule (null accepted, other values rejected). Its existing immutable-prototype template preserves same-origin prototype identity. Source inspection confirms V8's ordinary access-check paths already report cross-origin extensibility and hide inaccessible prototypes.
- Added structural cases to `cross_origin_location_projects_only_native_allowed_properties` for same-origin identity preservation, ordinary-object controls, cross-origin null prototypes, Reflect false returns and Object TypeErrors.
- Updated the patch source, public-header comment/hash, patch manifest and dependent patch-chain pin using source bytes. The patch status is now explicitly generation_pending_build_and_qualification; historical run reports were preserved. No engine patch application, build, tests, formatting, signing or deployment occurred.
- Still open: receiver validation, navigation permission/retirement and complete descriptor-cache lifetime reconciliation. New structural sources remain unexecuted.

## Cross-origin setter receiver routing

- Pinned `window-location-and-location-href-cross-realm-set.html` requires invalid Reflect.set/inherited receivers to throw TypeError. V8 150.4.0's SetPropertyWithInterceptorInternal exposes only Holder to property callbacks, losing the ECMAScript Receiver.
- Extended the explicitly marked Window/Location failed-access setter path to obtain the native descriptor and invoke its setter through Object::SetPropertyWithDefinedSetter with the actual Receiver. Properties without setters retain native denial. This avoids guessing receiver identity from the holder or rejecting all borrowed receivers.
- Cross-origin Location functions now resolve/brand their actual native Location receiver; Window.location's native navigation callback validates its actual Window receiver. Original contexts remain traced for cached function lifetime, but do not override the receiver's ownership.
- Generated invalid plain/prototype/proxy/null Location receiver cases, Reflect.set and inherited assignments, Window setter equivalents, rejection before URL coercion, and valid borrowed href/replace/Window setters targeting the caller's real Location/Window.
- Updated patch hunk records and source checksums, preserving the pending build/qualification status. No engine execution, builds or tests. Navigation permission/retirement and the complete cache/function receiver matrix remain open.

## Location native navigation admission

- Traced Location through its pending request and frame consumers: direct URL operations previously bypassed the existing parent CheckNativeTargetNavigation sandbox/ancestry gate used by targeted navigation.
- Added a native LocationNavigationBackend and installed it in both new-document and initial-document reuse paths. Production calls carry the incumbent source and receiver target's native Creator identities to the authenticated page service; denial or service failure becomes SecurityError before pending navigation or speculative URL publication. The existing parent check validates current documents, sandbox, ancestry, browsing-group relationship and activation.
- URL conversion can run author code, so admission rechecks target retirement and caller retirement afterward. Retired targets publish nothing. Cross-origin javascript: URLs are silently discarded without script execution, preserving the pinned allowed href/replace calls rather than throwing on the scheme alone.
- Generated `location_navigation_checks_native_authority_before_publication` for denied href/assign/replace/reload/Window.location, service failure, unchanged URL/no queued request, admitted replacement and retired-target suppression; cross-origin surface coverage now includes nonthrowing javascript: discard calls.
- No tests/builds run. Remaining scope includes source-attributed fetch/referrer custody through accepted navigation, full same-page/cross-page navigation matrix and cache lifetime reconciliation. Admission wiring does not establish those downstream algorithms.

## Cross-origin top-navigation activation

- Pinned top-navigation tests require denial for cross-origin descendants without source activation, including parent-only activation and nested same-origin immediate-parent cases. The parent navigation gate previously returned true for all unsandboxed ancestors, bypassing this rule.
- Added native source/target security-authority comparison and source sticky-activation checks for unsandboxed cross-origin top navigation. Same-origin access and explicit sandbox allow-top-navigation retain their own rules; allow-top-navigation-by-user-activation still requires transient activation.
- Removed the duplicate same-page top-target permission branch: it now resolves the current root Creator and delegates to the same native target-navigation gate, preserving exact lifetime/ancestry checks across both callers.
- Generated `cross_origin_top_navigation_requires_source_sticky_activation` covering root/same-origin/explicit-sandbox allowance, denied cross-origin/nested frames, parent-only activation, source activation and sticky allowance after transient consumption. Both native entry points must agree.
- Source-only generation. No tests/builds or conformance runs; downstream source-attributed navigation and full target matrices remain open.

## Typed frame navigation consumption

- Removed the frame scheduler's eager author-visible location.href evaluation and navigation synthesis from a URL difference. It now consumes owned LocationNavigation requests; history-only changes still update the committed frame URL separately. This prevents replay of an already consumed request while its target document is not yet committed.
- Replaced the author-writable __oryn_opaque_origin read with the native realm descriptor's origin. Added a native Location URL accessor for page-location reporting and default-action URL resolution, removing those JavaScript evaluations too.
- Generated `frame_location_scheduler_consumes_native_requests_without_author_flags`: a throwing author opacity getter is ignored, native URL is readable, the original document URL supplies the existing initiator field, the scheduler restores the top realm, and a second drain cannot synthesize a duplicate request.
- No execution. This closes these author-read/replay paths, not the broader caller-origin/referrer attribution gap: the current downstream initiator still derives from the target frame. About/data/blob frame Location routing and complete accepted-source custody remain required follow-through.

## Signed script-navigation source foundation

- Added ScriptNavigationAuthority, separate from form authority, and bounded parent-issued snapshots binding context, source/target Creator identities, destination page and exact initial URL. Source metadata reuses native document/history URL validation, referrer policy and cookie ancestry capture.
- HMAC input has a distinct script-navigation domain. Form and script tokens cannot be substituted, so a script navigation's exemption from form-action cannot authorize a form submission. Readback validates the admitted target's current document while allowing the already accepted source to retire.
- Added authenticated context-service issue/read routes. Generated `script_source_authority_binds_target_url_and_preserves_retired_source` for activation admission, immutable URL/referrer/ancestry, wrong target/URL/context, corruption, cross-kind token substitution, source retirement and target retirement.
- Source generation only. Backend issuance and propagation through LocationNavigation/root/frame fetch consumers are still pending; the new foundation is not yet a closed production attribution path. No tests/builds or protocol activation occurred; these fields remain part of the pending context45 batch.

## Signed script source propagation through navigation consumers

- LocationNavigationBackend now obtains the signed source authority during admission and records it with the exact target Creator in LocationNavigation. The request includes the incumbent native document URL/referrer policy; unit backend regression sources now assert those arguments and retained authority.
- Root navigate_script passes the admission to navigate_resource, which validates the target/URL through the parent before using original referrer, initiator, top-level URL and ancestor chain. Signed referrer policy takes precedence over separately supplied fields; later redirect response policies still apply normally.
- FrameNavigationContext retains admission through owned frame resource preparation. LoadPageResource validates it before local/network handling, selects signed source metadata for referrer/cookie context and preserves it across redirects. Existing embedding CSP, service interception and form-action checks remain independent.
- Updated all inspected request/options/Location constructors and added `script_frame_fetch_uses_sealed_source_after_initiator_retires` with a capturing redirect broker: wrong URL fails before IO, forged caller fields cannot replace sealed metadata, source retirement is tolerated, and redirect policy/taint survives all hops. No network or test execution occurred.
- Remaining generation review includes full root end-to-end regression coverage, no-network scheme commit attribution, accepted-target supersession during IO and source-attributed navigation outside direct Location operations. This source path is integrated, but not compiled or qualified; context45 remains pending.


## Navigation target retirement during IO and planned-submission stop

- Frame script-navigation loads now validate both the original embedding document and the signed target Creator before each network hop and after each response. A retired target cannot publish a terminal body or follow a redirect. Generated `script_frame_fetch_rejects_target_retirement_during_io` covers retirement inside the broker for both 200 and 302 responses, asserting exactly one request. This addresses retirement, not same-document navigation supersession.
- Native `stop_pending_document_load` now removes queued form Submit actions and a pending Location navigation before cancelling resource work. Queued Click dispatches remain and subsequent submissions are accepted by the live document. Generated `stop_cancels_planned_submissions_but_allows_later_submission` covers cancellation, retained click dispatch and subsequent submission.
- Source inspection confirmed NetworkResponse uses u16 status, matching the new retirement fixture. No build, test or conformance command was run. Pageswap activation/ordering and full navigation cancellation integration remain open.


## Native PageSwapEvent / PageRevealEvent generation

- Added pinned Web IDL event interfaces/dictionaries and Window/body/frameset handler routing. Constructors convert PageSwapEventInit in flattened dictionary order (activation, bubbles, cancelable, composed, viewTransition), preserve getter exceptions, and retain payloads in private internal fields. Native payload arrays are populated without invoking author Array index setters.
- Added native event attribute receiver checks and trusted dispatch through the Window listener registry, independent of public constructors/dispatchEvent. Host and executable expose swap/reveal entry points; swaps accept explicit activation-disclosure eligibility for native redirect-chain checks.
- Generated `page_swap_and_reveal_events_have_native_payloads_and_dictionary_conversion` covering defaults, flags, conversion order/errors, invalid receivers/proxies, Window/body aliases and trusted dispatch with replaced public constructors. No tests executed.
- NavigationActivation creation/validation is integrated against the concurrently generated navigation_api module signatures. Automatic runtime commit/BFCache/frame event ordering, full redirect-origin disclosure, stable native history synchronization, rendering-time reveal and real ViewTransition creation remain open. Trusted event payloads currently carry null viewTransition; no transition implementation or full lifecycle completion is claimed.


## Root navigation swap and cached reveal integration

- `load_document` now delegates to `load_document_with_redirect_history`; root document commits dispatch pageswap before suspension/pagehide. Reloads emit pageswap and non-persisted pagehide and stop old document loading before replacement. Same-document fragment updates retain their separate path and do not emit these cross-document events.
- The network redirect loop tracks exact URL origins separately from cookie schemeful sites and retains any cross-origin hop. Streamed commit passes this flag into swap disclosure, preventing a final same-origin URL from erasing an intervening cross-origin redirect. Disclosure additionally checks the departing realm's native non-opaque origin.
- Cached history traversal emits pageswap before incumbent suspension; successful restoration emits pagereveal before pageshow. Generated `navigation_commit_swaps_before_hide_and_cached_restore_reveals_before_show` covers trusted event ordering and null activation after a cross-origin redirect with a same-origin final destination.
- Source inspection caught and corrected the executable realm selection method (`activate_realm`) and the native global dispatcher return type. Location navigation is now sibling-visible so the concurrent Navigation API reuses signed native admission.
- Still open: stable native session-history identity/state synchronization before script exposure, initial rendering-time reveal, descendant transition ordering, reentrant supersession/rollback sequences, real ViewTransition creation and full network-driven regression matrix. No compilation or test execution occurred.


## Native history snapshot initialization before script execution

- Native history entries now retain typed NavigationHistorySnapshot identities and separate Navigation API / History structured state. UUID key/ID allocation is fallible and staged before incumbent suspension. Replacement retains key and changes ID/document identity; traversal carries an explicit native target index instead of inferring the destination from the current index.
- Root constructor resources carry a reserved typed initial history update. RustExecutable consumes it after DOM installation and before readiness/parser script work, using RawV8Host::sync_navigation_history. Visible history is a contiguous same-origin segment; opaque URL origins expose only the selected entry. Native commit installs the staged history after construction.
- Generated `native_history_identity_is_installed_before_first_script_and_replace_keeps_key` checks that the first inline script observes native URL/key/ID and replacement retains its key with a new entry/document identity. This is source-only regression coverage.
- Integration remains open for binding-to-native structured same-document updates, BFCache snapshot/activation refresh, frame history, native pending Navigation API state transfer, snapshot visibility using committed opaque/inherited origin policy, and reentrant changes during outgoing lifecycle callbacks. No builds/tests/generators ran.


## Structured history updates and cached snapshot refresh

- Host/executable now expose owned NavigationHistoryUpdate draining. Native session history applies updates to the current visible segment, retains hidden surrounding entries, preserves POST/form restoration payloads, and remaps cached entries by immutable entry ID. Wrong-document, duplicate-key/ID and cross-origin snapshot data is rejected; source diagnostics record rejected internal updates.
- Binding updates carry an explicit coalesced forward-truncation flag (parallel Navigation API integration), so push followed by replace in one task cannot preserve a hidden cross-origin forward suffix. Constructor-time script updates are applied after the staged initial native history commits.
- Successful BFCache restoration refreshes the realm's native history snapshot before pagereveal/pageshow, preserving cached wrapper identity for matching IDs. Ordinary cross-document Location replacement clears both state channels; reload/traversal retain their prior state pending API-specific overrides.
- Added `same_document_history_updates_preserve_state_and_remove_hidden_forward_entries`: a cached document pushes then replaces cyclic History.state, drops a cross-origin forward entry/cache, and preserves its native key and distinct Navigation API state. Regression source only.
- Remaining integration: native fragment-navigation identity, accepted pending Navigation API destination/state propagation, Navigation.activation synchronization, frame history, removal of remaining legacy URL-only callers, reentrant outgoing lifecycle and rollback custody. No tests/builds ran.


## Native typed history events and fragment entry identity

- Added PopStateEvent/HashChangeEvent Web IDL, private typed payloads, lexical dictionary conversion, native receiver checks and Window/body/frameset handler routing. Replaced dispatch_global_history_event's public dispatchEvent call with trusted native listener delivery; history traversal now shares the private typed event implementation.
- Added native host/executable dispatch_history_transition. Root fragment navigation no longer evaluates author URL/Event/dispatchEvent globals. It synchronizes native entry snapshots, emits popstate before hashchange for traversal, and rechecks document identity between callbacks.
- Fragment push allocates distinct entry key/ID while retaining the document identity; replacement retains key with a fresh ID, and traversal selects the explicit target index. Generated `native_fragment_history_uses_typed_trusted_events_and_stable_entries` poisons public constructors/dispatcher, checks typed trusted payloads/order and verifies entry identity after traversal without document replacement.
- Pending Navigation API destination/state propagation, activation, task-queue/reentrancy edge cases, full fragment state semantics and frame history remain in generation. No build/test/conformance execution.


## Accepted Navigation API destination state and activation custody

- LocationNavigation now carries an optional exact NavigationHistorySnapshot; ordinary Location/form/default-action constructors initialize None. Parallel Navigation API admission attaches its accepted snapshot. Native navigate_script rejects an initial URL mismatch, scopes that snapshot through the loader and restores prior scope on return (including failure).
- Native history preparation preserves accepted key/ID and both distinct state channels while using the final redirected URL and native new-document identity. This avoids discovering pending state by URL, which could incorrectly attach stale API state to an ordinary same-URL request.
- Root constructors receive native Navigation.activation before parser scripts via reserved typed resources. Cached restoration refreshes activation after history synchronization and before reveal/show; cross-origin/unavailable prior entries yield no disclosed from entry.
- Generated `accepted_navigation_destination_preserves_identity_state_and_clears_loader_scope` covers accepted state/identity, activation/currentEntry identity, scope cleanup and mismatched-request rejection. No tests executed.
- Remaining: exact staged pageswap destination identity (parallel binding hook), frame accepted-state/history integration, pending source promise completion/abort, opaque/inherited origin disclosure edge cases and redirect/reentrancy qualification. Full implementation remains unverified; no build/test/generator run.


## Staged swap identity and same-document path traversal

- Root commit and BFCache traversal now stage the actual native destination snapshot in the outgoing realm before disclosed pageswap activation creation. Staging is separate from a pending Navigation API promise and is not performed when activation must be hidden. Generated `pageswap_activation_uses_the_committed_native_destination_identity` checks destination ID/key/URL and exact from/currentEntry identity.
- Same-document traversal now uses native document identity, including path-changing pushState entries and destinations without fragments. It preserves the live document and avoids network/form-resubmission work; ordinary navigation to an unchanged URL without a fragment still loads a document.
- Generated `same_document_path_traversal_without_fragment_keeps_live_document` covers original state restoration, retained DOM wrapper identity, unchanged document generation and absence of pageswap for a path-changing history traversal.
- Remaining task-source ordering, reentrancy, cache rollback, source promise settlement, descendant/frame history and rendering-time reveal work remains open. Source edits only; no builds/tests executed.


## Native animation frames and corrected reveal ordering

- Removed the author-visible __oryn_animation_frames/__oryn_run_animation_frames shim. Generated Window requestAnimationFrame/cancelAnimationFrame now use document-owned native callback lists; callback snapshots honor cancellation during delivery, defer newly registered callbacks, use one native timestamp per document rendering step, run microtask checkpoints and release callbacks on realm retirement. Host rendering visits a snapshot of live frame/document identities.
- First rendering delivers pagereveal once per activation before animation callbacks. BFCache resume rearms reveal for retained documents. Native document load now emits initial non-persisted pageshow once per Document after load and exact lifetime checks.
- **Correction to earlier cached lifecycle notes:** pinned `pagereveal/order-in-bfcache-restore.html` and `order-in-new-document-navigation-iframe.html` require pageshow → pagereveal → rAF. Removed eager reveal-before-pageshow from restoration; reveal runs in the next native rendering opportunity. Updated the existing cached-order regression source accordingly.
- Added `native_animation_frames_preserve_cancellation_microtasks_and_reveal_order` for cancellation inside a callback, microtask ordering, callbacks deferred to next frame, shared timestamp, one reveal and independence from poisoned old scheduler globals. Native PopStateEvent now uses the cached History.state object for identity preservation.
- Rendering eligibility/hidden documents, shared-agent timestamp alignment, cross-frame callback snapshot ordering, initial-document Navigation suppression, task-source fairness and complete lifecycle reentrancy still need source reconciliation. No build, test or generator was run.


## Rendering callback ownership and shared phase time

- Native rendering captures one monotonic Instant before visiting documents. Every document derives its callback timestamp from that same rendering time and its own performance origin, so work in an earlier frame cannot advance a later frame's timestamp for the same opportunity.
- Added `native_animation_callback_errors_use_callback_realm_and_do_not_stop_other_callbacks`, matching pinned callback-cross-realm-report-exception.html: request owner, callback realm and thrown Error realm differ; only the callback's realm reports the error and other callbacks continue. Callback exception reporting reuses the existing native creation-context reporter.
- Initial document replacement resets reveal/initial-pageshow flags; existing retirement clears the native callback list. Committed root constructors explicitly clear Navigation initial-document suppression. Reconciled duplicate host setter introduced in parallel, retaining the shared implementation.
- Frame archived content/BFCache custody and traversal admission continue in coordinated S/V and P/W lanes. Hidden-document rendering eligibility, task-source fairness and complete cross-frame lifecycle matrix remain open. No tests/builds ran.


## Removed superseded URL-only history transport

- After S/V converted the final frame consumer to structured history updates, removed pending_history_url_update and take_history_url_update from binding state, History/Navigation producers, host/executable forwarding and native root synchronization. Source search confirms neither symbol remains in the JS/native source trees.
- Root synchronization no longer applies a second URL-only value after rejecting a structured update. Added native navigation-kind validation alongside current-document identity, unique key/ID and origin checks.
- Generated `rejected_structured_history_update_preserves_native_url_and_entries` for retired-document identity, cross-origin URL, duplicate identity and unknown-kind rejection without changing native URL/index/entry identity. Existing structured push/replace/cyclic-state regressions cover successful updates.
- Parallel frame BFCache work now has parent-subtree capability source; host/V8 reattachment remains under evidence-based implementation. No source search is treated as runtime verification, and no tests/builds executed.

## Whole-page retained descendant transition delivery

- Whole-page pagehide/pageshow now enter an explicit top-realm native tree dispatcher. Native FrameLifetimes supplies initial blank children as well as committed descendants, independently of the currently selected automation realm.
- Recipients are snapshotted in native tree order before callbacks; exact root, generation, stream epoch and every ancestor/frame/current-child edge are revalidated before each delivery. Removed subtrees and newly inserted documents cannot inherit a prior snapshot's event.
- Kept the single-document transition entry point for direct frame navigation. The new dispatcher selects native contexts without changing the host's active realm.
- Generated regression sources `page_transition_tree_includes_initial_children_and_preserves_active_realm` and `page_transition_tree_skips_removed_subtrees_and_new_documents`. Cross-document ordering and the complete reentrant restoration matrix remain subject to source reconciliation and later qualification; these sources have not been executed.
- Independent iframe BFCache reattachment is not established as a required implementation mechanism by the inspected plan/tests. Its optional engine patch remains unintegrated; required source restoration and whole-page child retention continue. No build/test/generator ran.

## Browser navigation admission and exact cancellation integration

- Browser-originated push/replace/reload enters native NavigateEvent admission before resource loading. Handled cancellation/interception synchronizes native document/history state and skips the fetch; accepted destination identities are scoped through the existing loader and restored afterward.
- Executable admission checkpoints callbacks and revalidates the exact native operation before allowing a fetch. A microtask that stops or supersedes navigation invalidates the original operation.
- Already-admitted script requests capture their accepted destination's operation instead of dispatching a duplicate event. Explicit beforeunload cancellation aborts only that still-live operation. No success-settlement hook was added: pinned cross-document, reload, 204/205 and download semantics keep source promises unresolved.
- Generated runtime regressions for cancellation/interception without fetching and microtask stop before fetch. Traversal admission integration, cross-realm source metadata and the remaining cancellation/error matrix are still open. These are unexecuted sources; no build/test/generator ran.

## Traversal and refresh admission integration

- Native Back/Forward now admits the actual history destination before entering either cached restoration or resource loading. Same-document traversal/cancellation/interception returns through structured native synchronization. Cross-document admission scopes exact destination identity and cancellation token through restoration/loading.
- Coordinated the snapshot-based API with S/V and P/W: cross-origin entries hidden from Navigation.entries still traverse via the native history snapshot without exposing their private entry state to the source. Checked traversal index addition avoids overflow.
- Refresh now shares browser admission while preserving POST resubmission body/authority; native reload cancellation cannot accidentally start its resource request.
- Generated `cancelled_browser_traversal_keeps_cached_destination_parked`, `cancelled_browser_refresh_preserves_document_and_history_identity`, and `browser_back_restores_cross_origin_entry_hidden_from_navigation_entries` regression sources. Complete reentrant beforeunload/pageswap handling and frame full-history custody remain open. No build/test/generator ran.

## Exact operation checks across outgoing lifecycle callbacks

- Scoped the accepted native operation alongside its destination through browser, script and traversal loads; both fields restore on success/error. Revalidate it after beforeunload/pageswap/pagehide and before document commit/suspension. A replacement navigation cancels the superseded load without aborting the newer operation.
- Generated `lifecycle_navigation_replacement_prevents_superseded_commit` for replacement during all three callbacks, retained incumbent generation/history and preserved newer pending request. This is unexecuted regression source.
- Operations without an exposed Navigation entry/token (including hidden cross-origin traversal) still need complete epoch-based supersession coverage. HTTP 204/205 and download outcome integration is the next known runtime gap; full source generation and all qualification gates remain open. No build/test/generator ran.

## HTTP no-content navigation outcome

- HTTP 204/205 now cancel/release the response operation before body demand and return the retained document's URL/generation/history. Response CSP, isolation and referrer policy are not installed on the incumbent Document; no pageswap/pagehide or parser commit occurs.
- Added a no-commit completion path for the browser navigation command, without settling source Navigation API promises. Generated `no_content_navigation_preserves_document_policy_and_unsettled_promises` covering both status codes, poisoned response policy, retained Document identity and absent success/error/commit/finish callbacks.
- Download response/body quarantine integration is still pending the authenticated parent route now being generated by P/W. Full source generation remains open; no build/test/generator ran.

## Root full session-history initialization and restoration

- Added a separate native full-session snapshot resource, consumed and removed before first author script. Navigation entry objects still receive only the visible segment; native HistoryRecord receives the full list and absolute cursor.
- Cached restoration and the runtime fragment/traversal commit path synchronize both views before lifecycle callbacks. The full snapshot rejects missing identities and invalid/beyond-bound cursors instead of silently shifting positions by filtering entries.
- Generated `full_session_history_is_available_before_script_without_exposing_foreign_entries` covering initial history.length across origins, same-document pushes, retained full length after cached Back, hidden foreign Navigation entries and current History.state.
- Source generation only. Root inherited-origin bookkeeping and full callback reentrancy still require reconciliation; no build/test/generator ran.

## Navigation download quarantine consumer

- Scoped explicit anchor download intent through the root script loader; explicit download requests cannot take the fragment-only path. The HTTP outcome consults the shared disposition parser and honors the attribute only for a native same-origin destination.
- Attachment/download responses collect body chunks under a 16 MiB transport bound, exact request ID, navigation deadline and cancellation checks. Capture/revalidate the source native Creator and operation, then send the authenticated parent quarantine request. Errors cancel the response operation; document response security and parser installation remain untouched.
- Successful quarantine completes the browser command without a document commit or source Navigation API promise settlement. Unsupported parent-service configurations fail without rendering attachment HTML; generated a regression source for this case.
- Shared disposition parser and frame no-content/download consumers are being generated in parallel. Data/blob download outcomes, transport-level successful runtime regressions and complete sandbox/origin/failure matrix still need source reconciliation. No build/test/generator ran.

## Data downloads and successful quarantine regression sources

- Extracted common byte-quarantine dispatch with source Creator/operation validation and the 16 MiB transport bound. Data URL anchor downloads use native data decoding and this same parent path without changing origin, installing response policy or creating a document.
- Integrated P/W's shared disposition parser. Generated successful HTTP attachment and data-anchor regression sources with explicit Download Allow fixture policy, exact binary bytes, filename handling, retained Document identity and absent outgoing page lifecycle events.
- These regressions have not run. Blob root resolution, initial-page download handling, frame consumers, admission policy interactions and the complete failure/cancellation matrix remain source-generation work. No build/test/generator ran.

## Native blob download resolution

- Root explicit blob downloads now resolve through the existing authenticated WorkerService BlobResolve request. The registry enforces caller partition/origin and retained creator authority; no author-visible blob-source shim is consulted. Lookup removes the fragment while the download descriptor retains the requested URL.
- Resolved bytes and media type enter the same bounded source-revalidated parent quarantine path as data/HTTP downloads, without creating a Document.
- Generated `blob_anchor_download_uses_registered_binary_bytes_and_retains_document` for binary payload, fragment lookup, media type, filename and retained source. Parent/frame resource loading is being wired to the same native registry by S/V.
- Ordinary blob document navigation, initial-page attachment source initialization, admission and cancellation edge cases remain under source reconciliation. No builds/tests/generators ran.

## Carried navigation admission identity

- Integrated P/W's native revision-aware operation matching into root lifecycle/download checks. A visible destination is no longer required to validate opaque-origin or hidden-history navigation.
- Script loader now consumes the token carried by LocationNavigation and validates it before loading. It never captures a fresh token when dequeuing an old request; missing admission identity is a protocol error and stale identity is cancellation without disturbing a newer queued request.
- Generated `queued_navigation_cannot_reacquire_authority_after_supersession` for tuple-origin and opaque data documents, checking retained incumbent generation and newer pending request.
- Producer/frame token integration continues in the parallel lanes. No build/test/generator ran; these are source changes, not qualification results.

## Child-history restoration input and staged payload retention

- Root same-URL traversal/reload now passes bounded saved child-frame descriptors through reserved `oryn:frame-history` before parsing; S/V owns constructor/child consumption. Redirected destinations do not receive a different resource's private saved frame payload.
- Fixed staged history losing form/frame snapshots captured during outgoing suspension: copy payloads from the native history list by immutable entry ID after suspension. Reload explicitly captures post-pagehide form/frame state into the replacement entry with a combined byte bound.
- Generated `staged_history_retains_form_state_captured_during_pagehide` using a form-associated custom element's updated restoration state.
- HTTP attachments now obtain explicit parent Download authorization before requesting body chunks, then revalidate again at quarantine publication. No build/test/generator ran.

## Shared buffered document loader for local URL bodies

- Extracted MIME decoding, encoding metadata, native parser setup and completed response capture into `load_buffered_navigation`; data navigation consumes it and the pending ordinary-blob path will share it.
- Generated a buffered data regression source for UTF-16LE BOM decoding in a text/plain document, literal markup preservation, charset/content type and committed URL.
- P/W is generating authenticated blob-document preparation with native creator-origin policy. Data/blob CSP, referrer and isolation inheritance are explicitly under source review; the shared loader does not establish those semantics by itself. Ordinary blob-document integration remains incomplete. No builds/tests/generators ran.

## Unified document MIME decoder selection

- Removed duplicated HTML/XML/plain-text decoder classification in the streaming runtime. Both streamed HTTP and buffered local bodies now use `network::navigation_text_kind`.
- Generated split-boundary decoding regression sources for UTF-16 BOM text, XML declaration encoding and HTML meta charset; each asserts independent expected text for buffered and streamed delivery with an empty intermediate chunk.
- Blob security preparation is being implemented as a staged parent grant, consumed after outgoing lifecycle/parking and before the new constructor. It must retain native creator origin and policy; integration remains pending. No builds/tests/generators ran.

## Removed loader-facing token recapture paths

- Source caller search confirmed host/executable `capture_native_navigation` and `capture_native_navigation_revision` had no remaining production callers after carried-token integration. Removed both forwarding APIs; producer-side capture remains inside the native Navigation implementation.
- Recounted detailed package checklist rows and confirmed the table matches them. Added explicit supplemental source items and clarified that cross-package additions are separate from those table counts. This inventory check is documentation/source inspection only, not generation-gate or runtime verification.

## Staged local-document grant integration

- Root ordinary blob navigation now prepares native bytes/media type/policy through the parent registry and uses the shared buffered parser. Data navigation with a live native source uses the same staged grant path instead of changing parent security before outgoing lifecycle.
- Grant consumption occurs after pagehide/parking and before generation publication and the destination constructor. Source-side CSP/isolation/referrer values change only at that boundary. The constructor obtains its exact tuple/opaque origin from native parent authority.
- Encoded/decoded body bounds are checked; preparation is discarded on precommit cancellation/errors and after successful consumption (idempotent cleanup). Initial data navigation without a source still uses the pre-existing initial path and remains under reconciliation.
- Generated blob-document regression source for native creator origin, HTML MIME, title and URL. Parent/frame origin metadata, complete policy inheritance and commit-failure rollback are still being reviewed; no build/test/generator ran.

## Local commit rollback and exact inherited CSP

- Staged local navigation retains the incumbent through parent commit, including stopped/non-reusable documents needed only for rollback; cache eviction occurs after successful commitment. Parent rejection restores native authority and the retained executable before pageshow. Reload rejection restores visibility on the still-owned incumbent.
- Extracted the existing cached-restore rollback into a shared incumbent recovery helper, removing its duplicate implementation. A failed second recovery remains an explicit error with owned cached state.
- Root now uses the exact CSP object returned by the parent commit, preserving creator-relative local-scheme `'self'` semantics instead of reconstructing policy against the blob/data URL.
- Generated `failed_local_grant_commit_restores_parked_incumbent` for rejected commitment, original Document/generation/history and restored visibility. Constructor failure after a successful parent commit and initial-document local navigation still require reconciliation. No build/test/generator ran.

## HTTP policy commit boundary and cached runtime policy

- Source review of constructor recovery exposed an earlier boundary defect: HTTP response security was installed before outgoing parking. Response headers now enter the document loader explicitly and install after outgoing lifecycle/cache capture, before destination construction.
- Cached entries retain runtime CSP, referrer/isolation policy, loaded fonts and challenge state. Both normal restoration and rollback recover these values and invalidate the layout input cache, rather than leaving the later document's values active.
- Generated `cached_restore_recovers_incumbent_policy_instead_of_destination_headers` with distinct CSP eval/referrer rules across two documents.
- HTTP parent-commit failure and constructor failure after commitment still require complete recovery reconciliation. No build/test/generator ran.

## HTTP policy rejection recovery

- HTTP response policy installation now retains the incumbent for rollback until parent acknowledgement. Failed installation uses the same native-authority/document recovery as local grant rejection, including explicit ownership on a second recovery failure.
- Runtime CSP/isolation/referrer assignments occur only after a valid parent acknowledgement. Generated a disconnected-parent regression source proving rejected response policy leaves the incumbent runtime CSP/referrer unchanged.
- Recovery logic for local and HTTP commitment shares one helper. Constructor failure after successful parent commitment remains open; no build/test/generator ran.

## Destination constructor recovery

- Cross-document construction now retains the parked incumbent until constructor and initial context polling succeed; rollback-only entries are released and normal cache bounds applied afterward. Constructor/polling failure drops the failed destination before restoring incumbent native authority and runtime state.
- Reload now parks its outgoing document after its existing non-persisted pagehide instead of destroying it before the constructor. The parking helper accepts the already-delivered transition, avoiding duplicate pagehide, and retains stopped documents solely for rollback.
- Generated `constructor_failure_recovers_push_and_reload_without_duplicate_pagehide` using invalid native restoration metadata to force constructor rejection; it checks original Document/generation/history and exact hide/show delivery.
- Failures after successful executable installation (including later layout/readiness work), initial-document initialization and full parent/worker failure matrices remain under reconciliation. No build/test/generator ran.

## Initial native authority and publication boundary

- Initial data/blob callers now prepare through parent-owned initial identity; the immediate PrepareDataNavigation path is removed. Root constructor no longer overwrites parent-provided initial-document metadata.
- Root wraps initial HTTP/local commitment in parent ParkInitialDocument custody. Pre-installation failures restore native initial authority and runtime fields; installed destinations discard the old snapshot even if later readiness fails. The monotonic generation allocation cursor is not rewound.
- Prepared resource/history serialization errors now enter incumbent recovery. Native document and session history publish before layout; controlled Committed is emitted after installation, avoiding a committed milestone for a rejected constructor.
- Added `initial_constructor_failure_restores_native_blank_authority_and_allows_retry` regression source for rejected first constructor and subsequent data navigation. Parent/worker failure and post-installation readiness matrices remain under source review. No builds or tests run.

## Native root history origin custody

- Root history entries now retain native origin identity separately from their URL. Before scripts, the parent document authority supplies the committed destination identity; cached/same-document updates retain it. Inherited about:blank and blob documents therefore participate in the correct contiguous Navigation history segment without reparsing their URLs as authority.
- Snapshot admission preserves URL rewrite validation, permits inherited same-resource fragment changes, rejects unknown document groups, and bounds the complete merged history including hidden entries.
- Generated `native_history_visibility_keeps_inherited_blank_and_separates_opaque_documents`; existing foreign-URL snapshot rejection sources remain applicable. No build/test execution.

## Native rendering recipient ownership

- Animation delivery now visits the native descendant document snapshot in DOM order, including initial frame documents. It no longer uses the executable automation inventory or mutates the active realm to select each callback context.
- Lifecycle and rendering share the same traversal with root/document/ancestor-edge revalidation after author callbacks. Newly inserted descendants wait for the next snapshot and removed subtrees cannot receive the old rendering delivery.
- Generated `animation_tree_uses_native_order_and_excludes_reentrant_new_children`, covering initial children, reordered embeddings, active realm preservation, delayed new children and removal. Visibility eligibility and shared-agent rendering timestamps remain separate open requirements. No tests/builds executed.

## Snapshot asynchronous scripts and graph fallback removal

- Context-backed snapshot async modules and external classics now enter the retained parser async scheduler, with shared page source/graph budgets and native script leases. They delay load while allowing DOMContentLoaded after immediate/deferred script work.
- Removed production preload_module_graph. Module execution with a parent context requires completed asynchronous graph preparation; ModuleGraphLoad's blocking wait helper is now test-only.
- Generated `snapshot_async_module_retains_graph_without_delaying_dom_content_loaded`. Existing live-parser asynchronous/deferred and frame preparation paths already retain graph work. Direct compatibility constructor loops and non-context standalone fetch/module callbacks remain explicit R-06.j generation work. No build/test execution.

## Public load_html commit caller reconciliation

- Public LoadHtml actor dispatch now passes response policy into the shared constructor commit boundary. Removed duplicate early parent security installation and early runtime CSP/referrer/isolation replacement on the native context-backed path.
- Outgoing pagehide therefore retains native source authority; cache capture precedes destination policy publication. Initial loads use the same initial-document park/recovery path. Challenge assessment is published only after successful construction.
- Generated `load_html_preserves_outgoing_authority_until_pagehide_and_cached_restore`, covering a native storage mutation during pagehide and cached original Document/origin restoration. Source only; no tests/builds executed.

## Storage navigation caller cleanup

- Removed runtime storage snapshot forwarding and the unused executable storage_snapshot method, which evaluated author-visible localStorage.items/sessionStorage.items. Document navigation uses parent-owned origin/session partitions; cached documents retain their native authority.
- Generated navigation_storage_uses_native_partitions_without_author_snapshots for independent local/session values across two origins, fresh same-origin construction, cached return, and an author-poisoned items property. Native storage bootstrap/event ownership remains under the P/W source review. No tests/builds executed.

## Retired queued navigation boundary

- Queued script navigation now rejects a missing source executable instead of treating it like a first browser navigation. The shared resource loader also validates any carried admission token before policy checks, same-document mutation or fetch setup.
- Extended stale-queue regression source with an admitted request whose source executable retires before dequeue; native document generation/URL remain unchanged.
- Source audit identified additional about:blank staged-origin and failed Navigation promise settlement integration; parent/native support is assigned to P/W and root owns subsequent caller wiring. No tests/builds run.

- Public GotoResource actor dispatch now enters browser native admission with its original POST/referrer/form authority, instead of directly entering the resource loader. Generated public_resource_navigation_obeys_native_cancellation_before_fetch to cover cancellation with no network broker. Same-document/history and already-admitted script paths retain their separate existing admission custody.

## Native precommit navigation failure delivery

- Browser, queued-script and history traversal callers now settle failed native operations through the shared typed fail_native_navigation API. Delivery requires the source generation to remain active and the native operation token to match; replacement documents and superseding operations cannot inherit failures.
- Network/download, policy, quota and cancellation failures map to native DOMException categories; secondary failure-delivery errors preserve the original error text. Success, 204/205 and download completion do not enter this path.
- Generated failed_navigation_rejects_matching_promises_and_retains_incumbent for shared error identity across navigateerror/committed/finished, aborted signal and original Document retention. Existing no-content unresolved-promise regression remains unchanged. Source only; no tests/builds run.

## Staged root about:blank commitment

- Context-backed about:blank navigation now uses PrepareBlankNavigation and the existing one-use local grant commit/discard path. Native creator and base URL are captured before outgoing lifecycle work; initial blank uses the parent-owned initial identity.
- The parent-returned inherited base travels through reserved constructor metadata and host.set_inherited_base_url before scripts. Native policy/origin commitment precedes history visibility initialization.
- Generated blank_navigation_inherits_native_origin_and_document_base covering native origin, cross-origin base element inheritance, same-origin Navigation entry visibility and a later fragment navigation. No tests/builds run.

## Compatibility constructor progress and deadline

- The remaining synchronous compatibility wrapper now services native timers/context completions as well as parser slices, with one construction deadline captured before initialization. Stalled input cannot renew the deadline on each poll; timeout stops owned document work and releases input.
- Generated compatibility_constructor_deadline_releases_pending_input using an immediately expired constructor budget and permanently pending input. This does not close R-06.j: production actor callers still need deferred navigation replies before all constructors can yield unconditionally. No tests/builds run.

## Deferred browser navigation replies

- Page actor Goto/GotoResource/Back/Forward/Refresh now dispatch streamed construction in scheduled mode and retain the response until the new document reaches load completion. Ready results refresh title/revision/lifecycle from the live document.
- Reply custody records the destination generation and a fixed navigation deadline. Supersession/close/error replies are terminal; readiness timeout stops owned document work. Actor command and context-event processing continue while parsing is pending.
- Generated deferred_navigation_reply_waits_for_parser_and_rejects_supersession. LoadHtml, script-triggered loads and snapshot constructor caller migration remain open; this does not yet close R-06.j. No tests/builds run.

## HTML and script actor construction scheduling

- Added typed HTML/navigation reply custody so public LoadHtml completes with the final native PageInfo after parser/load readiness. HTML strings normalize to owned DocumentStream when scheduling is requested.
- Page actors now keep scheduled construction enabled, including script-triggered navigation from page tasks. Direct compatibility users retain their explicit synchronous wrapper; native actor source/string loads no longer enter it.
- Generated html_reply_observes_final_document_after_scheduled_string_construction. Parsed-snapshot scheduling and standalone dependency callbacks remain under R-06.j reconciliation; no builds/tests run.

## Scheduled snapshots and exact deferred reply lifetime

- Root parsed-snapshot constructor branches now select scheduled construction when invoked by the actor/controlled navigation. Scheduled snapshots retain classic/deferred scripts in the owned completion queue rather than executing them in construction; synchronous compatibility constructors remain explicit.
- Pending replies now retain native realm/generation/stream identity. Replacement by document.open cancels the old reply even if root generation is unchanged, and an expired old deadline cannot stop the replacement stream.
- Generated scheduled_snapshot_defers_classic_and_module_execution_to_page_tasks and expanded deferred-navigation regression with an expired reply after document.open. No tests/builds run; standalone network/module callback reconciliation remains open.

## Parent-backed script source fallback removal

- Removed run_script_record's synchronous parent LoadScript fallback. Context-backed external execution now requires the scheduler's retained source result; inline authorization remains a native policy request.
- Snapshot compatibility construction moves external classics and following classics into the ordered completion queue, preserving order while using asynchronous source preparation. Scheduled snapshots already queue all scripts.
- Removed the duplicate synchronous context-backed external branch in load_dynamic_script_source; its production caller schedules all nonempty external URLs through retained RunScriptLoad jobs. Added a negative regression to the existing native module-source fixture for bypass rejection. Standalone broker-only paths remain under review. No tests/builds run.

## Incumbent events during response-header wait

- Header waits now pump native incumbent timers/context events and immediately revalidate the carried navigation operation. Superseding callbacks cancel the old transport rather than waiting for its headers/deadline.
- Generated incumbent_timer_supersedes_navigation_waiting_for_response_headers with a transport that withholds headers and a timer that queues replacement navigation.
- Header wait still occupies the command handler; full actor-owned pending-head state remains required for unrelated command concurrency. No tests/builds run.

## Native animation visibility eligibility

- Rendering callback delivery now checks parent-owned WindowState visibility; hidden or unpublished/parked documents retain callbacks without executing them. Author-shadowed document.hidden cannot change eligibility.
- Extended the native permission/visibility regression source with hidden callback retention, poisoned author visibility and later visible resumption. Shared-agent rendering timestamps and full lifecycle matrix remain under review. No tests/builds run.

## Navigation transport custody reconciliation

Source inspection found that `network_identity::IdentityBroker` already retains a native lease through receiver closure and acknowledged cancellation retry. A second broker-calling destructor in `NavigationBodyChunks` would duplicate custody. Instead, terminal input now closes the response receiver and releases demand immediately, even when the parser retains the adapter. Timeout/malformed input and retained-adapter cancellation-retry regression sources were added. The identity-mismatch diagnostic send now observes cancellation while its consumer queue is full; its regression retains the saturated receiver through broker drain.

Root navigation now creates a native request scope per navigation, uses checked redirect-local IDs and passes the exact scope to no-content/download cancellation. This removes dependence on page revision and prevents collision with unrelated retained page operations. A regression retains an older page request through a new no-content navigation and verifies independent native transport ownership. Deferred readiness deadline overflow is rejected before invoking admission/construction, with regression source.

The full root fetch actor continuation is still required: start/policy/header/download work must yield with owned source authority, immutable history destination and one absolute deadline. Scheduled constructors alone do not close it. No build, test, formatter or qualification was run.

## Retained transport admission and header owner

`navigation_fetch::PendingHeaders` now owns queued/running transport admission, the response receiver and the original deadline. Admission uses the existing bounded resource IO executor and the context's resource-job teardown barrier. Polling never blocks; retirement before queued admission suppresses transport creation, and a late admitted operation is dropped into native acknowledged cancellation custody. Accepted headers transfer the operation exactly once without body demand. Generated regression sources cover held admission, retirement/timeout before late completion, malformed pre-header events, exact transfer and no premature demand.

The V8 root loader now uses this owner instead of directly blocking inside `start_operation`. Its compatibility wait pumps incumbent native work while admission/headers remain pending. Full page-actor retention of the surrounding source/destination/history/redirect state is still open; this adapter does not claim command dispatch can yet proceed during the loader call. Initial/redirect policy RPC and download collection remain to be moved into owned continuations.

An additional production wrapper gap was fixed: audited `SharedNetworkBroker::start_operation` used an idle blocking-receive thread that did not notice its consumer retiring. It now uses the shared network runtime and selects consumer closure against upstream events. The native cancellation/retry regression now covers audit enabled and disabled. No execution was performed.

## Native navigation policy continuation

Initial form resubmission authorization, original form/script source-policy queries and redirect form/navigation checks now use `PendingPolicy` on the existing bounded IO lanes. Each captures its exact request, retains the context resource-job guard through response acknowledgement and shares the original navigation deadline. The adapter explicitly permits only non-publishing policy/source queries, so discarded policy responses cannot orphan newly issued resource grants. The loader's compatibility wait checks the carried native operation and controlled-navigation checkpoint while pumping incumbent work.

A socket-backed regression source holds a policy response, expires the consumer, proves teardown still sees the admitted job, then releases/acknowledges the response and drains retained ownership. No tests ran. Page-actor retention and immutable history postcommit continuation remain open; pumping native events in the compatibility wait does not establish that actor commands can run.

## Traversal identity and inline policy integration

Traversal now resolves the accepted native history key/id/document identity instead of trusting a pre-wait numeric index. The constructor re-resolves before preparing the committed history, and restoration payload selection uses the resolved target. Missing/duplicate accepted identities reject. After fetch, traversal no longer overwrites the index published by navigation; 204/205 therefore preserve the incumbent entry. Form restoration requires the accepted entry to remain current. Regression sources cover shifted/removed/duplicate identities and no-content traversal preserving current controls/document/index.

Root parser-module, script execution and dynamic inline policy requests now carry native Creator and parser-inserted provenance, coordinated with the parent/frame lanes. Dynamic inline source preparation previously ignored ScriptLoadFailed responses; it now requires explicit ScriptAuthorized and propagates typed denial. A native transport regression source covers creator-bearing denial. No build or tests were run.

## Retained download body continuation

HTTP download response collection now uses `PendingDownloadBody`: exact operation identity, retained byte buffer, one outstanding demand, a 32-event slice, original deadline and checked/fallible allocation under the 16 MiB limit. Terminal/error paths release transport and bytes; EOF transfers bytes once. Native download authorization uses retained policy IO before any body demand. The compatibility driver checks the original native Creator and navigation operation before each poll, pumps native events while pending, and rechecks the Creator before quarantine publication. The old synchronous body receive loop and direct broker cancellation call were removed from this caller; native receiver custody retains acknowledged cancellation.

Generated regression sources cover pending demand, binary byte transfer, ready-burst fairness, stale identity, capacity, timeout and disconnected input. Full actor retention and asynchronous final quarantine publication remain open; no test/build execution occurred.

## Request metadata commits with destination history

HTTP navigation now passes an owned `NavigationCommitResource` into document construction. The staged destination entry receives the final redirected POST resource and form authority before destination scripts and native history updates are reconciled. The old V8 caller mutation of whichever entry happened to be current after construction is removed. This preserves the original destination's request metadata when its first script pushes another entry, and lets same-document entries inherit the correct committed resource. Failed/no-content navigation never publishes this staged metadata.

A production-loader regression source covers an initial POST whose destination script pushes history, both directly and after a 303 rewrite to GET. It checks the sent requests and the resource retained by every destination entry. No tests or builds were executed.

## Retained complete HTTP fetch phase

`navigation_http::HttpNavigation` now owns current URL, redirected POST resource, original form authority, cookie initiator context, referrer source/policy, redirect counts, native broker scope, deadline and the active header/form-policy/navigation-policy phase. One poll advances one state; no policy/header receive or explicit cancellation RPC blocks the state machine. Redirect response retirement releases exact transport custody before the next policy check. Final response ownership transfers once to document/download handling.

The V8 loader uses this transaction and emits its request-start records. Its prior inline request/redirect/policy loop is removed from the V8 path; the non-V8 loader remains separately scoped. Regression sources cover 303/307 body rules, cross-site cookie-chain tracking with retained source, response referrer policy, independent request IDs, redirect limit and original deadline. Existing redirect deadline source now checks owned-operation drain rather than demanding cancellation of a response that may already be terminal.

This completes a retained HTTP fetch phase, not the enclosing actor transaction: initial policy/source admission, beforeunload, local schemes, commit/history restoration and replies still require the caller-level yield integration. No build/tests were run.

## Carried service navigation source integration

PageHandle::goto_service_resource and optional local/worker GotoResource service authority now carry the parent's sealed service-source admission. The root command wrapper rejects mixed form/POST/referrer overrides and scopes the admission through native navigation. ReadServiceNavigationSource validates the exact target and requested URL, then supplies source URL, referrer policy, opaque-origin handling, top-level URL and cookie ancestry to the shared HttpNavigation transaction. No independent service HTTP fetch path or invented initial-document URL is used.

Parent/service regression generation is coordinated in the P/W lane. Existing-client local navigation and full actor-retained scoped authority remain under integration; no tests/builds ran.

## Final qualification retains exact generation declaration

The final qualifier now records the original generation-gate reference in preflight and final evidence. Every receipt, component and subprocess validator checks that identity before and after evaluation, including exceptional exits. An observed change or missing declaration is latched for the rest of the qualification; restoring the original file cannot resume later validators or clear the failure. Final generation validation remains required independently.

Regression sources cover mutation during a successful Core validator, removal during a failing component, restored bytes before final validation, prevention of later validator launches, and preservation of the original hash. Source inspection only; no tests/builds were run.

## Direct native module graph continuation

Source review confirmed Unix dynamic imports and parser sources already shared pollable graph infrastructure; the old g/h description was stale. The remaining native direct-host entry is now integrated: `execute_module` / `execute_module_script` first perform bounded graph work and retain pending graphs between host turns. Ready/import-free and explicit-source entries retain immediate semantics. Direct snapshots are byte-accounted, share import admission/slots/fetch registry, preserve exact environment and deadline, and retire without transferring work into replacement environments. Failed dispatch preserves queued successors. Unix resolver networking is removed; linking consumes prepared/cache-owned sources. The existing TLA promise/error machinery remains the evaluation path.

Generated regressions cover one held dependency shared by a direct static root and duplicate dynamic imports, unrelated task progress, direct source/admission retirement, failure successor custody, and redirected native source completion. No execution. Non-Unix legacy broker fallback and complete caller/error-reporting reconciliation remain open; shared registry keys preserve policy and parser provenance rather than merging unauthorized requests.

## Portable core and worker startup follow-through

The preceding non-Unix resolver limitation is superseded: graph/import continuation, host readiness/drain/retirement, typed/worker prepared root identity, dynamic script graph/source queues and shared limits are now platform-neutral. Parent context IPC alone stays Unix-specific. All module resolver fetch fallbacks are removed; network work belongs to ModuleGraphBackend IO jobs. Worker startup retains its graph until JS/JSON dependency completion, then enters prepared linking and existing evaluation/TLA readiness. Termination releases startup graph custody before retiring the runtime. Generated held-source tests cover no early author execution/Ready and cancellation during dependency IO. Graph publication now retains response policies by requested URL, so redirect aliases do not merge distinct module credentials/referrer context.

Primary caller source audit found production script paths use the prepared-request entry after graph completion. Remaining plain direct-execution calls are regression fixtures; ready/offline/no-import calls preserve immediate semantics, and delayed native fixtures explicitly drain pending work.

Plan §2 states “The primary current target is the signed macOS native runtime” and requires per-claimed-platform evidence. The non-Unix complete HTML parser is not claimed here: its ordered_scripts branch still differs from the Unix resource scheduler. Portable module core source generation does not assert a complete Windows browser or qualify any platform without execution. No builds/tests have run.

## L-02.i complete caller failure reconciliation — September 17

Source generation and inspection only; no build, test or generator execution.

| Boundary / consumer | Native failure handling and cache rule |
|---|---|
| `ContextService::{load_script,run_script_load}` / `ContextScriptLoad` | ScriptLoadFailed carries Fetch/Cancelled/Capacity; transport and authority errors remain infrastructure. No exception-name or message classification. |
| `module_graph_backend::ModuleGraphBackend::{fetch,fetch_script}` | Parent typed reply, worker ScriptFailureKind, broker ScriptLoadFailure and native admission flow into HostError. Cancellation before IO is now Cancelled. Unclassified transport and missing prepared-source infrastructure remain retryable. |
| `module_graph_load::poll_with_backend` | Only explicit ScriptLoadFailure::Fetch publishes a failed JS/JSON/CSS response. Other HostError variants propagate without mutating the map. Native realm/document replacement now carries Cancelled. A previously published response still wins against a late result. |
| `host::{adopt_module_script_response,cached_module_script_response}` | Only explicit Fetch is sticky; generic Execution text cannot poison the map. Failed cache hits return typed Fetch. Existing first-response and source-limit rules remain intact. |
| Parser `adopt_parser_module_result`, parent parser completion, dynamic source completion | ParserScriptFailure classification remains native; only Fetch enters the shared map. Parent-response and unknown-response cases were inspected; missing/invalid transport responses do not become fetch failures. |
| `typed_modules::{typed_module_source,load_typed_module_record}` | Failed response-map lookup yields typed Fetch. Missing preparation stays infrastructure; local failure removes compiled pending ownership; actual JSON parse errors remain retained, while V8 termination removes the pending record. |
| Worker prepared JavaScript / typed module resolvers | Read only prepared response maps; they have no synchronous network fallback. A JS exception at the final author boundary does not insert a failed fetch entry. |
| `agent_script::{load_with_cache,decode_imported_source}` | Local constructor sites now pass ScriptFailureKind explicitly, replacing the earlier exception-name classification helper. Network/decoder variants preserve Fetch/Cancelled/Capacity/Infrastructure metadata. |
| Service stored imports / resource publication | MIME rejection and missing installed resource map entry are Fetch. Missing legacy metadata/immutable publication are infrastructure; resource map bounds are Capacity. Fetch cancellation and event admission capacity carry explicit metadata. An accepted respondWith rejection is a fetch result; earlier dispatch/transport failures remain infrastructure and any existing typed failure is preserved. |
| Blob entry/import producer | Checks cancellation before lookup and after decoding. Revoked/unknown and cross-origin resources retain typed Fetch; stale caller/authority lookup remains infrastructure. Failed creator-authority lookup no longer silently revokes a blob; explicit document/agent retirement owns revocation. |

Generated regression sources extend host first-response controls with unclassified NetworkError-looking text, exercise unclassified/cancelled/capacity/infrastructure failures across all three graph module kinds, assert typed stored-service MIME/missing-resource/immutable-publication outcomes, assert pre-admission and in-flight service cancellation metadata, and cover blob cancellation before authority lookup. Existing blob tests retain origin/revocation/retained-copy controls.

This closes the recorded L-02.i source caller inventory. It does not claim passing execution, full L package completion, or change protocol versions or the fixed Core scope.

## Standalone actor resource caller reconciliation — September 17

Source inspection confirms that macOS/Unix standalone construction (`context=None`) uses `schedule_script_auto` for blocking/async/deferred classics and external module roots, retained `ModuleGraphLoad` for dependencies, and `schedule_stylesheet_auto` for stylesheet/import work. Snapshot completion and frame/document.write parsing use the same page-owned queues. Runtime construction converts scheduled strings to streams, passes scheduling through parsed/stream constructors, and retains pending replies; explicit synchronous compatibility constructors alone pump native page events until their bounded deadline. `ModuleGraphLoad::wait_for_completion` and shared-fetch blocking receive are test-only.

A missing standalone frame route was found: external frame sources with a broker but no parent context only consulted the decoded resource map. Generated `standalone_frame_resource::Loader` now captures broker, document CSP, ancestor/referrer/cookie context and request identities; it performs bounded cancellable fetch/redirect/decode on the resource IO executor. `FrameResourcePreparation` owns local cancellation, native frame identity and byte reservations. Parser missing-source frames and dynamic standalone frames enter this retained route. Response CSP/sandbox metadata is installed before the existing child parser prepares scripts; explicit supplied frame sources keep their prior priority. The same native completion path rejects stale/replaced owners and publishes failure documents without author execution. Added scheduled snapshot/stream fixture source covering a frame stylesheet, document.write external classic, static module and dependency, plus pre-admission and owner-drop cancellation regressions.

The standalone source-resolution failure adapter was also reconciled with L-02.i: generic resolution infrastructure no longer gets relabeled as Fetch before parser completion.

**Remaining platform boundary:** plan §2 identifies signed macOS as the primary target and requires evidence for every claimed platform, but does not explicitly exclude non-Unix. The existing `cfg(not(unix))` parser resource fallback has therefore been preserved, not replaced with a failure merely to remove a blocking call. Full non-Unix parser scheduler extraction and its actor/resource callers remain an open generation requirement if that platform is claimed; these additions do not certify portability or whole R-06/L-02 completion. No builds, tests or generators ran.


### R-06.k parser ordering and retained owner review (2026-09-17)

Source-only caller review covers live HTML root/frame and document.write gates,
XML root/frame event publication, snapshot event preparation and per-document
round-robin scheduling. Existing regression sources cover nested inline writes
before caller return, written external script suffix order, XML write rejection,
observer changes before script preparation, idle XML input, oversized HTML tokens,
and stop/reopen retirement. Primary HTML input slices are 64 KiB on scalar
boundaries; parser completion work is bounded to 64, frame document turns to 64,
and each document async script turn to eight. XML advances at most 256 events;
snapshot publication takes one discovery per parser poll. These are source
algorithm observations, not measured latency or passing conformance results.

Confirmed fixes generated in `frame_script_preparation.rs`:

- Queued stylesheets remain in place until capture/admission succeeds; stale
  entries are explicitly removed. Failure cannot let a following parser script
  overtake a stylesheet. Regression sources inject admission failure, retry,
  and stop before retry with a must-not-fetch URL.
- Already-returned HTML continuations are restored before reporting frame
  publication errors; asynchronous parser scheduler ownership is borrowed in
  place across fallible completion/module processing.
- A consuming HTML parser resume failure explicitly stops that document load
  and discards its failed owner. It cannot later appear to be an empty successful
  parse and publish DOMContentLoaded/load. A terminal-input negative regression
  checks this behavior. This does not claim resumable recovery of arbitrary
  errors after partial HTML tokenizer/script consumption; that requires a
  retained phase conversion rather than reconstructing/replaying callbacks.

The broad R-06.k source/acceptance gate stays open: complete transient-error
recovery inside the consuming HTML resume algorithm and full caller fairness
qualification are not established by these changes. No execution occurred.

## L-03/L-05 resource caller policy and cancellation review — September 17

Confirmed and generated missing active mixed-content checks after each CSP upgrade in parent script/style/font/frame resource loading, standalone script/CSS/frame loading and agent script loading. Trustworthy loopback targets remain allowed; top-level navigation keeps its separate mixed-content semantics. Standalone frame redirects now preserve reduced referrer state and compare schemeful sites for cookie redirect taint. Added source regressions for denied script redirects, worker import rejection and same-site cross-origin frame redirects without referrer restoration.

Parent resource cancellation now reuses the exact-document reserve/run lease contract for typed LoadPageResource. `ResourceCancellation` keeps weak active leases, while caller-owned cancel-on-drop handles revoke admitted work independently of worker closure lifetime. Parser/dynamic linked stylesheets and imports, computed font demand, parser frames and dynamic frames use the contract. Parent frame-header and resource-body waits consume cancellation, and script/resource redirect chains retain one deadline. Parser style and frame custody now drops cancellation handles when native owner/revision/connection state becomes stale. Added a stylesheet/font/frame held-transport cancellation regression table. The existing frame-response lease continues owning bodies after header publication; late response release is not replaced by the request token.

ResourceInitiator carries optional linked-stylesheet nonce/integrity/crossorigin metadata. Parser/dynamic link preparation captures it; @import clears root element options. Parent stylesheet requests now carry nonce through per-hop CSP admission, CORS credentials and redirect origin taint, COEP/CORP response checks, and SRI verification before decoding. Fonts/images share applicable embedder checks. Added a redirected stylesheet matrix for correct/incorrect nonce, CORS omission, bad digest and require-corp behavior. Unsupported SRI metadata is parsed before response eligibility so ignored metadata does not invent a check.

These are source-generation findings, not completion of L-03/L-05. Remaining source reconciliation includes Request/fetch integrity and whether mode/redirect metadata reaches parent and agent fetch, image upgradable-content distinctions, response embedding/permissions policy and full negative case/accounting coverage. Build/tests/services remain deferred.


### R-06.k.2 primary HTML retained phases (2026-09-17)

The earlier consuming-HTML recovery gap above is superseded for the primary
Unix runtime. HTML root construction hands the untouched native tokenizer to
`FrameScriptPreparation`; HTML frame and document.write owners already use
that owner. `poll_html_parser` borrows tokenizer/source state, saves the exact
unconsumed suffix and transfers typed events before wrapper publication or
script/resource admission. It uses the existing retained event/job/readiness
queues also used by XML and snapshots. The old consuming `LiveParserState::resume`
is now compiled only for non-Unix compatibility and explicit legacy unit sources.
No non-Unix scheduler functionality was removed.

Failure boundaries and ownership:

- A terminal DocumentInput error or retired/replaced document ends its old load.
  Metadata, wrapper, resource, capacity and lifecycle publication failures retain
  their exact owner and retry deadline. Root error cleanup recognizes retained
  owners rather than discarding them as failed parses.
- Native script jobs retain successful source responses while module-map
  publication recovers. Typed infrastructure/capacity preparation failures retry;
  true fetch failures queue the script error. Graph preparation retries explicit
  native infrastructure/capacity/cancellation failures without permanent module
  failure entries.
- Classic compile/run and module evaluation entry callbacks record entry before
  author code. Script body progress, completion-event receipt and post-script
  checkpoint are separate. A later error never reruns an entered body. Script
  load events use the existing retained native event engine and parser-stream
  receipts, preserving listener suffix and event identity while retaining normal
  dispatch-before-following-parser-script ordering.
- Stylesheet content/dependency publication and completion-event admission are
  separate retained phases. The installed sheet continues blocking parser scripts
  while its completion event is retried; stale revision/document targets are
  discarded. Native dispatch handles the event listener progress.
- Readiness polls buffer at most one arriving input chunk/error/EOF. Ordered
  script waits do not repeatedly request tokenizer work. Existing 64 KiB scalar
  slices, 64 document turns and eight async jobs per document remain bounded.
  The gate resumes before a ready parser script, preserving synchronous nested
  document.write visibility and its input priority over the network suffix.

Generated regression sources cover HTML wrapper publication retry, real script
byte capacity recovery, post-body failure without nested-write/body replay,
frame child retry, stop/replacement retirement, terminal input failure, and
stylesheet event-admission retry. A binding regression checks retained script
load event identity and callback suffix. Existing root pending-owner assertions
and synthetic async-source fixture now use the shared owner rather than assuming
all roots remain in `pending_parser`. These are source changes only: no builds,
tests, services, generators or WPT were run. Broad R-06.k execution/fairness
qualification and independent CE parser integration remain separate gates.

### Fetch and final-frame policy integration — September 17

Generated `FetchOptions` transport from Request/fetch through native Window and agent HTTP, including mode, redirect and integrity. Request construction/cloning exposes integrity; no-CORS guards restrict methods and initial/subsequent header mutations, with the same safelist primitive used by native transport. Service-worker FetchEvent requests carry integrity. Internal navigation-mode requests retain their native brand rather than becoming author-selectable RequestInit modes.

Window and agent routes enforce same-origin mode on each hop, redirect error/manual behavior, CORS and no-CORS response filtering. Typed opaque/opaqueredirect results hide URL/status/headers/body before JavaScript publication. Both routes preserve full body cancellation custody while SRI verifies bounded representation bytes; data responses and the local blob path use the same verifier. Agent fetch now supports local data responses and preserves reduced referrer state across redirects. Total header/integrity deadlines cannot reset per hop. Valid opaque service responses retain their filtering rather than being rejected solely for status zero. Native response/body, mode/redirect/integrity tables and Request guard regression sources were added; none were executed.

Final-frame response checks now use full captured native ancestry at the resource loader, with the broker retaining its mandatory root ancestor. Redirect responses do not impose frame-ancestors/X-Frame-Options on the eventual document. A shared X-Frame-Options processor handles case, repeated/combined values, quoted invalid tokens, HTTP whitespace, all-ancestor SAMEORIGIN and enforced frame-ancestors precedence. Native and standalone final-frame callers use it; source regressions follow pinned x-frame-options multiple/invalid/get-decode-split cases.

The earlier resource paragraph's Request/fetch transport gap is now generated for these named paths. Full L-03/L-05 remains open: RequestInit conversion/reset completeness (including non-empty init resetting inherited navigation mode), image upgradable-content distinctions, complete embedding/Permissions Policy/COEP inheritance and report delivery, CSP redirected path matching, and exhaustive destination cancellation/accounting and negative-case mapping. See `L03-L05-DESTINATION-POLICY-SOURCE-MATRIX-2026-09-17.md`. These are source observations, not a passing build or conformance result.


## Late HTML encoding replay — source generation, 2026-09-17

Pinned `html/syntax/charset` callers require a late declaration in the original head to change a tentative decoder, including after 1 KiB and after high bytes in comments. Declarations after leaving the head, inside templates/scripted noscript, and document.write do not acquire this authority. The 1 KiB prescan is unchanged; early script execution remains a parser boundary.

- Native html5ever TreeSink emits opted-in encoding indicators and a head-exit commitment. `StreamingParser` resource boundaries carry those events without a second HTML scanner. Only meta elements generate indicators.
- `IncrementalTextDecoder` tracks tentative/certain confidence, normalizes UTF-16 and x-user-defined meta labels, commits valid same-encoding declarations, and permits one accepted replay. BOM/HTTP declarations remain certain.
- `HtmlEncodingReplay` retains original admitted bytes under 16 MiB/document and 64 MiB aggregate limits; allocation and quota failures are explicit. Root HTTP and frame HTTP input retain terminal/cancellation ownership. Buffered root/local/standalone frame input now keeps raw bytes instead of re-encoding previously decoded text. No replay performs another request.
- The Unix production `FrameScriptPreparation` event queue and legacy consuming parser path both handle restart. The production path performs read-only decoder preflight, restores authenticated policy before mutating decoder/raw bytes (retaining the event across ACK failure), resets only the same Document's provisional children/parser queues, and releases superseded style-source custody. Navigation-owned metadata, strict origin, document identity, referrer and history remain attached to the existing Document. Stale stylesheet event delivery already requires connected owner/revision and drops removed provisional nodes.
- A parser-owned author-effect flag is committed before native parser custom-element construction and event callbacks. Head-exit/EOF also commit input; ScriptReady preparation alone does not commit certainty. Restart is disallowed after such effects; accepted replay is never allowed to execute the early script twice.
- Parent parser-policy checkpoints capture the inherited/header baseline, authenticate exact document owner, allow one idempotently acknowledged restore, and release through retained cleanup custody even when response delivery or the document route fails. Retrying an acknowledged restore does not overwrite policies installed by the replay.
- Generated decoder split/commit/budget regressions, parser indicator eligibility cases, native buffered late-head/script/certain-header cases, and parent restore/foreign-owner/idempotent-release cases. These are unrun.

Additional source regressions now cover the native data-frame route, frame stream replay without early terminal acknowledgement, read-only preflight/restore failure retry, retired-owner restore rejection and cleanup, and srcdoc/archive separation. Archive review found only authored srcdoc strings enter `FrameRealm.source`; replayable responses use policy-checked refetch, while root captures replace decoded source on replay. Shared classic `script.run`, module evaluation and deferred import evaluation now commit the same parser author-effect gate immediately before V8 evaluation; module preparation alone does not. Final source review preserves native policy baseline, Document metadata/referrer, stream EOF ownership and exact retired-owner cleanup. This closes generation for the bounded late-HTML-replay group; broader encoding/MIME requirements and execution qualification remain separate. This is a bounded source update; L-01 and full loading/qualification gates remain open. No builds, tests or generators ran.

### Coordinator HTML replay cleanup review

Moved frame cancellation and stylesheet identity fields from DynamicScriptScheduler to ParserResourceScheduler, matching all existing initializers and users. Production encoding replay now replaces the provisional parser resource scheduler with the same page budgets and drops deferred/blocking script owners, deferred XML/HTML script jobs and their source reservations. This prevents the discarded decoding pass from retaining callbacks or resource completions into replay. The policy-retry regression now checks that preflight failure keeps the old scheduler lifetime and successful restart releases it. Independent review also identified premature ScriptReady certainty; the resource lane is correcting preparation versus actual execution and adding defer/importmap/CSP controls. Source only; no execution.

### Decoder selection publication source review

The new detector retains a bounded suspect-byte sample and publishes initial ASCII without replay. Source review identified that FrameScriptPreparation published encoding metadata only before the next input read, although that read can select a different encoding. It now publishes the selected encoding again before parsing the returned text, retaining consumed text if metadata publication fails. Added native script-in-selecting-chunk coverage and decoder boundary/empty-chunk regression sources for UTF-8 scalars around the 1024-byte sample. No execution; detector heuristic/platform conformance and full L-01 remain open.

### Coordinator parser-module checkpoint review

Checkpoint preload metadata accounting now uses the complete shared key byte measure, including media and nonce strings, for both seen admissions and queued jobs. Saturating accumulation preserves quota rejection on overflow. Added a source regression that captures checkpoints before and after a preload key with long media/nonce fields and verifies the exact retained-byte delta. Reviewed exact-environment rejection and idempotent restore paths. Baseline in-flight graph completion and publication interactions remain under source reconciliation; this entry does not close L-01 or module loading. No build or test execution.

### Production actor MIME prefix integration

The production HTTP actor now has a retained MIME-prefix phase between response headers and document/download admission. Response metadata stays with the transaction while DocumentMimeSniff owns the body operation; bounded polls yield without waiting on transport. Every poll keeps the existing source operation, load epoch, cancellation, deadline and parent-ack guards. Computed binary types enter the existing authorized download pipeline; other computed types choose the parser while original supplied response headers remain intact. Added source regression for pending-prefix incumbent usability, cancellation releasing the receiver, and exact-body handoff without refetch. The older blocking navigation helper is a test/non-V8 compatibility path; it was not the production actor route. No execution.


## DOM script UTF-16 compilation provenance (2026-09-17, source only)

Native `ScriptSource::{PlainUtf8,DomUtf16}` now separates decoded fetched source
from private String-backed DOM storage. `prepare_script_text` retains accepted
TrustedScript code units; parser `ScriptRecord`, `PreparedDynamicScript`, pending
module graph roots and direct-module continuations retain that distinction.
Classic V8 and module V8 compilation allocate two-byte source strings explicitly.
Module parse keys hash the resulting UTF-16 units, so identical encoded storage
with different provenance cannot share parse outcomes.

Caller review: connected insertion, document.write parser gates, snapshot/live
root and frame records, dynamic inline classic/module, native direct modules,
worker/fetched dependency roots and typed JSON modules. Fetched String callers
remain PlainUtf8. Dynamic loader response bookkeeping stays UTF-8; inline graph
and execution consumers select the frozen prepared DOM source rather than that
bookkeeping copy. Existing source-table dependencies and fetched JSON/CSS/text
payloads are not passed through the DOM decoder. CSP UTF-8 hashing/reporting and
import-map/speculation JSON handling use an explicit scalar boundary; this does
not change the existing JSON parser's representation capabilities.

Generated regression sources cover lone-surrogate classic/module execution,
document.write, frame realm selection, accepted default-policy source, frozen
parser records after later mutation, and fetched reserved-prefix versus DOM
module parse/cache identity. No compilation, tests or WPT have run. This bounded
source item does not close the broader L or Trusted Types conformance gates.
