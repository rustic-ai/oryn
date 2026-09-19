# L-01 WebVTT helper source reconciliation

Status: the mapped Core/five-family track dependency sources and regression sources are generated; independent source review and qualification remain open. No build, test, IDL generator or WPT execution has occurred for this delivery. This is not a conformance result.

## Pinned Core callers

The inventory is `artifacts/wpt-full-baseline/inventory-v3/coverage.jsonl`, with the pinned WPT checkout ending in `719d5e38fdd0903a18ed9007aba816c98cc491e0`.

- `html/infrastructure/urls/resolving-urls/query-encoding/resources/resolve-url.js` creates a detached video/track pair, sets the track URL and showing mode, waits for load, and reads the first cue text. Request URL parsing must use the owning Document's output encoding; WebVTT response decoding must use UTF-8 independently.
- `html/dom/idlharness.https.html` instantiates TextTrackList, TextTrack, TextTrackCueList and TrackEvent through native media/track APIs. Native interfaces, live lists, event targets and object identity are prerequisites.
- `content-security-policy/media-src/media-src-7_3.sub.html` and `media-src-7_3_2.sub.html` directly exercise media-src track admission, default-track error delivery and securitypolicyviolation reporting. The existing native media policy path is shared by track loading.
- `selection/crashtests/move-track-from-video-after-selection-setBaseAndExtent-from-outside-video-to-before-track.html` exercises native track node movement; it adds no caption-rendering dependency. `html/semantics/grouping-content/the-ol-element/grouping-ol.html` includes a track element in its generic element inventory. The fetch aborted-response crashtest uses a `.vtt` filename as a fetch body, without invoking a track decoder.
- The current Core inventory has no direct WebVTT/track-directory test rows. Necessary-tier caption-layout suites are not silently treated as passing or as Core evidence. The broader native track algorithm remains subject to the full plan's dependency review.

## Generated production paths

- `oryn-dom/src/webvtt.rs`: bounded UTF-8/BOM/newline parsing, cue identifiers, timestamps, settings, malformed-block recovery, missing blank-line recovery, NOTE blocks and pre-cue STYLE/REGION retention.
- `oryn-js/src/v8_bindings/text_tracks.rs`: native track/cue/region records, TrackEvent, live track/cue lists, DOM order and cue-time order, native cue fragments, add/remove, mode and active-cue events. Cross-realm cue references use shared native records and constructor-realm wrappers, with separate track owner identity; binding-local numeric IDs are not reinterpreted across realms.
- Detached media children participate in resource selection. URL encoding comes from the track Document; credentials/CORS come from the media element. Responses publish only after the terminal body acknowledgement. Source changes, disabled loading, stop, retirement and stale generations cancel native operations and suppress obsolete cue publication.
- Native parent track admission uses media-src, the track Fetch Metadata destination, CORS/same-origin rules and active mixed-content handling. Timing identifies the initiating resource as track.
- File, pending aggregate and retained cue capacity are bounded. Native notifications retain their event through suspension and allocation/dispatch retry.
- IDL and binding hooks are generated as source edits. The combined IDL generator/build step remains deferred.

## Regression sources, all unrun

Parser signature/UTF-8/settings/malformed and missing-separator cases; detached track legacy query encoding with UTF-8 body; terminal acknowledgement before load; suspended publication; source replacement and disabled-mode cancellation; cue fragment safety and identity; live track/cue reordering; cross-realm cue movement without ID collision; parent media policy/CORS/mixed-content negative matrix.

## Remaining qualification obligations

Regenerate the combined IDL/schema artifacts only after the global generation barrier, complete independent source review, then build and run native regressions plus the pinned Core/family qualification. No passing outcome is inferred from source generation.

A fresh literal scan of all unique Core source paths found no `::cue`, VTTRegion or caption-layout assertion. The known query-encoding helper loads cue text; direct CSP fixtures test admission/events; the HTML IDL harness constructs track/list objects. `test-harness/workflows` has no media text-track/caption dependency. STYLE/REGION syntax is retained by the parser, but caption painting/region layout is not claimed implemented or qualified by this bounded Core mapping. Any newly discovered transitive caption caller must reopen its implementation row.

## Timeline continuation (source generated, unrun)

Media playback now exposes one native time/session/seek-serial sample to text tracks. Cue transitions distinguish continuous playback from seeks/resource resets, queue enter/exit for cues crossed between samples, sort boundary events, and avoid pause-on-exit during seek discontinuities. Source or media-parent replacement clears the previous timeline sample. Added pure transition regressions for skipped cues, seek, and disabled tracks. This closes the identified missing continuous-playback transition algorithm at source level; subsequent adoption and lifetime continuation is recorded below.

## Adoption continuation (source generated, unrun)

The retained native node-migration transaction now cancels original track requests before ownership changes and publishes track/list/cue ownership alongside other native node maps. Existing platform wrappers remain in their original realm and forward behavior to the new native owner; cue and TrackEvent delivery preserves canonical event targets/listeners. Native track roots, DOM mappings and live list references are remapped without a duplicate request completion. Added a two-document adoption regression for unchanged media track/list/cue wrapper identities. Same-binding inert ownership and event/lifetime continuation is recorded below.


## Final source reconciliation continuation

- Inert Documents expose native track/list objects without starting loads or playback cue events. Detached nodes owned by the active Document still load when required. Discovery ignores unreachable detached native remnants, so pruning does not immediately resurrect them.
- HTML and WebVTT IDL now match track kind/mode enums, in-band dispatch metadata, HTMLTrackElement construction, TrackEvent's audio/video/text union, region identity/uint32 lines, and cue unrestricted end time. Native enum setters ignore invalid enumeration values; numeric failures retain proper finite/range distinctions. Regression sources cover inert no-load behavior and numeric contracts.
- `av_tracks.rs` projects native URL decoder and MediaStream metadata into live AudioTrack/VideoTrack lists. Selection and enabled state control actual native capture/URL output, including stopped audio output and removed video pixels. Lists retain native media roots and preserve wrapper identity during adoption. Generated tests cover admitted decoder metadata, event track identity, muted/deselected output and source reset.
- Native media graph edges connect DOM owners, lists, tracks, cues and regions through V8 tracing. Queued/in-progress notifications contribute explicit activity roots. Detached quiescent track/list records are pruned, and collected unattached cue records release native storage. Events drain before another large timeline batch; track/list change notifications coalesce. Per-file, total retained cue and pending-body quotas remain bounded.
- Repeated adoption updates the original wrapper's forwarding entry directly to the current owner, avoiding dependence on a retired intermediate binding. All source regressions remain unrun, including callback survival through GC, collectible detached cycles and cross-document identity.

### Final ownership source review

- Foreign `VTTCue.region` associations now retain the originating native region record through explicit reference accounting. Reassignment balances the old/new owner count; cue destruction releases it. Added a two-realm regression for repeated assignment, owner reclamation while referenced, and release after the last cue reference.
- Invalid media-owner cleanup no longer calls the element initialization/reset path or clears the adopted node's new-owner DOM output. It cancels old resource selection, stops native outputs, transfers failed asset releases to existing ACK custody, and retains pending play resolvers until AbortError publication succeeds before removing playback. Added adopted-owner cleanup with failed native release and idempotent retry coverage.
- These additions are generated source only. No build, test, generator, or qualification run has occurred. Broad L-01 and caption rendering qualification remain open under the dependency boundaries above.
