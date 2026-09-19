# Clipboard markup source and drag-image integration contract

Source generation only, 2026-09-16. No builds, tests, formatters, generators, browsers, or conformance runs were executed.

## Generated markup implementation

`crates/oryn-js/src/v8_bindings/clipboard_markup.rs` uses pinned `cssparser` 0.37 token parsing and native `url::Url` parsing. It replaces the raw-substring sanitizer in `clipboard_actions.rs`; async clipboard and native drag now use the state-aware entry with the committed native document base URL (including inherited about:blank/srcdoc base).

- CSS escapes are decoded before URL/function/property decisions. Unsafe declarations are removed individually; valid sibling declarations and numeric source precision remain. Nested URL functions, image-set quoted URLs, SVG presentation URLs, and source formatting URLs are rewritten.
- URL parsing handles scheme normalization and embedded tab/newline processing. HTTP/HTTPS/FTP, link-only mailto/tel/sms, native ownership-checked blob references, and raster image data resources have explicit handling. Executable schemes, HTML data URLs, and SVG data resources are not retained.
- HTML srcset parsing retains commas inside data URLs, validates candidate width/density/height descriptors, and resolves each surviving URL independently.
- SVG xlink href and XML base use native expanded namespace identity. The sanitizer snapshots inherited XML bases before stripping base attributes, preventing ancestor removal from changing child resolution. Source selection copying resolves URLs against the actual original source node before clone ownership is lost.
- Separately owned template content fragments are traversed and sanitized. Script, active embedding, SVG animation, event attributes, invalid namespace bindings and author base elements are removed without relying on public DOM properties.
- Markup node/byte, CSS nesting/output and expanded attribute/candidate output limits remain explicit failure/omission boundaries.

Regression sources cover escaped CSS executable URLs/properties/functions, image-set and fragments, data srcset commas, duplicate/invalid descriptors, non-ASCII descriptor rejection, xlink/SVG/XML base, inert template scripts, and author base spoofing. Existing drag/clipboard regression source remains; nothing has been run.

Remaining markup limits: this is native sanitization, not a certification against every clipboard/WPT case. CSS supported-value validation remains the native style engine's job. SVG data images are deliberately excluded; external resource loads still require the native broker and blob ownership policy. Full rendered formatting/ligature fidelity and oversized payload behavior require later qualification.

## Native drag-image findings

Current `data_transfer::Store.drag_image` holds only `(NodeId, x, y)`. `setDragImage` has no raster capture. `BindingLayout` supplies geometry/paint projections but no complete images/fonts. The actual native paint capture exists at `PageState::paint_snapshot_plan_with_layout` and resource completion at `complete_paint_snapshot`; `oryn_render::render_png` accepts the resulting `PaintSnapshot`. `LiveLayoutProvider` is synchronous and holds the shared native DOM. A PageCommand/worker round trip inside a V8 `setDragImage` callback would deadlock the active page operation.

## Proposed concrete source contract (not implemented)

Add a synchronous native-only provider alongside the live layout provider:

```rust
struct DragImageRequest {
    document: NodeId,
    realm: RealmId,
    generation: u64,
    stream_epoch: u64,
    source: DragImageSource, // Element(NodeId) or frozen selected range/control span
    hotspot_css: (i32, i32),
    max_pixels: usize,
    max_bytes: usize,
}
struct DragImageSnapshot {
    width: u32,
    height: u32,
    png: Arc<[u8]>,
    source_css_bounds: CssRect,
    device_scale: f64,
    hotspot_device: (f64, f64),
}
trait BindingDragImageProvider {
    fn capture(&self, request: &DragImageRequest)
        -> Result<DragImageSnapshot, DragImageError>;
}
```

Required implementation/integration points:

