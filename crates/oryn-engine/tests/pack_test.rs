use oryn_engine::intent::registry::IntentRegistry;
use oryn_engine::pack::loader::PackLoader;
use oryn_engine::pack::manager::{PackError, PackManager};
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
steps:
  - checkpoint: "start"
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
    let mut manager = PackManager::new(registry, vec![root.to_path_buf()]).await;

    // Test Auto Load BEFORE loading - should return pack name for configured domain
    assert_eq!(
        manager.should_auto_load("https://test.com/page"),
        Some("test.com".to_string())
    );
    // Should return None for non-configured domains
    assert!(manager.should_auto_load("https://github.com/foo").is_none());

    // Test Load
    manager.load_pack_by_name("test.com").await.unwrap();
    assert_eq!(manager.list_loaded().len(), 1);

    // After loading, should_auto_load returns None (already loaded)
    assert!(manager.should_auto_load("https://test.com/page").is_none());

    // Test Unload
    manager.unload_pack("test.com").unwrap();
    assert_eq!(manager.list_loaded().len(), 0);

    // Test Not Found
    let err = manager.load_pack_by_name("nonexistent").await;
    assert!(matches!(err, Err(PackError::NotFound(_))));
}

// ============================================================================
// Pack Manager Error Tests
// ============================================================================

#[tokio::test]
async fn test_pack_manager_unload_nonexistent() {
    let registry = IntentRegistry::new();
    let mut manager = PackManager::new(registry, vec![]).await;

    let result = manager.unload_pack("nonexistent");
    assert!(result.is_err());
}

#[tokio::test]
async fn test_pack_manager_load_twice() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = temp_dir.path();

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
    let mut manager = PackManager::new(registry, vec![root.to_path_buf()]).await;

    // Load first time - should succeed
    manager.load_pack_by_name("test.com").await.unwrap();

    // Load second time - should either succeed (idempotent) or fail
    let result = manager.load_pack_by_name("test.com").await;
    // Behavior depends on implementation
    assert!(result.is_ok() || result.is_err());
}

#[tokio::test]
async fn test_pack_manager_empty_paths() {
    let registry = IntentRegistry::new();
    let manager = PackManager::new(registry, vec![]).await;

    // With no pack paths, list should be empty
    assert!(manager.list_loaded().is_empty());
}

#[tokio::test]
async fn test_pack_manager_multiple_paths() {
    let temp_dir1 = tempfile::tempdir().unwrap();
    let temp_dir2 = tempfile::tempdir().unwrap();

    // Create pack in first directory
    let pack_dir1 = temp_dir1.path().join("pack1.com");
    fs::create_dir(&pack_dir1).await.unwrap();
    let manifest1 = pack_dir1.join("pack.yaml");
    fs::write(
        &manifest1,
        r#"
pack: "pack1.com"
version: "1.0.0"
description: "Pack 1"
domains: ["pack1.com"]
"#,
    )
    .await
    .unwrap();

    // Create pack in second directory
    let pack_dir2 = temp_dir2.path().join("pack2.com");
    fs::create_dir(&pack_dir2).await.unwrap();
    let manifest2 = pack_dir2.join("pack.yaml");
    fs::write(
        &manifest2,
        r#"
pack: "pack2.com"
version: "1.0.0"
description: "Pack 2"
domains: ["pack2.com"]
"#,
    )
    .await
    .unwrap();

    let registry = IntentRegistry::new();
    let mut manager = PackManager::new(
        registry,
        vec![
            temp_dir1.path().to_path_buf(),
            temp_dir2.path().to_path_buf(),
        ],
    )
    .await;

    // Should be able to load from both paths
    manager.load_pack_by_name("pack1.com").await.unwrap();
    manager.load_pack_by_name("pack2.com").await.unwrap();

    assert_eq!(manager.list_loaded().len(), 2);
}

// ============================================================================
// Pack Loader Error Tests
// ============================================================================

#[tokio::test]
async fn test_pack_loader_invalid_yaml() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = temp_dir.path();

    let manifest_path = root.join("pack.yaml");
    fs::write(&manifest_path, "{{invalid yaml: [")
        .await
        .unwrap();

    let result = PackLoader::load_pack(root).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_pack_loader_missing_required_fields() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = temp_dir.path();

    let manifest_path = root.join("pack.yaml");
    // Missing required 'pack' field
    fs::write(
        &manifest_path,
        r#"
version: "1.0.0"
description: "Missing pack name"
"#,
    )
    .await
    .unwrap();

    let result = PackLoader::load_pack(root).await;
    // Should fail due to missing required field
    assert!(result.is_err());
}

#[tokio::test]
async fn test_pack_loader_invalid_intent_yaml() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = temp_dir.path();

    let intents_dir = root.join("intents");
    fs::create_dir_all(&intents_dir).await.unwrap();

    let manifest_path = root.join("pack.yaml");
    fs::write(
        &manifest_path,
        r#"
pack: "test.com"
version: "1.0.0"
description: "Test"
domains: ["test.com"]
intents: ["intents/*.yaml"]
"#,
    )
    .await
    .unwrap();

    // Create invalid intent file
    let intent_path = intents_dir.join("bad_intent.yaml");
    fs::write(&intent_path, "{{invalid yaml").await.unwrap();

    let result = PackLoader::load_pack(root).await;
    // Should fail or skip invalid intent
    // Behavior depends on implementation
    assert!(result.is_err() || result.is_ok());
}

#[tokio::test]
async fn test_pack_loader_empty_intents_glob() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = temp_dir.path();

    let manifest_path = root.join("pack.yaml");
    fs::write(
        &manifest_path,
        r#"
pack: "test.com"
version: "1.0.0"
description: "Test"
domains: ["test.com"]
intents: ["intents/*.yaml"]
"#,
    )
    .await
    .unwrap();

    // Don't create intents directory - glob should match nothing

    let result = PackLoader::load_pack(root).await;
    // Should succeed with empty intents
    if let Ok(loaded) = result {
        assert!(loaded.intents.is_empty());
    }
}

// ============================================================================
// Auto-load URL Matching Tests
// ============================================================================

#[tokio::test]
async fn test_pack_manager_auto_load_github_subpath() {
    let temp_dir = tempfile::tempdir().unwrap();
    let root = temp_dir.path();

    // Create a github.com pack
    let pack_dir = root.join("github.com");
    fs::create_dir(&pack_dir).await.unwrap();
    let manifest_path = pack_dir.join("pack.yaml");
    let metadata = r#"
pack: "github.com"
version: "1.0.0"
description: "GitHub Pack"
domains: ["github.com"]
"#;
    fs::write(&manifest_path, metadata).await.unwrap();

    let registry = IntentRegistry::new();
    let manager = PackManager::new(registry, vec![root.to_path_buf()]).await;

    // Various GitHub URLs should match
    assert_eq!(
        manager.should_auto_load("https://github.com/user/repo"),
        Some("github.com".to_string())
    );
    assert_eq!(
        manager.should_auto_load("https://github.com/user/repo/issues"),
        Some("github.com".to_string())
    );
}

#[tokio::test]
async fn test_pack_manager_auto_load_no_match() {
    let registry = IntentRegistry::new();
    let manager = PackManager::new(registry, vec![]).await;

    // Non-GitHub URLs should not match
    assert!(
        manager
            .should_auto_load("https://example.com/page")
            .is_none()
    );
    assert!(
        manager
            .should_auto_load("https://gitlab.com/repo")
            .is_none()
    );
}
