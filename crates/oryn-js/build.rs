mod v8_bridge_abi;
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
    let patch = Path::new("../../patches/v8/window-proxy-structural-semantics.patch");
    assert_eq!(
        digest(patch),
        manifest["v8_patch_sha256"]
            .as_str()
            .expect("qualified V8 patch hash"),
        "V8 public headers must match the qualified engine patch"
    );
    println!("cargo:rerun-if-changed={}", patch.display());
    let growth_patch = Path::new("../../patches/v8/shared-memory-growth.patch");
    assert_eq!(
        digest(growth_patch),
        manifest["shared_memory_growth_patch_sha256"]
            .as_str()
            .expect("shared-memory engine patch hash"),
        "V8 shared-memory API headers must match the pinned engine patch"
    );
    println!("cargo:rerun-if-changed={}", growth_patch.display());
    let wait_patch = Path::new("../../patches/v8/shared-memory-wait.patch");
    assert_eq!(
        digest(wait_patch),
        manifest["shared_memory_wait_patch_sha256"]
            .as_str()
            .expect("shared wait patch hash"),
        "V8 shared-memory wait API must match the pinned engine patch"
    );
    println!("cargo:rerun-if-changed={}", wait_patch.display());
    let wasm_patch = Path::new("../../patches/v8/shared-wasm-backing.patch");
    assert_eq!(
        digest(wasm_patch),
        manifest["shared_wasm_backing_patch_sha256"]
            .as_str()
            .expect("shared Wasm backing patch hash"),
        "V8 shared Wasm API must match its pinned engine patch"
    );
    println!("cargo:rerun-if-changed={}", wasm_patch.display());
    let factory_patch = Path::new("../../patches/v8/shared-memory-factory.patch");
    assert_eq!(
        digest(factory_patch),
        manifest["shared_memory_factory_patch_sha256"]
            .as_str()
            .expect("shared factory patch hash"),
        "V8 shared allocation API must match its pinned engine patch"
    );
    println!("cargo:rerun-if-changed={}", factory_patch.display());
    let reattachment_patch = Path::new("../../patches/v8/frame-context-reattachment.patch");
    assert_eq!(
        digest(reattachment_patch),
        manifest["frame_context_reattachment_patch_sha256"]
            .as_str()
            .expect("context reattachment patch hash"),
        "V8 context reattachment API must match its pinned engine patch"
    );
    println!("cargo:rerun-if-changed={}", reattachment_patch.display());
    let html_constructor_patch =
        Path::new("../../patches/v8/html-constructor-deferred-prototype.patch");
    assert_eq!(
        digest(html_constructor_patch),
        manifest["html_constructor_deferred_prototype_patch_sha256"]
            .as_str()
            .expect("HTML constructor patch hash"),
        "V8 HTML constructor API must match its pinned engine patch"
    );
    println!(
        "cargo:rerun-if-changed={}",
        html_constructor_patch.display()
    );
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

fn verify_generated_registry() {
    let input = Path::new("../oryn-webidl/idl/browser.webidl");
    let output = Path::new("src/generated/browser.rs");
    println!("cargo:rerun-if-changed={}", input.display());
    println!("cargo:rerun-if-changed={}", output.display());
    let source = fs::read_to_string(input).expect("authored browser WebIDL");
    let definitions =
        oryn_webidl::parse_definitions(&source).expect("supported authored browser WebIDL");
    let expected = oryn_webidl::emit_binding_registry(&definitions)
        .expect("complete generated browser registry");
    assert!(
        fs::read_to_string(output).expect("checked-in browser registry") == expected,
        "generated bindings are stale; run cargo run -p oryn-webidl -- generate crates/oryn-webidl/idl/browser.webidl crates/oryn-js/src/generated/browser.rs"
    );
}

fn main() {
    verify_generated_registry();
    if env::var_os("CARGO_FEATURE_V8_HOST").is_some() {
        verify_v8_api_headers();
        // macOS SDK libxml2 cannot install the allocator hooks required by
        // the native budget. Use the explicitly supplied static engine there.
        if let Some(directory) = env::var_os("ORYN_XSLT_STATIC_LIB_DIR") {
            let include = env::var_os("ORYN_XSLT_INCLUDE_DIR")
                .expect("static XSLT libraries require their matching headers");
            let prefix = Path::new(&directory).parent().expect("XSLT prefix");
            assert_eq!(Path::new(&include), prefix.join("include/libxml2"),
                "XSLT headers and archives must use the same pinned prefix");
            for header in ["include/libxml2/libxml/xmlversion.h", "include/libxslt/xsltconfig.h"] {
                assert!(prefix.join(header).is_file(), "missing XSLT configuration header");
                println!("cargo:rerun-if-changed={}", prefix.join(header).display());
            }
        }
        let mut xslt = cc::Build::new();
        xslt.file("src/xslt_bridge.c").flag_if_supported("-std=c11");
        if let Some(include) = env::var_os("ORYN_XSLT_INCLUDE_DIR") {
            xslt.include(include);
            if let Some(directory) = env::var_os("ORYN_XSLT_STATIC_LIB_DIR") {
                xslt.include(Path::new(&directory).parent().expect("XSLT prefix").join("include"));
            }
        } else if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
            let sdk =
                env::var("SDKROOT").expect("XSLT compilation requires the admitted macOS SDKROOT");
            xslt.include(Path::new(&sdk).join("usr/include/libxml2"));
        } else {
            xslt.include("/usr/include/libxml2");
        }
        xslt.compile("oryn_xslt");
        if let Some(directory) = env::var_os("ORYN_XSLT_STATIC_LIB_DIR") {
            let directory = Path::new(&directory);
            for library in ["libxslt.a", "libxml2.a"] {
                assert!(directory.join(library).is_file(), "missing pinned XSLT archive: {library}");
                println!("cargo:rerun-if-changed={}", directory.join(library).display());
            }
            println!("cargo:rustc-link-search=native={}", directory.display());
            println!("cargo:rustc-link-lib=static=xslt");
            println!("cargo:rustc-link-lib=static=xml2");
        } else {
            assert!(env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos"),
                "macOS XSLT requires pinned static libxml2/libxslt with working allocator hooks; run scripts/build-xslt-libraries.sh and set ORYN_XSLT_INCLUDE_DIR / ORYN_XSLT_STATIC_LIB_DIR");
            println!("cargo:rustc-link-lib=xslt");
            println!("cargo:rustc-link-lib=xml2");
        }
        println!("cargo:rerun-if-changed=src/xslt_bridge.c");
        println!("cargo:rerun-if-env-changed=ORYN_XSLT_INCLUDE_DIR");
        println!("cargo:rerun-if-env-changed=ORYN_XSLT_STATIC_LIB_DIR");

        let mut bridge = cc::Build::new();
        bridge
            .cpp(true)
            .file("src/v8_context_bridge.cc")
            .file("src/v8_pattern_bridge.cc")
            .file("src/v8_shared_memory_bridge.cc")
            .file("src/v8_shared_wait_bridge.cc")
            .include("../../vendor/v8-api/include")
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
        v8_bridge_abi::configure(&mut bridge);
        bridge.compile("oryn_v8_context_bridge");
        println!("cargo:rerun-if-changed=v8_bridge_abi.rs");
        println!("cargo:rerun-if-changed=src/v8_context_bridge.cc");
        println!("cargo:rerun-if-changed=src/v8_pattern_bridge.cc");
        println!("cargo:rerun-if-changed=src/v8_shared_memory_bridge.cc");
        println!("cargo:rerun-if-changed=src/v8_shared_wait_bridge.cc");
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
