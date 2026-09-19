# Native URL media caller generation audit

Source-only, 2026-09-17. No builds, tests, generators or native media execution have occurred. This supersedes the earlier range-helper-only WAV metadata scope; G-02.g.4 remains open.

## Authoritative callers inspected

Pinned WPT revision `719d5e38fdd0903a18ed9007aba816c98cc491e0`, Core tier from `artifacts/wpt-full-baseline/inventory-v3/coverage.jsonl`.

- `fetch/range/{general.window.js,sw.https.window.js,resources/utils.js,resources/range-sw.js}`: private native Range headers, actual received-byte offsets, WAV metadata preloading, progress/remove cancellation, script/image/link preload and resource timing counts.
- `css/selectors/media/media-{playback-state,playback-state-timing,playing-paused-style-invalidation,loading-state,loading-state-timing}*` and `css/selectors/invalidation/media-*-pseudo-classes-in-has*`: synchronous paused/playing/seek state, currentTime, canplay/playing/seeking/seeked, waiting/stalled/recovery, native selector matching and style invalidation. `common/media.js` selects actual supported MP4/WebM and MP3/Ogg types. Stall helper serves a partial WebM body then waits for an independent POST.
- `html/semantics/the-button-element/command-and-commandfor/on-{audio,video}-behavior.tentative.html`: shared native play/pause/play-pause/mute/toggle-muted behavior, cancellation and command activation; command event/activation integration is owned by the notification lane.
- `mimesniff/media/media-sniff.window.js`: detached element `load()`, content sniff independent of declared response MIME, MP3/FLAC/Ogg/MP4/WAV/WebM metadata. Metadata success is not codec playback proof.
- `content-security-policy/media-src/media-src-7_1.html`: native src/source candidates, source-targeted failure event and loadeddata, actual WebM content.

The initial direct Core scan also found CSP blob/data/redirect, fetch-destination, ORB HLS, cross-origin video, mixed-content auto-upgrade and picture-in-picture permission fixtures. These still require individual caller reconciliation; this list is not a claim that all media-reachable helper functions are covered.

## Generated shared production paths

- Existing MediaStart broker policy/SW/range transport supplies bytes; decoder receives bounded base64 bytes and never fetches an author URL. Parent owner authentication is unchanged. Decoder operations do not require capture permissions.
- `media_capture/url_asset.rs` owns private assets by exact `AgentOwner`, bounded encoded/decoded inventory, real decoded sample requests, pause/seek audio sink reset, foreign-owner rejection and owner/page retirement.
- `decoded_audio.rs` probes and decodes FLAC, MP3, PCM/WAV and Ogg/Vorbis using exact Symphonia 0.5.5; no MIME-label oracle, fabricated samples or fixture URL branches. PCM bytes, channels, rate, finite samples and packet count are bounded. Codec changes within an asset reject explicitly.
- macOS AVFoundation handles supported video/native audio using exclusively created private files; `AVURLAssetReferenceRestrictionForbidAll` prohibits secondary media references. Files and native decoder objects are released with the asset. RGBA frame conversion and real PCM feed existing renderer/audio sink. Native MIME support is queried rather than inferred from extension.
- `media_element.rs` shares URL/native capture public state, pending play resolution, currentTime seeking, looping, ended/pause/timeupdate events, native frame painting and response-origin taint. Release failures retain IDs for retry.
- `media_resource.rs` supports explicit detached load/play, source alternatives and source-targeted error events, progress/stalled deadlines, selected-source identity and metadata/full decode transitions. Existing range and cancellation ownership remains in force.
- DOM, Stylo selector parser/query adapter, Blitz mirror and runtime layout input carry native paused/playing/seeking/buffering/stalled bits. Author attributes cannot set these bits. Initial media elements match paused; non-media nodes do not match.

## Regression sources added

- Real native admitted WAV decode, sample, wrong owner, retirement and malformed resource.
- Portable malformed-container rejection.
- JS URL play promise and native pixel publication, synchronous paused/playing/seek matches, seeking vs seeked checkpoint ordering, load abort, failed decoder release retry and native command dispatch.
- Native DOM initial/clone state and persistent layout `:has(video:stalled)`/playing/seeking invalidation.

## Remaining generation/review obligations

- Complete the direct and transitive Core caller review above, including response redirects/MIME-policy variants, audio/video metadata readiness ordering, source candidate type selection and dynamic source mutation.
- Finish source review of suspension/restore and allocation/callback failure custody, autoplay and native codec capability-to-decode agreement. Verify WebM variants against the selected native decoder before claiming platform coverage.
- Generate remaining positive/negative integration regressions from these caller obligations; exact real transport/codec execution remains deferred.
- Combined context/agent protocol reconciliation and Cargo.lock resolution are root-owned final steps; the new private wire variants and pinned dependency have not been executed or resolved by Cargo.

