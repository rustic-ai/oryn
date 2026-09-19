# First compiler repair review

Source: `artifacts/completion-validation-2026-09-18/native-build-04.log`, 13 errors. These repairs preserve the generated implementation; they do not establish a passing build.

| Source | Reported defect | Reviewed repair |
| --- | --- | --- |
| `vendor/usvg/src/text/layout.rs` | ResolvedFont derived Copy despite owned font settings | Remove Copy; existing consumers borrow or clone Arc handles. |
| `crates/oryn-dom/Cargo.toml` | Missing oryn_common dependency for native paint image resources | Add workspace path dependency; common does not depend on DOM. |
| `crates/oryn-dom/src/lib.rs` | Serde field attribute on a struct without Serde derives | Remove stray attribute; do not invent serialization of the native node. |
| `vendor/html5ever/src/lib.rs` | Three callers cannot find parse_fragment_for_element | Re-export existing driver implementation; preserve existing context, form and streaming semantics. |
| `crates/oryn-dom/src/image_intrinsics.rs` | Option callback expects &String rather than &str | Closure permits normal deref coercion without changing length parsing. |
| `crates/oryn-dom/src/document_scope.rs` | Snapshot omitted AX and form-associated metadata fields | Filter and copy metadata for retained nodes; regression checks owned detached nodes and excludes foreign nodes. |
| `crates/oryn-dom/src/lib.rs` | Rebuilt empty arena omitted metadata fields | Initialize empty AX maps and form-associated set, consistent with empty node slots. |
| `crates/oryn-dom/src/lib.rs` | Ordinary insertion/replacement omitted state_preserving_move | Set false; ordinary operations retain their intended move semantics. |
| `crates/oryn-dom/src/node_migration.rs` | Immutable slot iteration overlaps mutable arena borrow | Snapshot slot IDs before mutating corresponding records. |

The failed run and original declaration are retained. A new declaration must record these reviewed source changes and any completed Python repairs before subsequent execution. Full conformance and qualification remain outstanding.
