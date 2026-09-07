use sha2::{Digest, Sha256};
use std::{env, fs, path::Path};
fn digest(path: &Path) -> String {
    let bytes =
        fs::read(path).unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    format!("{:x}", Sha256::digest(bytes))
}
fn verify_v8_api_headers() {
    let root = Path::new("../../vendor/v8-api");
    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join("manifest.json")).expect("V8 API header manifest"),
    )
    .expect("valid V8 API header manifest");
    for (name, expected) in manifest["headers"]
        .as_object()
        .expect("V8 API header hashes")
    {
        assert_eq!(
            digest(&root.join("include").join(name)),
            expected.as_str().unwrap(),
            "V8 API header {name} differs from its pinned source"
        );
    }
    let lock = fs::read_to_string("../../Cargo.lock").expect("workspace Cargo.lock");
    let versions: Vec<_> = lock
        .split("[[package]]")
        .filter(|package| package.lines().any(|line| line.trim() == "name = \"v8\""))
        .filter_map(|package| {
            package.lines().find_map(|line| {
                line.trim()
                    .strip_prefix("version = ")
                    .map(|version| version.trim_matches('"'))
            })
        })
        .collect();
    assert_eq!(
        versions,
        vec![manifest["rusty_v8_version"].as_str().unwrap()],
        "qualify and synchronize the V8 API headers when updating rusty_v8"
    );
    println!("cargo:rerun-if-changed=../../Cargo.lock");
}

fn main() {
    if env::var_os("CARGO_FEATURE_V8_HOST").is_some() {
        verify_v8_api_headers();
        let mut bridge = cc::Build::new();
        bridge
            .cpp(true)
            .file("src/v8_context_bridge.cc")
            .include("../../vendor/v8-api/include")
            .std("c++20")
            .flag_if_supported("-Wno-unused-parameter")
            .flag_if_supported("-Wno-cast-function-type-mismatch");
        if env::var_os("CARGO_FEATURE_V8_SANDBOX").is_some() {
            bridge
                .define("V8_COMPRESS_POINTERS", None)
                .define("V8_COMPRESS_POINTERS_IN_SHARED_CAGE", None)
                .define("V8_31BIT_SMIS_ON_64BIT_ARCH", None)
                .define("V8_ENABLE_SANDBOX", None)
                .define("V8_EXTERNAL_CODE_SPACE", None);
        }
        bridge.compile("oryn_v8_context_bridge");
        println!("cargo:rerun-if-changed=src/v8_context_bridge.cc");
        println!("cargo:rerun-if-changed=../../vendor/v8-api");
    }

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