## PiP and native media document dependency generation

The mandatory reporting fixtures `permissions-policy/reporting/picture-in-picture-reporting.html`
and `picture-in-picture-report-only.html` import `common/media.js` and
`permissions-policy/resources/picture-in-picture.js`. They require a real loaded
video, trusted activation, `requestPictureInPicture()`, and enforce/report-only
ReportingObserver records. These are not optional-feature tests.

Generated sources now include the native DOM API, PictureInPictureWindow and
PictureInPictureEvent brands, native event payloads, policy checks before native
admission, retained enter/leave/resize event tasks and promise settlement,
load/source/disable invalidation, parent owner/activation/policy validation,
per-owner decoded-video surfaces, native frame updates, suspension hiding and
parent retirement cleanup. The existing AppKit main-thread host consumes those
surfaces in floating native panels; actor threads never call AppKit. Headless
runs retain the same bounded decoded output without an OS panel. Non-macOS video
PiP remains unavailable where there is no URL video decoder.

Source regressions cover rejected policy without backend admission, mandatory
report dispositions, missing metadata/activation, retained entry across parked
execution, native window/event identity, resize and exit ordering, malformed
pixel rejection and terminal surface closure. None has run.

Other generated dependency fixes: detached `src` assignment schedules native
selection; explicit metadata preloading is distinguished from ordinary complete
WAV loading; Media destination owns mixed-content upgrade including CORS/custom
ports; direct audio/video navigation creates native media Documents while
original transport custody reaches terminal acknowledgement; BFCache freezes
playback and stops native sinks; media event-handler IDL routes to native event
listeners. Fullscreen style projection now uses the existing Stylo Fullscreen
pseudo/state, viewport UA constraints and black native backdrop, with a retained
StyleEngine enter/exit regression.

MediaStream-backed PiP now resolves the selected native video-track identity,
uses capture samples for its decoded surface, and shares permission/stop/retirement
cleanup. A native regression covers foreign-track denial, actual initial pixels,
suspend/restore visibility and stop invalidation. Output-user-close/resize behavior
and all direct helper routes still need final source review; general media helper closure is not equivalent
to full codec, platform, or runtime qualification. Existing timing detailed
fields and PerformanceObserver/buffer breadth retain their documented gates.

## PermissionsPolicy inspection dependency

Pinned `picture-in-picture-supported-by-permissions-policy.html` requires
`document.permissionsPolicy.features()`, and the default-policy fixture explicitly
requires PiP's default `*` allowlist across cross-origin frames. Generated the
complete four-operation native inspection surface (`features`, `allowedFeatures`,
`allowsFeature`, `getAllowlistForFeature`) with SameObject Document/iframe wrappers,
current native owner-authenticated snapshots and standalone live-parent snapshots.
Admission and inspection share `oryn-js/src/permissions_policy.rs`; the previous
native isolation-policy parser path now reexports that implementation. Snapshot
constraints distinguish inherited enablement from the current document's declared
allowlist, so delegating to a nested frame does not mistakenly apply a grandparent's
container origin list directly to every descendant query. Unknown features and
malformed origin declarations cannot become grants; report-only fields never
change the enforced inspection result.

Generated iframe `allow`/`allowFullscreen` reflection and a single native DOM
container-normalization helper. Legacy allowfullscreen grants `fullscreen *` only
when an explicit fullscreen declaration is absent. Initial parent admission,
standalone frame response/inheritance, live container refresh and inspection all
consume that same normalized declaration. Regression sources cover default lists,
wildcard intersections, nested delegation, report-only separation, malformed
origins, live wrapper/header/container changes, exact native frame retirement and
legacy precedence. No regression or executable has run.

## Media event continuation source review

Resource-loader events now retain an independent native dispatch task after the
resource completion is consumed. Changing/canceling a source does not discard an
already-started listener suffix. Playback events for URL and MediaStream sources
share a bounded native task queue, and play promises settle only after the retained
playing event completes. Seeking event delivery finishes before a later sampling
turn clears seeking. Allocation/callback continuation failures preserve state and
are not converted into a decoder MediaError. Added callback-boundary fault,
suspension, source-cancellation and promise-order regression sources; all remain
unexecuted. Capture play/pause and volume-change events use the same queue.


## Explicit WebM VP9/Opus dependency

