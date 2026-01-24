use oryn_engine::intent::definition::{IntentDefinition, IntentTier, IntentTriggers, Step};
use oryn_engine::intent::registry::IntentRegistry;
use oryn_engine::intent::schema::{Validatable, ValidationError};
use oryn_engine::pack::manager::PackManager;
use tempfile::tempdir;

#[test]
fn test_validation_logic() {
    // 1. Invalid Intent (Empty Name)
    let invalid = IntentDefinition {
        name: "".to_string(),
        version: "1.0".to_string(),
        description: None,
        tier: IntentTier::BuiltIn,
        triggers: IntentTriggers::default(),
        parameters: vec![],
        steps: vec![], // Also invalid
        flow: None,
        success: None,
        failure: None,
        options: Default::default(),
    };
    assert!(matches!(
        invalid.validate(),
        Err(ValidationError::EmptyName)
    ));

    // 2. Invalid Intent (No Steps)
    let invalid_steps = IntentDefinition {
        name: "test".to_string(),
        version: "1.0".to_string(),
        description: None,
        tier: IntentTier::BuiltIn,
        triggers: IntentTriggers::default(),
        parameters: vec![],
        steps: vec![],
        flow: None,
        success: None,
        failure: None,
        options: Default::default(),
    };
    assert!(matches!(
        invalid_steps.validate(),
        Err(ValidationError::NoStepsOrFlow)
    ));

    // 3. Valid Intent
    let valid = IntentDefinition {
        name: "test".to_string(),
        version: "1.0".to_string(),
        description: None,
        tier: IntentTier::BuiltIn,
        triggers: IntentTriggers::default(),
        parameters: vec![],
        steps: vec![Step::Checkpoint(
            oryn_engine::intent::definition::CheckpointStepWrapper {
                checkpoint: "start".into(),
            },
        )],
        flow: None,
        success: None,
        failure: None,
        options: Default::default(),
    };
    assert!(valid.validate().is_ok());
}

#[tokio::test]
async fn test_pack_auto_load() {
    // Setup temp directory structure
    let dir = tempdir().unwrap();
    let packs_dir = dir.path().join("packs");
    tokio::fs::create_dir(&packs_dir).await.unwrap();

    // Create a mock pack
    let github_pack = packs_dir.join("github-pack");
    tokio::fs::create_dir(&github_pack).await.unwrap();

    // pack.yaml
    let pack_yaml = r#"
pack: github-pack
version: 1.0.0
description: "GitHub automation"
domains:
  - github.com
trust_level: Verified
intents:
  - "intents/*.yaml"
"#;
    tokio::fs::write(github_pack.join("pack.yaml"), pack_yaml)
        .await
        .unwrap();

    // Initialize PackManager
    let registry = IntentRegistry::new();
    let manager = PackManager::new(registry, vec![packs_dir]).await;

    // Test indexing
    let available = manager.list_available();
    assert_eq!(available.len(), 1);
    assert_eq!(available[0].pack, "github-pack");

    // Test auto-load resolution
    assert_eq!(
        manager.should_auto_load("https://github.com/login"),
        Some("github-pack".to_string())
    );
    assert_eq!(manager.should_auto_load("https://google.com"), None);
}
