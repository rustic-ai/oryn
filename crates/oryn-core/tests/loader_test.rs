use oryn_core::intent::loader::IntentLoader;
use oryn_core::intent::registry::IntentRegistry;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_loader_from_dir() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("test_intent.yaml");

    let yaml_content = r#"
name: test_loaded
version: 1.0.0
tier: loaded
triggers:
  patterns: ["test_pattern"]
steps:
  - checkpoint: "start"
"#;

    fs::write(&file_path, yaml_content).unwrap();

    let mut registry = IntentRegistry::new();
    let count = IntentLoader::load_from_dir(dir.path(), &mut registry).unwrap();

    assert_eq!(count, 1);
    assert!(registry.get("test_loaded").is_some());
}

#[test]
fn test_loader_missing_directory() {
    let mut registry = IntentRegistry::new();
    let path = std::path::Path::new("/non/existent/path/12345");
    // Should return Ok(0) or Err?
    // Implementation says: "Returns number of loaded intents".
    // If dir doesn't exist, read_dir fails.
    // The loader probably propagated io::Error.

    let result = IntentLoader::load_from_dir(path, &mut registry);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0);
}

#[test]
fn test_loader_invalid_yaml() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("invalid.yaml");
    fs::write(&file_path, "invalid: yaml: content: [").unwrap();

    let mut registry = IntentRegistry::new();
    // Loader logs error and continues

    let result = IntentLoader::load_from_dir(dir.path(), &mut registry);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0);
}

#[test]
fn test_loader_multiple_files() {
    let dir = tempdir().unwrap();

    let content1 = r#"
name: intent1
version: 1.0.0
tier: loaded
triggers: {}
steps:
  - checkpoint: "start"
"#;
    let content2 = r#"
name: intent2
version: 1.0.0
tier: loaded
triggers: {}
steps:
  - checkpoint: "start"
"#;

    fs::write(dir.path().join("intent1.yaml"), content1).unwrap();
    fs::write(dir.path().join("intent2.yaml"), content2).unwrap();

    let mut registry = IntentRegistry::new();
    let count = IntentLoader::load_from_dir(dir.path(), &mut registry).unwrap();

    assert_eq!(count, 2);
    assert!(registry.get("intent1").is_some());
    assert!(registry.get("intent2").is_some());
}
