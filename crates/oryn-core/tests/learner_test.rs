use oryn_core::learner::observer::Observer;
use oryn_core::learner::proposer::Proposer;
use oryn_core::learner::recognizer::Recognizer;
use oryn_core::learner::storage::ObservationStorage;
use oryn_core::learner::{LearningConfig, SessionLog};
use std::time::SystemTime;

#[test]
fn test_learner_pipeline() {
    let config = LearningConfig {
        enabled: true,
        min_pattern_length: 2,
        min_observations: 2,
        ..Default::default()
    };
    let storage = ObservationStorage::new();
    let observer = Observer::new(config.clone(), storage.clone());

    // Simulate user actions
    // Pattern: Click A -> Type B
    for _ in 0..3 {
        observer.record("example.com", "/home", "click \"A\"");
        observer.record("example.com", "/home", "type \"B\" \"Val\"");
    }

    // Check storage
    let history = storage.get_history("example.com");
    assert_eq!(history.len(), 6);

    // Recognize
    let recognizer = Recognizer::new(config);
    let patterns = recognizer.find_patterns(&history);

    // We expect at least one pattern of length 2 ("click A", "type B") appearing 3 times
    let pattern = patterns
        .iter()
        .find(|p| p.steps.len() == 2 && p.occurrence_count == 3);
    assert!(pattern.is_some(), "Pattern not found: {:?}", patterns);

    // Propose
    let proposer = Proposer::new();
    let def = proposer.propose(pattern.unwrap()).expect("Proposal failed");

    assert_eq!(def.steps.len(), 2);
    // Name is auto-generated
    assert!(def.name.starts_with("auto_discovered_"));
}
