fn main() {
    let macos = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");
    let embeds_v8 = std::env::var_os("CARGO_FEATURE_V8_HOST").is_some()
        || std::env::var_os("CARGO_FEATURE_V8_SANDBOX").is_some();
    if macos && embeds_v8 {
        // Chromium's partition allocator queries the process JIT entitlement.
        // Production binaries already link these frameworks transitively, but
        // the standalone generated-binding test executable must do so itself.
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=Security");
    }
}
