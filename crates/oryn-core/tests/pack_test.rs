use oryn_core::intent::registry::IntentRegistry;
use oryn_core::pack::loader::PackLoader;
use oryn_core::pack::manager::{PackError, PackManager};
use tokio::fs;

#[tokio::test]
async fn test_pack_loader_manifest_not_found() {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path();

    let result = PackLoader::load_pack(path).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_pack_loader_success() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = temp_dir.path();

    let manifest_path = root.join("pack.yaml");
    let metadata = r#"
pack: "test.com"
version: "1.0.0"
description: "Test Pack"
domains: ["test.com"]
patterns: ["*.yaml"]
intents: ["intents/*.yaml"]
auto_load: ["https://test.com/*"]
    "#;

    fs::write(&manifest_path, metadata).await.unwrap();

    let loaded = PackLoader::load_pack(root).await.unwrap();
    assert_eq!(loaded.metadata.pack, "test.com");
    assert_eq!(loaded.metadata.domains, vec!["test.com"]);
}

#[tokio::test]
async fn test_pack_loader_with_intents() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = temp_dir.path();

    // Create pack structure
    let intents_dir = root.join("intents");
    fs::create_dir_all(&intents_dir).await.unwrap();

    let manifest_path = root.join("pack.yaml");
    let metadata = r#"
pack: "test.com"
version: "1.0.0"
description: "Test Pack"
domains: ["test.com"]
patterns: []
intents: ["intents/*.yaml"]
auto_load: []
    "#;
    fs::write(&manifest_path, metadata).await.unwrap();

    let intent_path = intents_dir.join("test_intent.yaml");
    let intent_yaml = r#"
name: "test_intent"
description: "A test intent"
version: "1.0.0"
tier: "loaded"
steps: []
    "#;
    fs::write(&intent_path, intent_yaml).await.unwrap();

    let loaded = PackLoader::load_pack(root).await.unwrap();
    assert_eq!(loaded.intents.len(), 1);
    assert_eq!(loaded.intents[0].name, "test_intent");
}

#[tokio::test]
async fn test_pack_manager() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = temp_dir.path();

    // Create a pack in "test.com" directory
    let pack_dir = root.join("test.com");
    fs::create_dir(&pack_dir).await.unwrap();

    let manifest_path = pack_dir.join("pack.yaml");
    let metadata = r#"
pack: "test.com"
version: "1.0.0"
description: "Test Pack"
domains: ["test.com"]
"#;
    fs::write(&manifest_path, metadata).await.unwrap();

    let registry = IntentRegistry::new();
    let mut manager = PackManager::new(registry, vec![root.to_path_buf()]);

    // Test Load
    manager.load_pack_by_name("test.com").await.unwrap();
    assert_eq!(manager.list_loaded().len(), 1);

    // Test Auto Load
    assert_eq!(
        manager.should_auto_load("https://github.com/foo"),
        Some("github.com".to_string())
    );

    // Test Unload
    manager.unload_pack("test.com").unwrap();
    assert_eq!(manager.list_loaded().len(), 0);

    // Test Not Found
    let err = manager.load_pack_by_name("nonexistent").await;
    assert!(matches!(err, Err(PackError::NotFound(_))));
}
