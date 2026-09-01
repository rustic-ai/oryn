use std::{env, fs, path::Path};

use sha2::{Digest, Sha256};

fn digest(path: &Path) -> String {
    let bytes =
        fs::read(path).unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    format!("{:x}", Sha256::digest(bytes))
}

fn main() {
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