Read-only inspection of `media/movie_300.webm` confirms a 320×240 `V_VP9`
track and mono `A_OPUS` track. This is the exact hard-coded payload served by
`css/selectors/media/support/stall-resume.py`, reached by both media loading-state
Core fixtures. `mimesniff/media/resources/webm.webm` is also mono Opus WebM.
This dependency now has an explicit decoder source route, rather than relying on
AVFoundation container support.

- `media_capture/webm.rs` parses bounded EBML segments, finite/unknown-size
  clusters, track metadata, signed block timestamps, SimpleBlock/BlockGroup,
  keyframe references, fixed/Xiph/EBML lacing, Opus packet duration, codec delay,
  and discard padding. Compressed payload bytes are never scanned for sibling
  signatures. Offsets retain the admitted immutable response as their authority.
- `webm_macos.inc` feeds VP9 profile-0 packets into VideoToolbox and mapping-family
  zero mono/stereo Opus packets into AudioToolbox. It preserves decoder prediction,
  decodes actual RGBA/float PCM, applies pre-skip/timestamps, discard padding and
  output gain, seeks from keyframes and bounds decoded PCM and frame allocation.
  `url_asset.rs` integrates metadata/sample/PiP/park/retire and inventory accounting
  through the same exact-owner asset registry. No new URL or network access exists.
- Capability queries instantiate the required native decoders; unknown codecs,
  unsupported VP9 profiles and Vorbis-in-WebM are not advertised by this route. VP8 is now generated through the separately pinned libvpx backend below.
  `build.rs` links the system VideoToolbox framework; no external codec binary or
  Cargo dependency was added. Runtime SDK codec availability still requires the
  final macOS qualification run.
- Pure source regressions cover full containers, unknown cluster boundaries,
  lacing, signed timestamps, codec delay and malformed lengths. A native integration
  regression uses `ORYN_WPT_ROOT/media/movie_300.webm` to require real pixels, PCM,
  backward seek stability, PiP and owner retirement. It is explicitly ignored by
  ordinary unit invocations because it requires the external pinned WPT checkout;
  final qualification must run it explicitly with that checkout. No test ran.

This closes the identified VP9/Opus source dependency, not every WebM profile,
media codec, platform output or full G-02.g.4 qualification obligation.


## G-02.g.4j source review and mandatory qualification producer

The final candidate now requires `native_media_tests`, an immutable native libtest
executable artifact. `run-completion-component.py native-media` calls the new
`run-native-media-qualification.py` producer. It checks the generation declaration
before candidate access or execution, obtains the checkout from the frozen native
control contract, compares both decoder fixtures with blobs at the pinned WPT
revision, then invokes the exact ignored test with `ORYN_WPT_ROOT`. The final
`qualify-completion.py` requires its receipt and independently checks the original
command, cleanup, pinned input hashes, compiled media-source digest and all three
compiled native protocol constants. It rejects zero tests, ignored tests, changed
sources, changed fixtures and unsuccessful decode. This gate has not run.

The compiled digest covers native media sources, native Cargo/build declarations
and workspace Cargo.toml/Cargo.lock; it is distinct from signed worker identity.
The native test binary and the worker remain separate candidate artifacts.
Existing historical reports are not rewritten. Python regression sources exercise
missing/boolean/mismatched protocols, no-test success, ignored-test success,
source changes, fixture substitution and the global generation barrier.

Additional source checks repaired eight-byte EBML size masking, rejected
unsupported content encoding, bounded compressed-packet/lacing operations and
added malformed/truncated/private-codec regression sources. PiP decoded surfaces
now reject duplicate native identities and oversized base64 before decoding;
late close/resize/pixel messages cannot recreate a closed surface. Source tests
cover close-after-park and stale publication. The AppKit adapter continues to
consume retained pixel snapshots and reports user close/resize through this
native surface state; real window interaction remains final qualification.

`common/media.js` explicitly prefers `video/webm; codecs="vp9, opus"`; its MP4
fallback is used only when actual native codec admission fails. The mandatory
`media-src-7_1.html` source/src-element fixture uses `media/A4.webm`, whose pinned
headers also identify VP9/Opus. MIME-sniff WebM is Opus-only and now follows the
same decoder admission. Broader CSP redirect/blob/data, fetch-destination and
cross-origin media routes still require their recorded caller-level review;
G-02.g.4j and the combined helper gate therefore remain open.

## Poster dependency closure (source generated, unrun)

The pinned Fetch Metadata `element-video-poster` helper loads an SVG poster and
waits for `video.clientWidth === 123`; a request-only shim cannot satisfy it.
Native image discovery now includes video poster URL reflection, independent
poster bytes/type/dimensions and shared native image policy/stream cancellation.
Poster completion does not touch decoded playback/canvas pixels or emit media
load/error events. Raster and SVG posters project through ordinary layout,
raster/print and SVG foreignObject image paths. The native showing-poster flag
selects intrinsic size/paint; play/autoplay and metadata-ready seek hide it,
resource selection resets it. Invalid/replaced posters preserve playback state.
Clone/import resets private poster state; rendering snapshots retain it. Source
regressions cover SVG size, URL/base reflection, no-CORS poster admission,
replacement, malformed bytes, event suppression and clone/snapshot separation.

