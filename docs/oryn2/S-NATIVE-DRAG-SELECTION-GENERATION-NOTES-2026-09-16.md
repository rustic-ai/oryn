# Native selected-content drag generation — 2026-09-16

Source generation only. No build, test, formatter, generator, deployment, browser run, or conformance run was performed for this work. These notes do not certify implementation or WPT success.

## Sources changed

- `crates/oryn-js/src/v8_bindings/native_drag.rs`: selected source capture, projected selected-text hit testing, rich default drop, original-source move defaults, regression source.
- `crates/oryn-js/src/v8_bindings/clipboard_actions.rs`: narrow rich-selection/data export; native editing events resolve their explicit native target's editing host.
- `crates/oryn-js/src/v8_bindings/native_keyboard.rs`: compound control undo identities, mixed document/control undo markers, memory accounting, selection finalization.
- `crates/oryn-js/src/v8_bindings.rs`: compound editor snapshots, native control state in snapshots, rollback/undo/redo, companion retention and bounds.

Root separately changed pointer begin to pass the captured pointer-down position. Existing native pointer down preserves selection while waiting for a drag threshold. Shared `native_drag::dispatch_input` remains the typed entry used by native pointer dispatch.

## Generated behavior

- Snapshot exact UTF-16 selected control/range content before `dragstart`; password source values are excluded. Original source range/value and native document generation/stream epoch stay independent of subsequent public Selection changes.
- Source start hit uses the selected portion of physical native text clusters, inverse projective mapping, writing-mode and bidi orientation. It rejects points behind the projection plane and does not use a rotated rectangle's enclosing screen box.
- DOM range payload reuses source-owned native formatting and absolute URL serialization from clipboard selection copying. `dragstart` may replace DataTransfer payload and effectAllowed.
- Readonly/noneditable selected sources can copy. Move requires a currently editable native source and destination; text/control ownership is checked after author callbacks.
- Drop inserts sanitized HTML into rich editors and plain text into plaintext editors/controls. Native InputEvent dataTransfer uses private native payload state and per-event detached lifetime, including insertFromDrop/deleteByDrag.
- Canceled drop handlers skip native defaults; canceled beforeinput preserves the original source. Mutation-journal truncation or intervening relevant source changes prevent source deletion, even when script restores equal bytes.
- Single-control, cross-control, same-editor, cross-editor and mixed text-control/contenteditable moves use original-source deletion, native destination carets and explicit rollback. No deletion reads a handler-modified current Selection. A post-beforeinput native caret-owner check rejects a handler moving the destination node into an uncaptured third editor.
- Editor moves use companion snapshots in one document undo unit. Cross-control moves share one identity across both control histories. Mixed moves bridge the document undo unit to exact control-history markers; undo from either owner requires all owners' expected live identity/value/history position.
- Control snapshots preserve dirty/default value ownership, pending change state and selection. History accounting includes companion snapshots and retained group payloads; oversized compound edits are rejected before commit or rolled back.

## Regression source generated

Exact UTF-16/lone-surrogate moves; payload overrides; inside-selection/no-op rejection; source replacement followed by equal-value restoration; unrelated-control changes; rich sanitization; two-editor undo/redo and injected insertion failure rollback; two-control shared undo and stale-owner rejection; both mixed directions; canceled insert beforeinput; mixed undo from either focus; script value replacement invalidating one owner without undoing the other half. A third-editor reparenting negative regression is also generated. The prior canceled-drop/event-store lifetime test remains intact.

## Remaining source/integration work

- Cross-frame/process drag session routing and atomic source/target editing cannot reuse this realm's NodeIds or undo identities. Parent/host coordination is still needed.
- DataTransfer drag-image element references still need actual native raster snapshot/placement and compositor feedback. This batch does not manufacture a drag image or claim it exists.
- Clipboard's inherited rich-HTML sanitizer limitations remain: complete CSS/namespace URL handling and all formatting fidelity need further source work and qualification.
- Explicit DOM editing undo mutation/reentrancy behavior beyond the generated companion checks, selection affinity around complex ligatures, and platform-specific drag modifier behavior still require pinned-suite validation after the global source-generation gate.
- Byte and snapshot caps are deliberate failure boundaries. They are not evidence that larger moves conform or that memory/performance acceptance passed.
