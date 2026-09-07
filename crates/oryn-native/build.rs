use std::{env, fs, path::Path};

use sha2::{Digest, Sha256};

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
    println!("cargo:rustc-check-cfg=cfg(oryn_startup_snapshot)");
    println!("cargo:rustc-check-cfg=cfg(oryn_icu_data)");
    for name in [
        "ORYN_STARTUP_SNAPSHOT",
        "ORYN_ICU_DATA_PATH",
        "ORYN_ICU_DATA_FINGERPRINT_PATH",
        "ORYN_EXTERNAL_REFERENCES_SHA256",
        "RUSTY_V8_ARCHIVE",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let idl = Path::new("../oryn-webidl/idl/browser.webidl");
    println!("cargo:rerun-if-changed={}", idl.display());
    println!(
        "cargo:rustc-env=ORYN_WEBIDL_MANIFEST_SHA256={}",
        digest(idl)
    );
    let external_references = Path::new("assets/external-references-v1.json");
    println!("cargo:rerun-if-changed={}", external_references.display());
    println!(
        "cargo:rustc-env=ORYN_EXTERNAL_REFERENCES_SHA256={}",
        digest(external_references)
    );

    if let Ok(path) = env::var("ORYN_STARTUP_SNAPSHOT") {
        let path = Path::new(&path)
            .canonicalize()
            .expect("startup snapshot path");
        println!("cargo:rerun-if-changed={}", path.display());
        println!("cargo:rustc-cfg=oryn_startup_snapshot");
        println!(
            "cargo:rustc-env=ORYN_STARTUP_SNAPSHOT_PATH={}",
            path.display()
        );
        println!(
            "cargo:rustc-env=ORYN_STARTUP_SNAPSHOT_SHA256={}",
            digest(&path)
        );
    }
    if let Ok(path) = env::var("ORYN_ICU_DATA_PATH") {
        let path = Path::new(&path).canonicalize().expect("ICU data path");
        println!("cargo:rerun-if-changed={}", path.display());
        println!("cargo:rustc-cfg=oryn_icu_data");
        println!("cargo:rustc-env=ORYN_ICU_DATA_PATH={}", path.display());
        println!("cargo:rustc-env=ORYN_ICU_DATA_SHA256={}", digest(&path));
    } else if let Ok(path) = env::var("ORYN_ICU_DATA_FINGERPRINT_PATH") {
        let path = Path::new(&path)
            .canonicalize()
            .expect("ICU fingerprint path");
        println!("cargo:rerun-if-changed={}", path.display());
        println!("cargo:rustc-env=ORYN_ICU_DATA_SHA256={}", digest(&path));
    }

    let lite = env::var_os("CARGO_FEATURE_CORE_LITE").is_some();
    let release = env::var("PROFILE").is_ok_and(|profile| profile == "release");
    let v8_host = env::var_os("CARGO_FEATURE_V8_HOST").is_some();
    let v8_sandbox = env::var_os("CARGO_FEATURE_V8_SANDBOX").is_some();
    if release && v8_host && !v8_sandbox {
        assert!(
            env::var_os("ORYN_ALLOW_UNSANDBOXED_V8_PROBE").is_some(),
            "release V8 artifacts require v8-sandbox; set ORYN_ALLOW_UNSANDBOXED_V8_PROBE only for explicitly unattested probes"
        );
    }
    if lite && release {
        assert!(
            env::var_os("ORYN_ICU_DATA_PATH").is_some(),
            "production core-lite requires an actual reduced ICU data artifact"
        );
        assert!(
            env::var_os("RUSTY_V8_ARCHIVE").is_some(),
            "production core-lite requires its matching custom sandboxed V8 archive"
        );
    }
}
