# Standalone Window Fetch source closure — 2026-09-18

Status: production and regression sources generated; **not built or executed**. Global all-generation-before-validation directive remains active. This is a bounded transport closure, not a WPT conformance result.

## Native integration

`crates/oryn-native/src/standalone_platform_http.rs` implements `PlatformHost` for a native standalone Window with `current_fetch_endpoint`, installed at the end of `host.rs::install_native_document` after document provenance and environment binding. Parent-context Windows retain their existing `ContextPlatformHost` route. The modern generated Fetch binding now has an ordinary standalone HTTP endpoint; no legacy `perform_fetch`, synthetic browser page, or Agent principal is substituted.

Each accepted request captures native document generation/authority epoch, original Window owner, policy snapshot, document URL/base/referrer, origin/top/ancestors and blob registry bytes on the realm thread. Executors receive only frozen native values. Ordinary keepalive reserves the same `StandaloneOwner` quota as deferred Fetch and a bounded orphan drain before admission.

## Production and regression matrix

| Surface | Generated production | Authored regression source |
| --- | --- | --- |
| Modern JS Fetch | `install`, `Host::submit`, generated binding completion pump | Real RawV8Host `fetch('/data').then(r => r.text())` with operation broker |
| Fetch authority | Current native CSP/Connect destination, URL schemes/credentials, mixed content, native Fetch Metadata, request method/header and no-cors guards | Same-origin denial; CSP denial before transport; native mode metadata |
| CORS and credentials | Preflight options, same-origin/include/omit credentials, native preflight/response checks, exposed response headers, cookie filtering | PUT preflight, credentialed response, Reload cache, hidden headers/set-cookie |
| Origin and referrer | CORS-tainted and non-GET/HEAD Origin; native referrer policy including non-CORS null suppression; redirect policy | Same-origin POST, cross-origin policy suppression, downgrade and tainted Origin cases; redirect referrer removal |
| Redirects | Follow/error/manual, HTTP(S)-only next URL, hop bound, POST rewrite and credential header removal, origin/site taint | POST-to-GET body/auth/content-type removal, manual opaque redirect |
| SRI/local resources | Original representation bytes before response publication; native data decoding; authorized blob registry and range | Wrong digest cannot publish, empty verified EOF, non-UTF8 data bytes, blob 206 range |
| Response custody | Bounded stream IDs/capacity, demand reads, exact response identity/order, retained cancellation ticket before transport, no blocking UI cleanup | Streamed response, empty verified stream; bad integrity no published stream |
| Keepalive | Shared ordinary/deferred Window byte budget, protected cleanup, explicit cancellation, pre-reserved nonblocking orphan body drain | Combined 16 KiB deferred + 48 KiB ordinary boundary, quota release only EOF, accepted start after retirement, explicit cancellation denial |
| Images/media | Shared HTTP start categories preserve Image/Media CSP and native cookie destination; image-only mixed upgrade | Covered structurally by destination dispatch; dedicated media playback validation remains later global verification |
| Resource timing | Native response headers, hop/transport measurements and terminal stream size; generated binding receives metadata | Existing native timing owner consumes metadata; no fabricated successful WPT timing assertion |

## Bounds and ownership

64 pending/completion or active-stream capacity; 16 MiB representation/upload bound; 256 response headers/256 KiB aggregate; 20 redirects; 30-second native operation deadline. IO lives on the bounded resource executor. Retirement revokes author delivery, cancels ordinary operations and transfers accepted keepalive streams to previously reserved drain custody. Explicit cancellation revokes keepalive too. Cancellation tickets retain quota through physical cancellation acknowledgement; EOF releases quota directly. Response-read cancellation uses a separate atomic flag, so the realm thread never waits on a body read mutex.

## Scope and verification dependencies

- Installation and standalone deferred ownership are now platform independent under the existing `v8-host` feature. `http_cleanup_custody` and keepalive are portable crate modules; the context service reexports the same cleanup module/pool. `runtime::deferred_fetch` retains one shared registry/delivery/scheduler implementation; only parent authority is in `deferred_fetch_context.rs` and its scheduler variant remains Unix-specific. Pure and standalone tests are no longer accidentally gated by the parent transport. No profile substitution or portable test execution is claimed.
- This adapter uses the standalone native network scope. It does not fabricate a BrowserContext service-worker controller. Parent Window and Agent controlled-fetch pipelines are owned by their respective lanes.
- Regression sources are authored and API signatures manually reconciled; no compilation or passing-test claim. Run these with the global build/test/fix phase, then complete WPT and five-family evidence updates.

## Portable extraction and frame owner reconciliation

The plan’s section 2 combined exit requires separate evidence for every claimed platform/execution policy and identifies signed macOS as the primary current qualification target. This source extraction does not turn Linux compilation or unexecuted portable tests into qualification evidence.

Narrow shared edits: `lib.rs` portable keepalive/cleanup declarations; `context_service.rs` same-path cleanup reexport; `runtime.rs` deferred module availability for Unix parent or V8 standalone; `http_cleanup_custody.rs` explicit portable imports; `host.rs` standalone install, deferred-provider capture, creator retirement and frame quota hooks. `deferred_fetch_context.rs` contains the extracted Unix parent implementation in the same Rust module scope, preserving private owner/registry identity and shared budget. No duplicate implementation or cancellation pool is created.

`standalone_platform_http::Installed` binds the transport to the actual native Window creator and binding state. A frame that receives its native lifetime after initial document binding replaces the provisional transport exactly once; repeated binding of the same creator leaves in-flight work intact. Frame discard returns its native deferred container reservation. The new all-platform initial-frame regression asserts the installed creator token and root-page identity; existing modern Fetch, shared quota, retirement and deferred scheduler sources now compile into every applicable V8 platform target.

Shared Origin logic is extracted to `referrer::request_origin_header`; standalone ordinary/deferred producers use it, and parent/Agent lanes are reconciling their producers against the same helper. It distinguishes CORS response taint from a simple non-GET/HEAD Origin header and applies Referrer Policy suppression only to the latter.
