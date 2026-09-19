#[path = "build/rusty_v8_source.rs"]
mod rusty_v8_source;
#[path = "src/media_capture/vpx_build.rs"]
mod vpx_build;
use std::{env, fs, path::Path};

use sha2::{Digest, Sha256};

fn digest(path: &Path) -> String {
    let bytes =
        fs::read(path).unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    format!("{:x}", Sha256::digest(bytes))
}

fn main() {
    if env::var_os("CARGO_FEATURE_V8_HOST").is_some() {
        rusty_v8_source::emit(Path::new("../.."));
    }

    // Bind native decoder qualification to the source actually compiled into
    // its libtest executable. This is independent of the signed worker's hash.
    let mut media_sources = vec![
        "../../Cargo.toml".to_owned(),
        "../../Cargo.lock".to_owned(),
        "build.rs".to_owned(),
        "Cargo.toml".to_owned(),
        "src/media_capture.rs".to_owned(),
    ];
    for entry in fs::read_dir("src/media_capture").expect("media source directory") {
        let path = entry.expect("media source entry").path();
        if path.is_file() {
            media_sources.push(path.to_str().expect("media source path").to_owned());
        }
    }
    fn vendor_sources(directory: &Path, paths: &mut Vec<String>) {
        for entry in fs::read_dir(directory).expect("vendor source directory") {
            let path = entry.expect("vendor source entry").path();
            if path.is_dir() {
                vendor_sources(&path, paths)
            } else if path.is_file() {
                paths.push(path.to_str().expect("vendor source path").to_owned())
            }
        }
    }
    vendor_sources(Path::new("../../vendor/libvpx"), &mut media_sources);
    media_sources.sort();
    let mut media_hash = Sha256::new();
    for path in media_sources {
        println!("cargo:rerun-if-changed={path}");
        let bytes = fs::read(&path).expect("media source");
        media_hash.update((path.len() as u64).to_be_bytes());
        media_hash.update(path.as_bytes());
        media_hash.update((bytes.len() as u64).to_be_bytes());
        media_hash.update(bytes);
    }
    println!(
        "cargo:rustc-env=ORYN_NATIVE_MEDIA_SOURCE_SHA256={:x}",
        media_hash.finalize()
    );
    println!("cargo:rerun-if-changed=src/media_capture/macos.m");
    println!("cargo:rerun-if-changed=src/media_capture/webm_macos.inc");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let vpx_source = vpx_build::build();
        cc::Build::new()
            .include(vpx_source)
            .file("src/media_capture/macos.m")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .compile("oryn_capture");
        for framework in [
            "AVFoundation",
            "Foundation",
            "CoreMedia",
            "CoreVideo",
            "AudioToolbox",
            "AVFAudio",
            "VideoToolbox",
        ] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }

    println!("cargo:rustc-check-cfg=cfg(oryn_startup_snapshot)");
    println!("cargo:rustc-check-cfg=cfg(oryn_icu_data)");
    for name in [
        "ORYN_STARTUP_SNAPSHOT",
        "ORYN_ICU_DATA_PATH",
        "ORYN_ICU_DATA_FINGERPRINT_PATH",
        "ORYN_EXTERNAL_REFERENCES_SHA256",
        "RUSTY_V8_ARCHIVE",
        "RUSTY_V8_SRC_BINDING_PATH",
        "ORYN_V8_BRIDGE_ABI",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let mut engine_chain = Sha256::new();
    for name in [
        "window-proxy-structural-semantics",
        "shared-memory-growth",
        "shared-memory-wait",
        "shared-wasm-size",
        "shared-wasm-backing",
        "shared-memory-factory",
        "frame-context-reattachment",
        "html-constructor-deferred-prototype",
    ] {
        let path = Path::new("../../patches/v8").join(format!("{name}.patch"));
        println!("cargo:rerun-if-changed={}", path.display());
        engine_chain.update(name.as_bytes());
        engine_chain.update([0]);
        engine_chain.update(fs::read(&path).expect("current native V8 patch source"));
        engine_chain.update([0]);
    }
    println!(
        "cargo:rustc-env=ORYN_V8_PATCH_CHAIN_SHA256={:x}",
        engine_chain.finalize()
    );
    if let Some(archive) = env::var_os("RUSTY_V8_ARCHIVE") {
        let archive = Path::new(&archive);
        println!("cargo:rerun-if-changed={}", archive.display());
        println!("cargo:rustc-env=ORYN_V8_ARCHIVE_SHA256={}", digest(archive));
    }
    if let Some(binding) = env::var_os("RUSTY_V8_SRC_BINDING_PATH") {
        let binding = Path::new(&binding);
        println!("cargo:rerun-if-changed={}", binding.display());
        println!("cargo:rustc-env=ORYN_V8_BINDING_SHA256={}", digest(binding));
    }
    if let Some(manifest) = env::var_os("ORYN_V8_BRIDGE_ABI") {
        let manifest = Path::new(&manifest);
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!(
            "cargo:rustc-env=ORYN_V8_BRIDGE_ABI_SHA256={}",
            digest(manifest)
        );
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
