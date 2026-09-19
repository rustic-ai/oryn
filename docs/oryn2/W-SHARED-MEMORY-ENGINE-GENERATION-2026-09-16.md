# Shared-memory engine integration: source audit and generation

Status: incomplete source generation. No engine build or tests run.

## Authoritative engine source

Pinned rusty_v8: `150.4.0`; V8 revision: `ac1e23989121713ca642f6650b34deff7b686896`.
Inspected the matching Cargo source and `vendor/v8-api` public headers.

- `src/objects/backing-store.h`, `BackingStore::byte_length`: reads the backing object's process-local atomic length.
- `src/objects/backing-store.cc`, `TryAllocateAndPartiallyCommitMemory`: reserves/commits native memory, including Wasm guard regions.
- `GrowInPlace` and `GrowWasmMemoryInPlace`: update local page permissions and the local length atomic. Sharing a data descriptor does not share these metadata operations.
- `src/execution/futex-emulation.cc`, `WaitSync`, `WaitAsync`, and `Wake`: use process-local `GetWaitList()` and virtual-address keys. Different process mappings cannot notify each other through this table.
- Public `v8-isolate.h` exposes `SetAllowAtomicsWait`, but no replacement wait-queue delegate. Public external backing construction supplies fixed length, not a growable external backing contract.

The existing sandbox-safe shared mapping bridge therefore establishes only fixed backing storage. It does not prove growable SAB, shared Wasm memory, or cross-process waiter correctness.

## Required native/engine contract

1. Allocation returns an attested backing capability, shared metadata page, sandbox data mapping, logical initial/max lengths, reserved capacity, and guard-region extent. Capacity and guard extent are distinct from logical maximum length.
2. Length reads must acquire the shared logical length wherever V8 exposes growable backing length. Updating only the wrapper's cached length is insufficient.
3. GSAB growth performs checked monotonic absolute growth. Wasm growth performs atomic delta-page growth and returns the distinct prior page count; an expected-length CAS retry is an acceptable equivalent.
4. Growth zeroes/publishes bytes before publishing length. Each mapped process must have usable capacity before observing the new length, and Wasm instance bounds/trap assumptions must refresh at native engine checkpoints.
5. Wait identity is backing capability plus byte offset, never a process address. Width/alignment/bounds and exact native cluster membership are checked before access.
6. Wait admission atomically compares the mapped word while holding the coordinator queue lock, checks zero timeout after comparison, then registers. This prevents lost notifications between comparison and queue insertion.
7. Notify, timeout, cancellation and retirement choose exactly one terminal result. Async completion returns to the originating live native environment. Synchronous waits use bounded interrupt polling so isolate termination cannot be trapped behind an unbounded RPC.
8. Backing/waiter ownership survives native transport delivery and is reclaimed on failed delivery, realm retirement, process exit, and final V8 backing release. Empty/failed admission must not enable the corresponding author API.

The worker agent owns coordinator and wire generation. Root owns the V8 hook integration. Public-header/archive/profile synchronization and qualification remain mandatory after all source generation is complete.

## Regression source generated

`crates/oryn-native/tests/fixtures/shared-memory/engine-lifecycle.https.html`, its header sidecar, and `worker.js` cover:

- Cross-mapping synchronous wait/notify with registration-race handling and exact removal.
- Remote notify settling an asynchronous waiter.
- Concurrent GSAB growth, length-tracking views, preservation and zeroing.
- Concurrent shared Wasm delta growth returning distinct old sizes and refreshing visible bounds.

These fixtures require testharness serving and COOP/COEP headers. Qualification integration and native worker-placement attestation are still open: passing this fixture in colocated workers would not prove cross-process behavior. The engine hooks themselves remain to be generated; this audit is not a completion claim.


## Generation update: GSAB growth bridge

Generated the pinned `shared-memory-growth.patch` and exact source/hash manifest,
public-header synchronization, builder patch application, and profile provenance.
The external backing keeps its custom mapping deleter while a native delegate
reads the coordinator's shared u64 length with acquire ordering and routes
absolute growth through the principal-bound native callback. Explicit growability
is independent of whether maximum and initial lengths are equal. The FFI consumes
its callback lease even on admission failure; backing and delegate share mapping
ownership. Fixed buffers retain the release lease as well.

Engine regression source in `patches/v8/tests/test-oryn-shared-memory-growth.cc`
covers length-tracking/fixed views, remote metadata publication, shrink/failure
behavior, zeroed capacity, and custom-deleter lifetime. The runtime-profile builder installs this source into the pinned cctest target
using exact base, resulting build-file, and test-source hashes. No build or test
has run; executing cctest remains part of deferred qualification.
Wasm allocation/guard/bounds, engine futex dispatch, allocation interception,
and end-to-end factory/transport integration remain open; this is not a completed
shared-memory implementation or qualification claim.


