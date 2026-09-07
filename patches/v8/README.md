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