1. Factor synchronous native paint snapshot construction from runtime capture into a helper consumable by `LiveLayoutProvider`. Supply an immutable snapshot of already-loaded image/font/SVG resources. The callback must not fetch network resources, enter JavaScript, request the page actor, or use an author screenshot/canvas helper.
2. For an image element, capture its real decoded native image using the appropriate intrinsic image semantics. For an arbitrary element, render only that element's native paint subtree with resolved styles, text glyphs, transforms and clipping; do not use an enclosing viewport crop that includes unrelated siblings. Default selected-content feedback needs the actual selected range/control geometry mask.
3. Generate the immutable snapshot during native `setDragImage`, before returning to author code. Store bytes and hotspot in the drag data store instead of a mutable NodeId reference. Later mutation/removal must not change the stored image. A subsequent allowed setDragImage replaces the snapshot; protected/readonly calls remain ineffective.
4. Enforce exact document/realm/generation/stream epoch before and after capture; validate finite bounds/scale, checked dimension multiplication, raster and encoded byte caps, and source ownership. Initial proposed caps: 4096 per dimension, 16 million pixels and 64 MiB decoded bytes; these are explicit bounded-runtime policy, not conformance claims.
5. Queue native feedback updates during V8 dispatch and drain them at the existing typed native action boundary. Add a process-safe `DragFeedbackUpdate { page, realm, generation, epoch, drag_session_id, snapshot_or_clear }` transport. Parent/platform consumer accepts only the live matching drag session. Do not make pixels or authorization available through author globals.
6. Clear queued/stored feedback on canceled dragstart, drop/cancel, source navigation/retirement and context teardown. Worker loss also clears parent feedback; late updates for old sessions are ignored.
7. Generate source tests for call-time snapshot immutability, CSS/transformed subtree capture, intrinsic images, selected-range feedback, no sibling paint leakage, hotspot scaling, caps, stale-node/document rejection, protected-mode no-op, worker/session retirement and decoded-resource unavailability. Only run after the whole-plan source-generation gate.

## Native drag-image generation update

Generated the real synchronous `LiveLayoutProvider` capture path, immutable call-time `setDragImage` snapshot, selected-text native quad mask, intrinsic raster/SVG image capture, transformed subtree rasterization and typed private action-response feedback. Renderer core now lives in the independent `oryn-render-core` crate; the `oryn-render` facade preserves native screenshot/PDF wrappers and CLI without a native dependency cycle. Native loaded raster/canvas resources and original successfully decoded SVG bytes are copied under bounded native ownership; capture performs no network or actor request. The frame-paint agent consumes the same lifetime-checked resource accessor.

Feedback is parent-owned, bounded and atomically validated, with root page generation plus realm generation/epoch/sequence ordering. Drop/cancel, stream retirement, page navigation/close/crash clear feedback. Scoped action responses carry the root page generation. Regression sources cover intrinsic SVG pixels, missing SVG dependencies, selected offscreen crop alpha, transformed bounds, raster limits, invalid PNG metadata, atomic batch rejection, stale feedback and epoch renewal.

Native capture inventory installation now hands completed root and selected-frame paint snapshots back to their owning provider. Root asynchronous completions revalidate document generation, stream epoch and native mutation cursor before installation; stale completion never replaces inventory. Provider uses loaded original-URL background resources and only reuses nested-frame/foreignObject geometry after native layout/style/projection agreement. Checkpoint feedback publication now uses the authenticated context-service route between typed action ticks, outside the V8 capture callback. Typed pointer actions now prepare a native paint inventory before dispatch and refresh it when the root native cursor/epoch changes between polls. Completion I/O runs outside callbacks with a total request timeout and broker cancellation flag; selected/root identity is checked again before installation and input. Release Actions bypasses resource preparation. SVG cached resources retain final response URL and requested URL alias for correct relative dependency resolution.

Native selected-frame planning now receives the action cancellation controller and absolute deadline before start/read and on completion; root completion uses broker cancellation. Inventory invalidation compares exact native DOM revision, viewport, scroll offsets and custom states for every live realm, with explicit scrollbar invalidation. Canvas bytes are always replaced from the current native capture even without a DOM mutation. No OIL drag command or held-pointer move sequence exists in the current native OIL/parser surface; typed WebDriver pointer sequences are the production drag-capable scheduler. Parent owns removal of legacy author-callable testdriver/native drag globals.

Concrete remaining generation dependencies: a change made inside dragstart can require resources not present at the pre-action snapshot; capture cannot fetch them or reuse mismatched cached geometry. Cross-frame drag data/session transport remains separate from nested frame image composition. Frame context transport requests are individually bounded but cannot observe the action cancellation token until the synchronous response returns. Whole-plan execution must validate source integration and raster fidelity. Default capture failure leaves the drag operational without a raster; custom capture failure reports an error. Cross-frame drag data/session transport remains distinct from frame image composition. These gaps must be closed before claiming full drag-image behavior.

No build, dependency resolution, formatter, generator or test was run. Cargo.lock local package edges were updated by source editing; external dependency resolution remains deferred with the whole-plan gate. All source changes and new regression sources remain unverified.
