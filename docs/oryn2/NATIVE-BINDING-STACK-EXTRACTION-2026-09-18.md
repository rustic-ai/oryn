# Native binding stack repair

## Observed failure and evidence

Test14's native library aborts with `Check failed: isolate_->IsOnCentralStack()` during V8 string allocation / garbage collection. Its C backtrace retains `call_operation`, DOM post-connection publication, nested frame checkpoints, `create_native_document_context`, and `install_generated_bindings_with_root_for_global`.

Read-only ARM64 disassembly of the exact test binary `target/debug/deps/oryn_native-20366294d76e8c80` showed these complete fixed frames, including saved registers:

| Function | Stack bytes |
| --- | ---: |
| `call_operation` | 242,768 |
| `install_generated_bindings_with_root_for_global` | 153,072 |
| `generated_operation_impl` | 17,200 |
| `create_native_document_context` | 8,704 |
| `ensure_native_initial_frame` | 1,344 |

The first two functions retain 395,840 bytes before additional native and JavaScript frames. Nested DOM callback operations multiply that cost. This evidence identifies excessive debug native frames; it does not establish a post-repair pass.

## Source repair and equivalence

- `call_operation` retains its complete prefix, interface normalization, match patterns, guards, arm order, and default error. Each of its 116 nondefault expressions moves unchanged to a separate `#[inline(never)]` handler in `v8_bindings/node_operation_dispatch.rs`. Unlike a single large interface handler, individual operation frames cannot accumulate unrelated operation temporaries. All six original arguments pass unchanged, including pattern-bound `interface` on guarded arms.
- Binding installation moves into three non-inlined phases in `v8_bindings/binding_installation.rs`: create/register state, install interfaces, publish realm objects. Phase ordering and original statements remain unchanged. The coordinator owns the returned `Rc` and constructor map, so state construction and interface temporaries are released before the next phase.
- The JSON companion records every moved arm pattern and SHA-256 of its original expression, plus each original installation phase. Extraction checked exact expression/phase inclusion before publishing source. No routing denominator, exception behavior, policy, lifecycle ordering, V8 assertion, stack flag, or native thread configuration changed.
- Neighboring DOM/input, navigation and parser-owned central changes remain in their original regions. The platform-operation dispatcher is outside this extraction.

## Regression source and pending execution

`binding_installation::native_stack_tests::nested_dom_callbacks_install_realms_and_collect_on_the_central_stack` creates seven real V8 binding realms through nested custom-element connected callbacks and `appendChild`. Each child forces V8 GC while ancestor callbacks remain active. It asserts all child installations and insertions complete, with default V8 configuration.

All existing operation/lifecycle tests remain enabled and unchanged. No build/test was executed for this repair in this lane. The next coordinated build must verify helper signatures and measure the new frames; the regression and native library rerun must verify behavior. The new regression exercises binding/context allocation, while the existing native frame integration tests exercise actual BrowserContext frame ownership.
