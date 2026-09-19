# Clipboard source generation — 2026-09-16

Source generated only. No builds, tests, formatters, generators, or browser execution performed. This is not conformance evidence.

- Shared `oryn-js/src/clipboard.rs` wire model retains UTF-16 text, ordered MIME entries and actual file bytes; validates entry and byte limits.
- `v8_bindings/clipboard_actions.rs` implements Rust-only single-use target/realm/generation/epoch grants; trusted copy/cut/paste, cancellable beforeinput, native control/contenteditable edits, private InputEvent payloads and detached event DataTransfer associations. Author script globals cannot mint authorization.
- Copy uses native range boundaries and exact source-node computed formatting, bounded light-tree cloning and UTF-16 slices. Partial text selections inherit formatting from their actual parent; unrelated siblings cannot shift a clone-to-source list index.
- Paste preserves native text/control selection handling and sanitizes active HTML nodes/event attributes before native rich insertion. Password default selection copy is excluded.
- Context clipboard storage is typed and context-local. Parent policy mediates native IPC read/write; author context-service JSON rejects the private clipboard variants. No ambient OS clipboard access was introduced.
- Host root/frame install sites attach the parent backend. Legacy script clipboard shortcut simulation was removed; typed native input routing is owned by the root agent.
- Regression sources cover native grants, retirement, callback cancellation, data lifetime, byte/surrogate fidelity, partial-selection formatting ownership and sanitization.

## Remaining integration and scope

Typed native WebDriver input routing remains root-owned. Async navigator.clipboard/ClipboardItem APIs and execCommand clipboard permission semantics are not implemented by this batch. Rich copy does not yet resolve relative URLs against the source document or reconstruct full rendered plain-text block separators. Sanitization needs broader namespace/CSS/URL-policy coverage; no claim of complete HTML clipboard fidelity or conformance. All source requires the final whole-plan generation/build/test/fix/conformance phase.

## Async and command follow-up source

The async APIs and execCommand gaps above now have implementation source:

- `v8_bindings/async_clipboard.rs` installs native Clipboard/EventTarget and ClipboardItem objects, promise-returning read/write/readText/writeText, frozen types, presentation style, getType and mandatory-format support queries. ClipboardItem accepts promised text/Blob representations; writes use private native continuation state and recheck epoch, live realm and parent policy after resolution. Generic iterable sequence conversion uses native IteratorClose handling.
- Parent-only ClipboardAuthorizeScript IPC checks the actual active BrowserContext page, trustworthy origins collected from native private Window ancestry, and explicit Clipboard policy for the caller origin. The author JSON bridge rejects this request. Native document focus uses the DOM focus owner and embedding ancestry; unfocused subframes fail closed.
- Document clipboard execCommand/query integration invokes the same parent script policy before a one-use native action. It never infers activation from synthetic events or script globals.
- Native navigator survives framework bootstrap initialization. Secure-origin exposure is checked when native installation runs; operation-time ancestry is checked again.
- Async HTML read/write uses the native sanitizer. PNG payloads are decoded with bounded dimensions/allocation and re-encoded without arbitrary metadata. Rich copy resolves ordinary URL attributes against the committed document base URL.
- Source regressions cover denied/allowed text access, delayed promised-write policy revocation, ClipboardItem native Blob conversion and command policy denial. Earlier copy/default regressions remain.

Remaining precise gaps: late frame parent-link establishment can affect secure-context interface exposure until native installation is repeated; operation-time checks still deny insecure ancestry. Cross-process native focus propagation must be verified in the whole-plan phase. The context clipboard currently supports one multi-representation item, so writes of multiple items reject. `read` options for explicitly unsanitized formats are not implemented. HTML sanitizer still needs full CSS-tokenization/namespace/URL-policy completeness, relative srcset/CSS URL resolution and rendered block-separator text reconstruction. Error DOMException prototype fidelity and cross-realm promise/receiver behavior need integration review. Generated IDL tables remain deferred. No qualification or passing result is claimed.

Further source review closed generic iterable write conversion, native Clipboard/EventTarget prototype linkage, ClipboardItem prototype accessors and presentationStyle, bounded PNG decoding/re-encoding, and native DOMException creation for async rejection. These changes remain unexecuted. Cross-realm receivers, abrupt JS conversion completion, insecure late-bound frame exposure, unsanitized read options, and complete CSS/source-URL formatting still require whole-plan integration review; prior unresolved statements are superseded only where explicitly closed here.
