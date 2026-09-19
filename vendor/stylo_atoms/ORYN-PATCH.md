# Oryn stylo_atoms patch

Upstream: Servo `stylo_atoms` crate, version **0.20.0**, crates.io package from
https://crates.io/crates/stylo_atoms/0.20.0. Repository: https://github.com/servo/stylo.
License: **MPL-2.0**, as declared in the unchanged Cargo manifests and source
license headers.

The eight package files (Cargo.toml, Cargo.toml.orig, Cargo.lock, README.md,
build.rs, lib.rs, predefined_counter_styles.rs, static_atoms.txt) were copied
from the pinned local crates.io cache. Registry bookkeeping files are omitted.
The sole upstream-source change adds `prefers-reduced-motion` to
`static_atoms.txt`, adjacent to `prefers-color-scheme`.

The unchanged build.rs uses string_cache_codegen to generate OUT_DIR/atom.rs.
No generated atom hashes or macro output are hand-maintained. This patch makes
the existing native Stylo media feature available to the compile-time atom macro.
No generator, build or test was run while introducing this source patch.
