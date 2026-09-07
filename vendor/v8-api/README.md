# Pinned V8 public API headers

Transitive public headers for `v8-context.h`, `v8-isolate.h`, `v8-version.h` and
`v8-template.h`, from the exact V8 revision recorded in `manifest.json`.
`v8-template.h` additionally contains the opt-in WindowProxy declaration from the
hash-pinned [engine patch](../../patches/v8/window-proxy-structural-semantics.patch).
The BSD license is retained in `LICENSE`. These declarations support the small
C ABI bridge for public APIs not exposed by rusty_v8 150.4.0: incumbent context
lookup, global-proxy detachment, cross-context access handlers and the WindowProxy
structural opt-in. No V8 implementation is vendored here.

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
