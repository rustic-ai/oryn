//! Independent build-time validation of the Rust crate selected by Cargo.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};
const VENDOR: &str = "vendor/rusty-v8";
const MANIFEST: &str = "vendor/rusty-v8/oryn-source-manifest.json";
const PATCH: &str = "patches/rusty-v8/weak-callback-teardown.patch";
const METADATA: &str = "patches/rusty-v8/weak-callback-teardown.json";
fn bytes(path: &Path) -> Vec<u8> {
    println!("cargo:rerun-if-changed={}", path.display());
    fs::read(path).expect("Rust V8 provenance input")
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn inventory(root: &Path, directory: &Path, result: &mut BTreeMap<String, String>) {
    println!("cargo:rerun-if-changed={}", directory.display());
    for entry in fs::read_dir(directory).expect("Rust V8 vendor directory") {
        let entry = entry.expect("Rust V8 source entry");
        let path = entry.path();
        let kind = entry.file_type().expect("Rust V8 source kind");
        assert!(!kind.is_symlink(), "Rust V8 vendor symlink forbidden");
        if kind.is_dir() {
            inventory(root, &path, result)
        } else if kind.is_file() {
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .expect("UTF-8 Rust V8 source path")
                .replace('\\', "/");
            if name != "oryn-source-manifest.json" {
                result.insert(name, hash(&bytes(&path)));
            }
        }
    }
}
pub fn emit(root: &Path) {
    for path in [root.join(VENDOR), root.join(MANIFEST)] {
        assert!(
            !fs::symlink_metadata(&path)
                .expect("Rust V8 selected source metadata")
                .file_type()
                .is_symlink(),
            "Rust V8 vendor root/manifest symlink forbidden"
        );
    }
    let cargo: toml::Value =
        toml::from_str(std::str::from_utf8(&bytes(&root.join("Cargo.toml"))).unwrap())
            .expect("workspace Cargo manifest");
    let selected = cargo["patch"]["crates-io"]["v8"]
        .as_table()
        .expect("local Cargo V8 override");
    assert_eq!(selected.len(), 1, "unexpected Cargo V8 override fields");
    assert_eq!(selected["path"].as_str(), Some(VENDOR));
    let lock: toml::Value =
        toml::from_str(std::str::from_utf8(&bytes(&root.join("Cargo.lock"))).unwrap())
            .expect("Cargo lock");
    let packages = lock["package"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["name"].as_str() == Some("v8"))
        .collect::<Vec<_>>();
    assert_eq!(packages.len(), 1, "ambiguous Cargo V8 resolution");
    let package = packages[0].as_table().unwrap();
    assert_eq!(package["version"].as_str(), Some("150.4.0"));
    assert!(
        !package.contains_key("source") && !package.contains_key("checksum"),
        "Cargo resolved registry V8 instead of local crate"
    );
    let raw = bytes(&root.join(MANIFEST));
    let manifest: Value = serde_json::from_slice(&raw).expect("Rust V8 manifest JSON");
    assert_eq!(manifest["schema_version"], 1);
    assert_eq!(manifest["crate"], "v8");
    assert_eq!(manifest["version"], "150.4.0");
    assert_eq!(manifest["archive_only"], true);
    assert_eq!(
        manifest["registry_checksum"],
        "42a978ff11f15b24e5c05a7123cf2b68f41e763546699781a924ef4e2cf43a49"
    );
    let mut files = BTreeMap::new();
    inventory(&root.join(VENDOR), &root.join(VENDOR), &mut files);
    assert!(!files.is_empty());
    assert_eq!(
        manifest["files"],
        serde_json::to_value(&files).unwrap(),
        "Rust V8 source drift"
    );
    let tree = hash(&serde_json::to_vec(&files).unwrap());
    assert_eq!(manifest["tree_sha256"].as_str(), Some(tree.as_str()));
    let metadata_raw = bytes(&root.join(METADATA));
    let metadata: Value = serde_json::from_slice(&metadata_raw).expect("Rust V8 patch metadata");
    assert_eq!(metadata["schema_version"], 1);
    assert_eq!(metadata["crate"], "v8");
    assert_eq!(metadata["crate_version"], "150.4.0");
    assert_eq!(metadata["cpp_changes"], false);
    assert!(
        metadata["source_basis"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    let patch = hash(&bytes(&root.join(PATCH)));
    assert_eq!(metadata["patch_sha256"].as_str(), Some(patch.as_str()));
    assert_eq!(
        manifest["patches"],
        serde_json::json!([{"path":PATCH,"sha256":patch}])
    );
    let supplemental = serde_json::json!({"third_party/icu/common/icudtl.dat":{
        "sha256":"1cf67874b5a87a8363a86fb3f81e3cbbed54d389062dab8fb52308d5cf8c8612",
        "source_path":"target/rusty-v8-src-150.4.0-schema5/third_party/icu/common/icudtl.dat",
        "icu_revision":"ee5f27adc28bd3f15b2c293f726d14d2e336cbd5"}});
    assert_eq!(manifest["supplemental_files"], supplemental);
    for (name, receipt) in supplemental.as_object().unwrap() {
        assert_eq!(
            files.get(name).map(String::as_str),
            receipt["sha256"].as_str()
        );
    }
    let base = manifest["base_files"]
        .as_object()
        .expect("Rust V8 base inventory");
    let before = metadata["base_files"]
        .as_object()
        .expect("patch base sources");
    let after = metadata["patched_files"]
        .as_object()
        .expect("patch final sources");
    assert!(!before.is_empty());
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>()
    );
    for name in base.keys() {
        assert!(files.contains_key(name), "removed base source");
        assert!(
            supplemental.get(name).is_none(),
            "supplemental source claimed as upstream"
        );
    }
    for (name, digest) in &files {
        if supplemental.get(name).is_some() {
            continue;
        }
        if base.get(name).and_then(Value::as_str) != Some(digest.as_str()) {
            assert!(
                before.contains_key(name),
                "unreviewed Rust V8 source change"
            );
        } else {
            assert!(!before.contains_key(name), "patch source unchanged");
        }
    }
    for (name, old) in before {
        assert!(
            name == "Cargo.toml"
                || name == "build.rs"
                || (name.starts_with("src/") || name.starts_with("tests/"))
                    && name.ends_with(".rs")
        );
        assert!(!name.split('/').any(|part| part == ".."));
        assert_eq!(base.get(name).unwrap_or(&Value::Null), old);
        assert_eq!(files.get(name).map(String::as_str), after[name].as_str());
    }
    println!(
        "cargo:rustc-env=ORYN_RUSTY_V8_SOURCE_MANIFEST_SHA256={}",
        hash(&raw)
    );
    println!("cargo:rustc-env=ORYN_RUSTY_V8_SOURCE_TREE_SHA256={tree}");
    println!("cargo:rustc-env=ORYN_RUSTY_V8_WEAK_TEARDOWN_PATCH_SHA256={patch}");
    println!(
        "cargo:rustc-env=ORYN_RUSTY_V8_PATCH_METADATA_SHA256={}",
        hash(&metadata_raw)
    );
}