## Generation update: native wait delegation

Generated `shared-memory-wait.patch` on the exact growth-patched inputs, with
public API/header hashes and builder/profile provenance. Sync waits delegate
compare/enqueue and bounded polling to native ownership, service V8 interrupts
between polls, and destroy the waiter on every terminal/exception path. JS async
wait dispatch calls the native environment delegate for the genuine result and
promise. JS buffer notification dispatches by backing identity; Wasm sync entry
points also use the backing delegate. Raw-address Wasm notification remains open.
The native adapter, async lifecycle, allocation factory and policy integration
remain under generation. Regression source covers expected signed word bits,
byte offsets, immediate/timeout dispatch, notify routing and termination cleanup;
it has not been built or executed.


Raw-address Wasm notification source is now generated. JIT calls keep their
existing ABI; native backing registration resolves the address to the correct
wait delegate and byte offset. Registry entries are weak, non-overlapping,
removed before mapping destruction, and never hold their mutex across Notify.
Growth must be installed before wait registration so the logical maximum is
final. Regression source covers offset dispatch, exclusive upper bounds,
retirement/address reuse safety, and retained in-flight delegate ownership.
Wasm backing allocation/growth/guard/bounds integration still remains open.


### Remaining Wasm bounds source findings

Pinned `wasm-module.cc::UpdateComputedInformation` selects trap-handler bounds
checks independently of backing allocation. A shared descriptor mapped read/write
through its maximum cannot be passed to that path: bytes beyond current length
would become accessible before Wasm's bounds contract permits them. Merely
attaching the shared length delegate is therefore insufficient.

Both Turboshaft's `LoadMemSize`/`MemSize` and the baseline memory checks currently
read instance-local size fields. Remote shared growth must update these readers
from native shared metadata (including all memory indices and `memory.size`), or
perform a complete permission/bounds publication handshake. The current ordinary
`BroadcastSharedWasmMemoryGrow` only interrupts isolates registered in this
process. These are required implementation paths, not optional qualification
work, and remain open before shared Wasm exposure can be enabled.


### Async enqueue/notify race integration

Source review found the adapter polled immediately after enqueue and classified
waitAsync from that later result. A notification winning that interval could
incorrectly produce `{async:false,value:"ok"}`. Begin now returns its original
compare/enqueue state through the native callback ABI, and WaitAsync uses that
immutable classification. Enqueued operations always return a genuine promise;
terminal completion is delivered by the owning environment poll. Added an
engine/adapter regression with a coordinator that reports Pending on admission
and Ok on the first poll, verifying promise identity, deferred settlement, and
single acknowledgement without an extra cancellation. No execution performed.


### Stable shared-length address contract

Generated a native-size atomic length-address contract on the growth delegate and
internal BackingStore. Native mapped buffers expose their retained metadata cell;
ordinary V8 shared buffers expose their backing-owned length atomic. The address
is stable across growth, aligned, and tied to backing ownership. The 64-bit native
mapping bridge explicitly requires size_t to match the coordinator's u64 cell.
The pinned growth/wait patch chain and public header hashes were synchronized.
Regression source verifies direct acquire reads observe remote length publication
without an instance refresh. Compiler bounds/size readers, native Wasm allocation,
and atomic delta growth still need to consume this contract; none is qualified.


### Shared Wasm size-reader source

Generated `shared-wasm-size.patch`, pinned after the growth/wait patches. Trusted
instance metadata appends one stable size-cell address per memory without
changing existing base/size entry offsets. Liftoff bounds checks and memory.size,
Turboshaft memory-size reads, and native C++ size accessors load shared length
atomically. Shared memories use explicit bounds checks, so logical shared size
rather than process-local page permissions defines bounds. Non-shared memories
retain their existing trap-handler path. The qualified builder records and
verifies the full patch chain; it has not been executed.

Native Wasm backing creation/import, delta growth, full interpreter cached-bound
integration, compiler regression sources, and qualification remain open. In
particular, this patch alone does not make shared Wasm available to author code.


Added engine regression source constructing a shared-memory Wasm module with
memory.size and load8_u exports. It checks current-end bounds traps, growth's old
page return, newly accessible zeroed bytes, data visibility and repeated compiled
reads. Qualification must execute it under both Liftoff and --no-liftoff; this is
not remote-process coverage or proof of native allocation integration.


### Interpreter bounds integration

Pinned interpreter source inspection confirms memory.size, scalar/bulk explicit
bounds handlers, and runtime memory-range helpers all call the updated trusted
size accessors. Architecture-specific fast load/store handlers instead assume
memory guard-page faults. The shared-Wasm patch now selects explicit DrumBrake
bounds handlers by default and rejects a DrumBrake build that disables them.
The runtime-profile builder also sets this option explicitly. Thus enabled
interpreters use the same logical shared-length reads as compiled paths, rather
than accessing fully reserved bytes past the current logical end. This generated
integration still requires interpreter-enabled build/test qualification; native
Wasm creation, imports and delta growth remain unfinished.


