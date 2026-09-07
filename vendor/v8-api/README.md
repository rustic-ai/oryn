# Pinned V8 public API headers

Unmodified transitive public headers for `v8-context.h`, `v8-isolate.h` and
`v8-version.h`, from the exact V8 revision recorded in `manifest.json`.
The BSD license is retained in `LICENSE`. These declarations support the small
C ABI bridge for public APIs not exposed by rusty_v8 150.4.0: incumbent context
lookup and global-proxy detachment. No V8 implementation is vendored here.

Keep this manifest and the locked rusty_v8 version in sync when qualifying a V8
update. Recopy the transitive quoted includes from that revision's `v8/include`
directory and recompute each SHA-256. The optional `V8_GN_HEADER` configuration
and z/OS headers are not used by our builds. The bridge uses the same pointer
compression and sandbox configuration as rusty_v8; it exchanges only pointers,
never C++ standard-library objects. Runtime execution and sandbox checks remain
in the pinned V8 library.
