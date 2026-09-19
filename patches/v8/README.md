# WindowProxy engine experiment

`window-proxy-structural-semantics.patch` adds an opt-in public ObjectTemplate
method to the pinned V8 revision recorded in the adjacent JSON manifest. The
runtime profile builder now applies it and records its hash. Release qualification
remains incomplete.

The isolated candidate passes all nine native context tests and 131 binding tests
(two existing ignores). Its complete native matrix is 283/296, retaining the same
13 prior nonpasses and failure reasons. The
[qualification evidence](../../docs/oryn2/CONFORMANCE-NATIVE-WINDOW-STRUCTURAL-2026-09-06.json)
records exact source and archive hashes. The subsequent
[core-full profile validation](../../docs/oryn2/CONFORMANCE-NATIVE-WINDOW-PROFILE-2026-09-06.json)
uses the canonical build and a fresh snapshot: nine context passes, 131 binding
passes and 284/296 native outcomes. Correcting the external-reference manifest
resolves one prior failure. The signed candidate also passes the builder's
entitlement, OS denial, handshake, DOM-mutation and broker smoke checks.
Core-lite/profile equivalence and remaining security/release gates are still
required.

V8's public failed-access callback cannot implement HTML's cross-origin
`[[SetPrototypeOf]]` and `[[PreventExtensions]]` completions: it must throw, and
does not identify which structural operation failed. The patch handles those
operations in V8 for explicitly marked global proxies. Ordinary objects and
unmarked global templates retain their existing behavior. Property access checks
remain in place. The embedder must still configure immutable prototypes and
cross-origin property handlers.

The flag uses the remaining bit in ObjectTemplateInfo's 31-bit Smi payload;
`From31BitPattern` preserves that bit without changing object layout or reducing
the embedder-field count. Context creation copies it to the generated global
proxy template. This changes engine semantics and requires a new qualified
archive, synchronized public headers, snapshots and profile fingerprints.

Use an isolated source tree at the manifest's V8 revision. Verify the exact input
and patch hashes before applying:

```sh
python3 scripts/apply-v8-window-patch.py --check /path/to/isolated/v8
python3 scripts/apply-v8-window-patch.py /path/to/isolated/v8
```

The helper accepts only the complete base or complete patched file hashes. A
mixed or modified source is rejected. Application alone is not qualification.
Build using the existing pinned Rusty V8 dependencies, toolchain and sandbox GN
settings, with a separate output directory. Before adoption, validate native
Window structural/indexed operations, ordinary-global controls, cross-origin
property denial, navigation/reused proxies and context lifetimes; then run the
native matrix and existing snapshot/profile/sandbox gates. The full conformance
claim still requires two passing runs of the exact 4,022-ID inventory.

### Preserved frame context reattachment

`frame-context-reattachment.patch` is the seventh exact-input patch, after
`shared-memory-factory`. Its manifest pins both input files after the complete
predecessor chain and their resulting hashes against V8
`ac1e23989121713ca642f6650b34deff7b686896`. The public declaration is mirrored
exactly in `vendor/v8-api/include/v8-context.h`.

The implementation follows the pinned engine's `Genesis::HookUpGlobalProxy`
sequence. It accepts only a detached preserved context whose original proxy is
also currently detached. It reconnects that original proxy and the preserved
inner global, then restores the embedder-owned microtask queue. Native host
callers remain responsible for reinstating their authoritative security token
and keeping the queue alive.

The runtime-profile builder applies this patch and installs
`test-oryn-frame-context-reattachment.cc` into the pinned cctest target. Profile
validation requires the patch hash and regression-manifest hash; source
installation accepts only the exact clean, previous, or current target and
rejects divergent existing fixtures. These are source/provenance records,
not engine build or conformance qualification. No build or test was executed
while generating this integration.

The same patch also supplies per-context microtask suspension and FIFO transfer
back into the common agent queue. `DetachGlobalPreservingMicrotasks` retains the
paused queue, and `ReattachGlobal(nullptr)` preserves it until native restore
explicitly resumes it. The cctest fixture covers retained Promise work,
restoration exactly once, and rejection of suspension while contexts are
entered or the shared queue still has pending work. These remain unexecuted
regression sources pending the generation gate.

### Native HTML constructor prototype ordering

`html-constructor-deferred-prototype.patch` is the eighth exact-input patch,
following `frame-context-reattachment`, on the same pinned V8 revision. Its public
`FunctionTemplate::SetOrynDeferredPrototype` opt-in defers the engine receiver's
`NewTarget.prototype` lookup. Native HTML constructors validate the active
constructor/registry and interface first, then perform one observable prototype
lookup. Ordinary API constructors retain V8's existing allocation algorithm.

The flag lives in initialized, tagged `FunctionTemplateRareData`; no existing
flag bits or embedder fields are repurposed. Torque and native layout declarations
change together. This requires regenerating the engine's layout code as part of
the later qualified build; only the source patch has been authored now. The
embedder preserves the original NewTarget and performs the fallback intrinsic
prototype lookup in its native function realm, without authored realm markers.

The build-profile source applies all eight patches in order. Profile schema 5
requires this patch's SHA-256, the complete prior chain, matching vendored public
headers, and the regression-source manifest. The installer verifies the final
combined engine file hashes, including files extended by multiple patches.
`test-oryn-html-constructor.cc` covers pre-admission lookup suppression, exactly
one accepted lookup, and an ordinary-constructor control. Binding regression
sources add registered Proxy, cross-realm primitive prototype fallback, double
super, and reentrant getter consumption of an upgrade stack. All new sources
remain unbuilt/unexecuted; this is not archive or conformance qualification.