Added bulk-memory regression source for shared memory.copy: out-of-bounds ranges
must trap without partial destination writes; zero-length end-of-memory copies
are valid; growth makes newly in-range copies work. The source manifest records
Liftoff, Turboshaft, and enabled-DrumBrake execution modes for later qualification.
No engine build or test has run.


### Native Wasm backing and delta-growth source

Generated `shared-wasm-backing.patch` with a public native mapped-backing factory.
It validates page-sized logical extents, capacity, engine limits, and stable
metadata alignment; consumes mapping ownership on validation failure; initializes
V8's shared-Wasm isolate registration data; and stores the mapping deleter outside
that existing union. Destruction retains registration cleanup and releases the
native mapping exactly once. Native growth delegates receive delta and maximum
pages and return the coordinator's atomic previous page count, with postconditions
checked against shared metadata. Public headers, hashes, builder and profile
provenance are synchronized. The browser constructor factory and clone/admission
adapters still need to call this API.

Regression source covers two concurrent delta grows returning distinct prior
sizes, zero delta, maximum failure without length change, mapping release, and
failed-factory ownership. It has not been compiled or executed.


### Native Wasm mapping adapter source

The C++ shared mapping implementation now has a Wasm-specific entry point using
the engine's native Wasm backing factory and a delta/maximum/previous-page growth
callback. Its Rust caller consumes only an isolated native broker response,
selects address type from the attested memory64 descriptor, and forwards the
call-site maximum to parent GrowByPages. The existing ordinary SAB entry point
retains its ABI. Both paths preserve consumed-on-failure release ownership.
The returned temporary SAB is solely a handle to the Wasm backing; the real
WebAssembly.Memory constructor/import factory still needs integration.

Generated callback and adapter regression source checks prior-page propagation,
maximum-result validation, and exactly-one release when an invalid native mapping
is rejected. No build or test has run.

## Native constructor allocation dispatch — source generation

Added `shared-memory-factory.patch` after WindowProxy, growth, wait, Wasm-size,
and Wasm-backing patches. A per-isolate `SharedMemoryAllocationDelegate` returns
an independently owned shared backing. Actual V8 SharedArrayBuffer construction
retains length/max validation and observable new-target prototype lookup ordering.
Actual shared Wasm memory construction dispatches with the native page-derived
initial/maximum bounds and memory64 kind. Missing native allocations and returned
backings with inconsistent shared/Wasm/growable/length metadata fail allocation;
there is no process-local fallback while a delegate is installed. Ordinary
ArrayBuffer and non-shared Wasm allocations retain their original paths.

The public API headers, exact source manifests, build-script patch chain/profile
hash and crate header checks now include this patch. The patch application helper
recognizes the complete final chain, including the WindowProxy api.cc overlap.
Regression sources cover ordinary allocation bypass, actual SAB growth, invalid
maximum validation before native dispatch, failed allocation prototype lookup,
wrong-kind backing rejection and shared Wasm failure without local fallback.

**Still incomplete:** native callback bridge and per-environment authority lookup,
installation, successful broker-backed constructor integration, Wasm clone/import
adapters, full cross-process lifecycle coverage, and all execution qualification.
Physical backing leases must survive document retirement while any wrapper remains
reachable; new allocation/delivery authority remains tied to live documents.
No builds, tests, formatters, or project generators were run for this update.

### Native factory callback integration

Generated the native C++ allocation delegate and Rust callback bridge. Binding
installation installs an isolate-level delegate with no retained realm identity;
every allocation resolves the rooted current context's state, exact creator,
liveness, and transport before requesting parent allocation. Returned mappings
install the wait/growth ownership and transfer a shared backing owner to C++ before
the Rust callback's handle scope ends. Failure without a wait environment releases
the admitted parent reference. An unbound-context regression source verifies SAB,
GSAB, and shared Wasm fail while ordinary allocations remain available.

The W/G track is concurrently integrating isolate-wide live-mapping registration
and physical mapping retention after creator-document retirement; the factory's
mapping registration must use that same path. Successful broker-backed constructor
coverage, shared Wasm clone/import, visibility policy, all builds and qualification
remain open. This is source generation only.

### Shared Wasm deserialization and remote wrapper refresh

Source inspection confirmed V8's native Wasm deserializer already reconstructs
real WebAssembly.Memory objects from its SAB delegate, including cyclic graph
links; the native delivery mapper must return an actual Wasm backing for Wasm
references. Added backing kind, address type, page alignment, and maximum extent
validation before attaching that backing to a deserialized memory object.

