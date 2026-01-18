use oryn_core::config::loader::ConfigLoader;
use oryn_core::config::schema::OrynConfig;
use std::io::Write;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_default_config_loading() {
    let config = ConfigLoader::load_default()
        .await
        .expect("Failed to load default config");
    assert_eq!(config.intent_engine.default_timeout_ms, 30000);
    assert_eq!(config.intent_engine.max_retries, 3);
}

#[tokio::test]
async fn test_load_from_file() {
    let mut file = NamedTempFile::new().unwrap();
    writeln!(
        file,
        r#"
intent_engine:
  default_timeout_ms: 15000
  max_retries: 5
security:
  sensitive_fields:
    - "api_key"
    "#
    )
    .unwrap();

    let config = ConfigLoader::load_from(file.path())
        .await
        .expect("Failed to load config from file");

    assert_eq!(config.intent_engine.default_timeout_ms, 15000);
    assert_eq!(config.intent_engine.max_retries, 5);
    // Vector defaults should be replaced or merged?
    // Serde replaces by default for Vec.
    assert_eq!(
        config.security.sensitive_fields,
        vec!["api_key".to_string()]
    );
}

#[test]
fn test_default_values() {
    let config = OrynConfig::default();
    assert_eq!(config.packs.auto_load, true);
    assert!(config.security.redact_in_logs);
}
