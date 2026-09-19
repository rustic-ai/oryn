#![cfg(feature = "v8-host")]
// Compile the exact reviewed regression source against the workspace's selected
// patched v8 dependency, rather than an independently resolved vendor workspace.
// Preserve the canonical external regression bytes; this module still compiles and runs.
#[rustfmt::skip]
#[path = "../../../patches/rusty-v8/weak-teardown-regressions.rs"]
mod regression;