Added fixed-length shared Wasm buffer refresh on native `memory.buffer` reads.
Another process cannot broadcast to the local V8 isolate, so the getter compares
its cached wrapper extent with the acquire-loaded backing length and materializes
a new wrapper after remote growth. Previously returned fixed wrappers retain their
original extent. Requests for an already-selected buffer resizability also pass
through this refresh path.

Generated an engine regression covering native-backed Wasm construction, real
V8 serialization/deserialization, module import, shared bytes, local growth,
remote metadata growth without instance refresh, updated getter identity/extent,
preserved old wrapper length, compiled bounds, and incorrect backing-kind
rejection. The callback retains one backing rather than allocating a new memory
on deserialization. These are unexecuted source regressions; real process broker
mapping and qualification remain required. No execution gate was crossed.

### Zero-length native allocation coverage

Generated Rust/C++ bridge regression source using anonymous file-backed broker
responses for zero-length fixed SAB, zero-maximum GSAB, and zero-page shared Wasm.
The physical mapping reserves one native granularity unit while logical maximum
and current length remain zero. Assertions cover growability distinctions,
zero growth, rejection above maximum, exact native creator routing, and rejection
of new allocation before broker dispatch after environment retirement. These
fixtures exercise the actual constructor delegate and descriptor mapping ABI;
they are generated, not executed.

Source audit also identified that waitAsync environment ownership cannot follow
the mapping creator after a SAB survives in another realm. The W/G track has
updated queue selection to the current native execution context and removed
creator-environment retirement as a blanket revocation of physical backing waits.
Its binding-backed regression integration remains ongoing.

### Current-realm allocation authority regression

Added native mapped-constructor coverage for two contexts sharing one isolate.
Both install the allocation delegate; alternating allocations assert distinct
route-bound agent identities and broker counts. After retiring one context,
allocation in its live peer still succeeds. The test broker rejects any allocation
using the other realm's identity. This covers delegate installation replacement
and execution-time authority lookup without retaining realm authority in the
isolate hook. Source generated only; no build or test execution.

### Current engine artifact acceptance path

Source audit found the signing script still accepted an obsolete schema-1 V8
manifest, while the engine builder emits schema 4. Replaced that path with a
current-profile validator covering all six engine patch hashes and prerequisites,
pinned revisions, sandbox/pointer-compression/DrumBrake bounds options, requested
profile/target, and exact sizes/digests of archive, bindings, ICU data, snapshot,
external references and worker. Added the missing ICU source path to the builder's
manifest. Signing now requires an explicit current core-full manifest before its
build step. This validates build inputs, not conformance or independent provenance.
Regression sources reject stale patches, modified archives, old schema, target
mismatch and conflicting bounds flags. None of these scripts or tests was run.

### Runtime and signed-bundle engine identity

Generated build-time archive and ordered patch-chain fingerprints in native
runtime metadata. Initial worker handshakes compare both fingerprints to the
parent build; reused worker environments must preserve them. The signing script
checks the built worker's archive against the selected validated profile, copies
that profile into signed resources, and records its digest alongside archive and
patch-chain identity. Bundle verification checks the profile digest even before
returning an executable-verification cache hit, then compares manifest engine
identity to the compiled parent. Missing legacy fields reject rather than attest
an older engine. Actual archive production, signing, tests, and qualification are
still deferred until all generation is complete.

### Cached signed-profile regression

Generated a regression that seeds a previously verified executable/manifest cache
entry, verifies the unchanged bundled engine profile, then replaces only the
profile and requires rejection before cached verification is returned. The test
uses a private temporary fixture and no signing identity or worker execution.
Updated native constructor broker fixtures to admit the newly integrated native
Bind operation while preserving exact agent identity and allocation-count checks.
All sources remain unbuilt and untested.

### Wait admission interruption interface

Generated a distinct native `kAdmitting` state and an engine-owned interruption
callback for native waitAsync admission. The callback invokes V8's stack guard,
so pending termination/GC work is processed between bounded admission polls.
The synchronous wait loop also recognizes admission-pending state. The C++ bridge
uses a separate admission callback to preserve atomic compare/enqueue's original
classification even when notification arrives before the result object is built.
The Rust adapter retains this classification separately from later wait results.

Added engine regression source for termination during waitAsync admission and a
Rust regression assertion preserving asynchronous classification after early
completion. Updated the wait patch, dependent patch digests, vendored API header
and test-source manifest through source-edit scripts only. No engine builds,
project generators, tests, signing or qualification ran.

The Rust Begin/owner-Open transport path still blocks. The worker agent owns the
bounded request lane and late-reply cancellation custody integration required to
make the new admission contract true end to end. The interface alone is not a
claim of bounded wait interruption or completed shared-memory conformance.
