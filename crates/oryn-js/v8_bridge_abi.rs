//! Build-time bridge configuration. A system C++ library must never cross V8's
//! Chromium libc++ shared_ptr boundary, including return types invisible to nm.
use serde_json::Value;
use std::{env, fs, path::Path};

fn checked_file(record: &Value) {
    let path = Path::new(record["path"].as_str().expect("ABI input path"));
    assert_eq!(
        fs::metadata(path).expect("retained ABI input").len(),
        record["size_bytes"].as_u64().expect("ABI input size"),
        "ABI size drift: {}",
        path.display()
    );
    assert_eq!(
        super::digest(path),
        record["sha256"].as_str().expect("ABI input hash"),
        "ABI input drift: {}",
        path.display()
    );
    println!("cargo:rerun-if-changed={}", path.display());
}

pub fn configure(bridge: &mut cc::Build) {
    const MANIFEST: &str = "ORYN_V8_BRIDGE_ABI";
    println!("cargo:rerun-if-env-changed={MANIFEST}");
    let path = env::var(MANIFEST).expect("V8 bridge requires ORYN_V8_BRIDGE_ABI; prepare it with scripts/prepare-v8-bridge-abi.py from the retained archive build");
    let path = Path::new(&path);
    println!("cargo:rerun-if-changed={}", path.display());
    let manifest: Value = serde_json::from_slice(&fs::read(path).expect("bridge ABI manifest"))
        .expect("valid bridge ABI manifest");
    assert_eq!(manifest["schema_version"], 1);
    assert_eq!(
        manifest["target"].as_str(),
        env::var("TARGET").ok().as_deref(),
        "bridge/archive target mismatch"
    );
    assert_eq!(manifest["libcxx_namespace"], "__Cr");
    assert_eq!(manifest["libcxx_abi_version"], 2);
    assert_eq!(manifest["stdlib_linkage"], "included-in-rusty-v8");
    for (name, key) in [
        ("RUSTY_V8_ARCHIVE", "archive"),
        ("RUSTY_V8_SRC_BINDING_PATH", "binding"),
    ] {
        println!("cargo:rerun-if-env-changed={name}");
        checked_file(&manifest[key]);
        let selected =
            env::var(name).unwrap_or_else(|_| panic!("{name} must select the ABI-bound artifact"));
        assert_eq!(
            super::digest(Path::new(&selected)),
            manifest[key]["sha256"].as_str().unwrap(),
            "bridge ABI does not describe selected {name}"
        );
        println!("cargo:rerun-if-changed={selected}");
    }
    // cc-rs otherwise appends these after our flags, silently changing the ABI.
    for (key, value) in env::vars() {
        if (key.contains("CXXFLAGS") || key.contains("CXXSTDLIB")) && !value.is_empty() {
            panic!("{key} cannot override the archive-bound bridge ABI");
        }
    }
    for input in manifest["inputs"].as_array().expect("retained ABI inputs") {
        checked_file(input);
    }
    assert!(
        manifest["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|input| input["path"] == manifest["compiler"]),
        "compiler provenance missing"
    );
    if let Some(sdk) = manifest["sdkroot"].as_str() {
        println!("cargo:rerun-if-env-changed=SDKROOT");
        if let Ok(selected) = env::var("SDKROOT") {
            assert_eq!(
                fs::canonicalize(selected).unwrap(),
                fs::canonicalize(sdk).unwrap(),
                "bridge SDK must match archive SDK"
            );
        }
    }
    let flags: Vec<&str> = manifest["flags"]
        .as_array()
        .expect("GN flags")
        .iter()
        .map(|v| v.as_str().expect("GN flag string"))
        .collect();
    assert!(
        flags.contains(&"-nostdinc++"),
        "system C++ headers forbidden"
    );
    bridge
        .no_default_flags(true)
        .compiler(manifest["compiler"].as_str().expect("GN compiler"))
        .cpp_link_stdlib(None)
        .flags(flags);
    println!(
        "cargo:rustc-env=ORYN_V8_BRIDGE_ABI_SHA256={}",
        super::digest(path)
    );
}
