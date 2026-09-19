// Exercise the vendored driver regressions with the workspace's exact patched
// html5ever/markup5ever pair, without a second vendor dependency resolution.
// Preserve the canonical external regression bytes; this module still compiles and runs.
#[rustfmt::skip]
#[path = "../../../vendor/html5ever/tests/driver.rs"]
mod upstream_driver;