The separate AudioWorklet/ClearKey permission/report-only dependencies remain
required generation work; none of these rows claims runtime conformance.

## AudioWorklet Core caller generation

The pinned Core inventory contains 42 AudioWorklet source paths: 20 CSP generated
worklet/import-data paths, two CSP eval/Trusted Types eval cases, two Fetch Metadata
paths and 18 mixed-content worklet/import-data paths. Their transitive helpers
construct AudioContext or OfflineAudioContext, call audioWorklet.addModule, and
(in the eval cases) construct a registered AudioWorkletNode and exchange messages
through its native MessagePort. They do not call startRendering or render a DSP
graph. Full Web Audio rendering is not established by this bounded inventory.

Generated production paths now include independent worklet globals, native
processor registration/construction, retained native message ports, native module
graph preparation/evaluation, repeated module loads, credential propagation,
creator CSP/Trusted Types/secure-origin inheritance, script CSP destination,
CORS cross-origin entry admission and audioworklet Fetch Metadata. Worklet global
creation omits Window and Worker APIs. Original graph parse exceptions are retained
before linking; failed later loads reject while preserving earlier registration.
Window promises/events retain their queue until publication, and close/retire
release the authenticated agent connection. Worklet sample rate comes from the
native audio context rather than a fixed value.

Regression sources cover native globals/registration error brands and getter
exceptions, creator eval policy, later module loads, public end-to-end processor
messaging after syntax failure, CORS positive/negative entry admission, worker-src
versus script-src and secure data-root imports. All remain unrun. Full runtime
qualification, generated IDL output, combined protocol reconciliation, ClearKey
report-only admission and VP8 capability required a subsequent source group; see the generation delta below.

### Encrypted-media policy and configuration source delta (not closed)

Native secure-Window requestMediaKeySystemAccess and MediaKeySystemAccess identity,
keySystem/getConfiguration snapshot generation now use Web IDL dictionary/sequence
conversion, ordered ClearKey configuration selection, native codec admission,
enforced SecurityError and report-only ReportingObserver producers. Pure and V8
regression sources distinguish policy allowance from real codec support. No tests
were executed. Literal Core callers are the two encrypted-media reporting pages.
The positive report-only page requests VP8 WebM. Native VP8 decode is now generated using the pinned libvpx backend below; execution remains unverified. MediaKeys/session decryption is not implemented by this
access-selection change, and no complete EME capability or conformance is claimed.

### Native VP8 backend and required qualification delta

Vendored libvpx v1.17.0 (2026-08-07 release), exact upstream commit
`6df3ec34557879fff673706f4a1d9fbd0f3a6f0e`, supplies the actual VP8 decoder.
`vendor/libvpx/ORYN-PROVENANCE.json` records archive SHA256 and upstream URL;
LICENSE/PATENTS/AUTHORS remain with the source. Only source retrieval/extraction
occurred. Cargo build.rs will configure and compile a decoder-only generic-C,
PIC static library in OUT_DIR; VP8 encoder, VP9, runtime CPU detection, tools,
examples, docs and upstream tests are disabled. No downloaded code was executed.

The native WebM adapter accepts bounded V_VP8 tracks, validates keyframe sync and
track dimensions before libvpx decode, converts real I420 frames to BGRA/RGBA,
resets reference state on seeks, and destroys the decoder with its exact native
asset owner. Capability admission initializes this linked decoder. Wider dynamic
resolution changes and Vorbis-in-WebM are not claimed.

The existing mandatory native-media qualification test now also decodes pinned
`media/rgb100.webm`, checks actual colored opaque pixels, seeks back to the first
frame, and rejects an oversized encoded keyframe. The producer binds that fixture's
Git blob and all vendored source paths/bytes in its compiled source identity;
source-mutation negatives cover vendored decoder code. Qualification is deferred.

### Independent native tool invocation source review — September 17

The libvpx build recipe now preserves both compiler and archiver executable paths, argument boundaries and command-specific environment through OUT_DIR wrappers. Upstream configure/make expands tool variables through shell word splitting; passing the archiver executable directly would lose cc-selected arguments/environment and fail with spaces in tool paths. The wrapper source handles explicit environment removals before assignments. This is source inspection/editing only: configure, make and all decoder tests remain unrun.
