use oryn_core::command::{Command, LearnAction, Target};
use oryn_core::config::schema::SecurityConfig;
use oryn_core::intent::definition::{IntentDefinition, IntentTier};
use oryn_core::learner::observer::Observer;
use oryn_core::learner::proposer::Proposer;
use oryn_core::learner::recognizer::Recognizer;
use oryn_core::learner::storage::LearnerStorage;
use std::collections::HashMap;
use tokio::fs;

#[test]
fn test_observer_ignores_commands() {
    let security = SecurityConfig::default();
    let mut observer = Observer::new(security);

    // Should ignore navigation (no history change)
    observer.observe(
        Command::GoTo("https://example.com".into()),
        "example.com".into(),
    );
    assert!(observer.get_history("example.com").is_none());

    // Should record click
    observer.observe(
        Command::Click(Target::Text("Login".into()), HashMap::new()),
        "example.com".into(),
    );

    // Should ignore Learn command
    observer.observe(Command::Learn(LearnAction::Status), "example.com".into());

    let history = observer
        .get_history("example.com")
        .expect("Should have history");
    assert_eq!(history.len(), 1);
    assert!(matches!(history[0].command, Command::Click(_, _)));
}

#[test]
fn test_recognizer_min_observations() {
    let recognizer = Recognizer::new(3, 0.5);
    let mut observer = Observer::new(SecurityConfig::default());

    // 2 actions (below min 3)
    observer.observe(
        Command::Click(Target::Id(1), HashMap::new()),
        "example.com".into(),
    );
    observer.observe(
        Command::Click(Target::Id(2), HashMap::new()),
        "example.com".into(),
    );

    let history = observer
        .get_history("example.com")
        .expect("Should have history");
    let patterns = recognizer.find_patterns(history);

    // Currently stub returns empty, but logic check: if stub implemented min obs check
    // it should return empty
    assert!(patterns.is_empty());
}

#[test]
fn test_proposer_generates_definition() {
    // Manually create a pattern for testing proposer
    use oryn_core::learner::observer::ActionRecord;
    use oryn_core::learner::recognizer::Pattern;

    let proposer = Proposer;

    // Create 3 records with same structure but different text
    let cmd1 = Command::Type(Target::Id(1), "foo".into(), HashMap::new());
    let cmd2 = Command::Type(Target::Id(1), "bar".into(), HashMap::new());
    let cmd3 = Command::Type(Target::Id(1), "baz".into(), HashMap::new());

    let _rec1 = ActionRecord {
        command: cmd1.clone(),
        domain: "example.com".into(),
        timestamp: 1,
    };
    let _rec2 = ActionRecord {
        command: cmd2.clone(),
        domain: "example.com".into(),
        timestamp: 2,
    };
    let _rec3 = ActionRecord {
        command: cmd3.clone(),
        domain: "example.com".into(),
        timestamp: 3,
    };

    // Pattern found from these 3 occurrences
    let pattern = Pattern {
        sequence: vec![cmd1.clone()], // Representative has "foo"
        occurrences: vec![vec![cmd1], vec![cmd2], vec![cmd3]], // Variations
        domain: "example.com".to_string(),
        observation_count: 3,
        confidence: 1.0,
    };

    let intent = proposer.propose(&pattern);

    assert!(intent.name.starts_with("intent_"));
    assert_eq!(intent.tier, IntentTier::Discovered);
    assert_eq!(intent.triggers.urls, vec!["example.com"]);

    // Check steps
    assert_eq!(intent.steps.len(), 1);

    // Check parameters
    assert_eq!(intent.parameters.len(), 1);
    assert_eq!(intent.parameters[0].name, "param_1");

    // Check if step uses parameter
    if let oryn_core::intent::definition::Step::Action(action_step) = &intent.steps[0] {
        let text_opt = action_step.options.get("text").unwrap();
        assert_eq!(text_opt.as_str().unwrap(), "{{param_1}}");
    } else {
        panic!("Expected ActionStep");
    }
}

#[tokio::test]
async fn test_storage_persistence() {
    let temp = std::env::temp_dir().join("oryn_test_learner");
    if temp.exists() {
        fs::remove_dir_all(&temp).await.unwrap();
    }

    let storage = LearnerStorage::new(temp.clone());

    let intent = IntentDefinition {
        name: "test_intent".to_string(),
        description: Some("Test".to_string()),
        version: "0.1.0".to_string(),
        tier: IntentTier::Discovered,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![],
        success: None,
        failure: None,
        options: Default::default(),
    };

    // Save
    storage.save_intent("example.com", &intent).await.unwrap();

    // Load
    let intents = storage.load_intents("example.com").await.unwrap();
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0].name, "test_intent");

    // Cleanup
    fs::remove_dir_all(&temp).await.unwrap();
}
