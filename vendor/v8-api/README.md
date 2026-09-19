# Pinned V8 public API headers

Transitive public headers for the four native bridge translation units, including
`v8-context.h`, `v8-isolate.h`, `v8-version.h`, `v8-template.h`, `v8-regexp.h`,
`v8-function.h`, `v8-proxy.h`, `v8-external.h` and the `v8.h` umbrella, from the
exact V8 revision recorded in `manifest.json`. The complete quoted-include closure
currently contains 61 headers.
`v8-template.h` additionally contains the opt-in WindowProxy declaration from the
hash-pinned [engine patch](../../patches/v8/window-proxy-structural-semantics.patch).
The BSD license is retained in `LICENSE`. These declarations support the small
C ABI bridge for public APIs not exposed by rusty_v8 150.4.0: incumbent context
lookup, global-proxy detachment, cross-context access handlers and the WindowProxy
structural opt-in, and bounded Unicode-set regular expressions for native form
validation. No V8 implementation is vendored here.

Keep this manifest and the locked rusty_v8 version in sync when qualifying a V8
update. Recopy the transitive quoted includes from that revision's `v8/include`
directory, apply the recorded engine patch, and recompute each SHA-256. Builds
using this bridge require the patched archive produced by
`scripts/build-v8-runtime-profile.sh`; the upstream archive lacks the new API.
The optional `V8_GN_HEADER` configuration
and z/OS headers are not used by our builds. The bridge uses the same pointer
compression and sandbox configuration as rusty_v8; it exchanges only pointers,
never C++ standard-library objects. Runtime execution and sandbox checks remain
in the pinned V8 library.

The pinned template header also declares the native HTML-constructor deferred
prototype opt-in from `html-constructor-deferred-prototype.patch`. Its exact
patch and public-header hashes are recorded in the manifest and checked by the
bridge build. The profile builder requires schema 5 and the complete eight-patch
chain; an earlier patched archive cannot satisfy this new bridge symbol.

## Header-closure repair provenance

The additional public headers needed by the context and shared-wait bridges were
copied byte-for-byte from the cached crates.io `v8-150.4.0.crate`, whose SHA-256
`42a978ff11f15b24e5c05a7123cf2b68f41e763546699781a924ef4e2cf43a49`
matches `Cargo.lock`. Its packaged rusty_v8 revision matches this manifest. The
archive itself records `dirty: true` in `.cargo_vcs_info.json`; that is preserved
release metadata, not evidence of an unverified local header modification.

Only missing headers were copied. The existing patched `v8-array-buffer.h`,
`v8-context.h`, `v8-isolate.h` and `v8-template.h` were not overwritten. All added
headers have individual hashes in the existing manifest. This changes no V8
version, engine implementation, engine patch, sandbox setting or archive ABI.
The normal candidate/profile qualification remains required after source repair.
