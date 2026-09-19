# Native test02 stack overflow source disposition

The crash report `/Users/rohit/Library/Logs/DiagnosticReports/oryn_native-60e796c400d81b43-2026-09-18-125506.ips` identifies thread 630530, matching the test log. Its native call chain is page actor creation → related actor run → dispatch → navigation transaction poll/step/launch → HTTP navigation construction → SchemefulSite → PSL lookup. There is no repeated native recursion chain.

Read-only disassembly of the existing test02 executable shows these simultaneously live unoptimized stack reservations (including saved-register area):

| Function | Stack bytes (hex) |
| --- | --- |
| page actor spawn closure | 0x50d80 |
| related_actor::run | 0x9ffb0 |
| page_actor_turn::dispatch | 0x12090 |
| poll_navigation_transaction | 0x1b990 |
| step_navigation_transaction | 0x99a80 |
| launch_navigation_transaction | 0x23970 |
| start_navigation_fetch | 0x1b910 |
| PSL generated lookup | 0x9500 |

These frames exceed the ordinary thread stack cumulatively; the final PSL lookup's stack probe is the failure point, not evidence of recursive PSL processing. The report unwinds to its caller because the lookup fails during its prologue.

## Source repair

- `page_actor_turn::Root.state` now owns `Box<PageState>`. Moving roots through scheduler queues retains a stable native state allocation and preserves Root::shutdown/Drop ordering.
- `prerender_commit::Commit.candidate` now owns `Option<Box<CandidateDocument>>`. Pending phase/recovery movement no longer embeds a full retained executable. The candidate is moved out exactly once at the existing publication boundary; cancellation still drops its original reservation and document.
- Regression sources cover root queue growth/removal preserving the state pointer, root drop closing its receiver, and phase/actor envelopes remaining smaller than their document payloads.

No thread stack limit changed. No builds or tests were executed for this repair. A refreshed test binary and full native run must confirm frame reduction and runtime recovery; old disassembly establishes the cause, not validation of new code.

## Test03 JS central-stack fatal

The report `oryn_js-e15dd60812ea9340-2026-09-18-130601.ips` identifies the pointer down/up dispatch regression. Its call chain is fixture binding install → custom-state iterator setup → V8 Function::Call → V8 StackOverflow → IsOnCentralStack fatal. There is no isolate transfer or invalid scope lifetime in the fixture. Existing-binary disassembly reserves 0xfcc60 bytes in the test function before Isolate creation and 0x25340 in binding installation.

`native_pointer/dispatch_receipt.rs` now retains event, capture, focus, prompt, top-layer, range, selection, drag admission, drag and touch continuations in individually owned boxes. Range likewise retains its Event in a box. Constructors still admit the same operations; poll, take, cancellation and Drop sequences are unchanged. The original callback, capture, focus, range, cancellation and reset regression sources remain intact. A layout regression guards against re-embedding the combined continuation trees. No arbitrary V8 stack limit or test thread stack override was added. The next execution must confirm the fatal is resolved; no new binary was run during this repair.
