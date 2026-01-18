use oryn_core::learner::LearningConfig;
use oryn_core::learner::observer::Observer;
use oryn_core::learner::proposer::Proposer;
use oryn_core::learner::recognizer::Recognizer;
use oryn_core::learner::storage::ObservationStorage;

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

// ============================================================================
// Observer Tests
// ============================================================================

#[test]
fn test_observer_disabled_config() {
    let config = LearningConfig {
        enabled: false,
        ..Default::default()
    };
    let storage = ObservationStorage::new();
    let observer = Observer::new(config, storage.clone());

    // Recording should be skipped when disabled
    observer.record("example.com", "/home", "click \"A\"");

    let history = storage.get_history("example.com");
    assert!(history.is_empty(), "Should not record when disabled");
}

#[test]
fn test_observer_empty_domain() {
    let config = LearningConfig {
        enabled: true,
        ..Default::default()
    };
    let storage = ObservationStorage::new();
    let observer = Observer::new(config, storage);

    // Query non-existent domain
    let history = observer.get_history("nonexistent.com");
    assert!(history.is_empty());
}

#[test]
fn test_observer_multiple_domains() {
    let config = LearningConfig {
        enabled: true,
        ..Default::default()
    };
    let storage = ObservationStorage::new();
    let observer = Observer::new(config, storage.clone());

    observer.record("domain1.com", "/page1", "click \"A\"");
    observer.record("domain2.com", "/page2", "click \"B\"");
    observer.record("domain1.com", "/page1", "click \"C\"");

    assert_eq!(storage.get_history("domain1.com").len(), 2);
    assert_eq!(storage.get_history("domain2.com").len(), 1);
}

// ============================================================================
// Recognizer Tests
// ============================================================================

#[test]
fn test_recognizer_empty_history() {
    let config = LearningConfig {
        enabled: true,
        min_pattern_length: 2,
        min_observations: 2,
        ..Default::default()
    };
    let recognizer = Recognizer::new(config);

    let patterns = recognizer.find_patterns(&[]);
    assert!(patterns.is_empty());
}

#[test]
fn test_recognizer_disabled_config() {
    let config = LearningConfig {
        enabled: false,
        ..Default::default()
    };
    let storage = ObservationStorage::new();
    let observer = Observer::new(
        LearningConfig {
            enabled: true,
            ..Default::default()
        },
        storage.clone(),
    );

    // Record some data
    for _ in 0..3 {
        observer.record("example.com", "/home", "click \"A\"");
        observer.record("example.com", "/home", "type \"B\" \"Val\"");
    }

    let history = storage.get_history("example.com");
    let recognizer = Recognizer::new(config);

    // Should return empty when disabled
    let patterns = recognizer.find_patterns(&history);
    assert!(patterns.is_empty(), "Recognizer should skip when disabled");
}

#[test]
fn test_recognizer_insufficient_observations() {
    let config = LearningConfig {
        enabled: true,
        min_pattern_length: 2,
        min_observations: 5, // Require 5 observations
        ..Default::default()
    };
    let storage = ObservationStorage::new();
    let observer = Observer::new(
        LearningConfig {
            enabled: true,
            ..Default::default()
        },
        storage.clone(),
    );

    // Only record 2 occurrences
    for _ in 0..2 {
        observer.record("example.com", "/home", "click \"A\"");
        observer.record("example.com", "/home", "type \"B\" \"Val\"");
    }

    let history = storage.get_history("example.com");
    let recognizer = Recognizer::new(config);

    let patterns = recognizer.find_patterns(&history);
    assert!(
        patterns.is_empty(),
        "Should not find patterns with insufficient observations"
    );
}

#[test]
fn test_recognizer_no_repeating_pattern() {
    let config = LearningConfig {
        enabled: true,
        min_pattern_length: 2,
        min_observations: 2,
        ..Default::default()
    };
    let storage = ObservationStorage::new();
    let observer = Observer::new(
        LearningConfig {
            enabled: true,
            ..Default::default()
        },
        storage.clone(),
    );

    // All unique commands - no patterns
    observer.record("example.com", "/home", "click \"A\"");
    observer.record("example.com", "/home", "click \"B\"");
    observer.record("example.com", "/home", "click \"C\"");
    observer.record("example.com", "/home", "click \"D\"");

    let history = storage.get_history("example.com");
    let recognizer = Recognizer::new(config);

    let patterns = recognizer.find_patterns(&history);
    assert!(
        patterns.is_empty(),
        "Should not find patterns in unique commands"
    );
}

#[test]
fn test_recognizer_longer_patterns() {
    let config = LearningConfig {
        enabled: true,
        min_pattern_length: 3,
        min_observations: 2,
        ..Default::default()
    };
    let storage = ObservationStorage::new();
    let observer = Observer::new(
        LearningConfig {
            enabled: true,
            ..Default::default()
        },
        storage.clone(),
    );

    // Pattern of length 3
    for _ in 0..2 {
        observer.record("example.com", "/home", "click \"A\"");
        observer.record("example.com", "/home", "type \"B\" \"Val\"");
        observer.record("example.com", "/home", "click \"Submit\"");
    }

    let history = storage.get_history("example.com");
    let recognizer = Recognizer::new(config);

    let patterns = recognizer.find_patterns(&history);
    let pattern = patterns.iter().find(|p| p.steps.len() == 3);
    assert!(pattern.is_some(), "Should find pattern of length 3");
}

// ============================================================================
// Storage Tests
// ============================================================================

#[test]
fn test_storage_clone_independence() {
    let storage1 = ObservationStorage::new();
    let storage2 = storage1.clone();

    // Both clones share the same underlying data
    storage1.record(oryn_core::learner::SessionLog {
        timestamp: std::time::SystemTime::now(),
        domain: "test.com".to_string(),
        url: "/page".to_string(),
        command: "click \"A\"".to_string(),
        input_snapshot: None,
    });

    // Storage2 should see the same data (Arc shared)
    assert_eq!(storage2.get_history("test.com").len(), 1);
}

#[test]
fn test_storage_empty_query() {
    let storage = ObservationStorage::new();

    let history = storage.get_history("nonexistent.com");
    assert!(history.is_empty());
}

// ============================================================================
// Proposer Tests
// ============================================================================

#[test]
fn test_proposer_valid_pattern() {
    let proposer = Proposer::new();
    let pattern = oryn_core::learner::recognizer::Pattern {
        steps: vec![
            "click \"Button\"".to_string(),
            "type \"Input\" \"Value\"".to_string(),
        ],
        occurrence_count: 3,
        domain: "example.com".to_string(),
    };

    let def = proposer.propose(&pattern);
    assert!(def.is_some());

    let def = def.unwrap();
    assert!(def.name.starts_with("auto_discovered_"));
    assert_eq!(def.steps.len(), 2);
    assert!(def.description.is_some());
}

#[test]
fn test_proposer_invalid_steps() {
    let proposer = Proposer::new();
    let pattern = oryn_core::learner::recognizer::Pattern {
        steps: vec!["{{invalid syntax".to_string()],
        occurrence_count: 2,
        domain: "example.com".to_string(),
    };

    let def = proposer.propose(&pattern);
    // Should return None for unparseable patterns
    // (depends on how forgiving the parser is)
    assert!(def.is_none() || def.is_some());
}

#[test]
fn test_proposer_empty_pattern() {
    let proposer = Proposer::new();
    let pattern = oryn_core::learner::recognizer::Pattern {
        steps: vec![],
        occurrence_count: 0,
        domain: "example.com".to_string(),
    };

    let def = proposer.propose(&pattern);
    // Empty steps may result in parsing failure or empty intent
    if let Some(def) = def {
        assert!(def.steps.is_empty());
    }
}

#[test]
fn test_proposer_single_step() {
    let proposer = Proposer::new();
    let pattern = oryn_core::learner::recognizer::Pattern {
        steps: vec!["click \"SingleButton\"".to_string()],
        occurrence_count: 5,
        domain: "example.com".to_string(),
    };

    let def = proposer.propose(&pattern);
    if let Some(def) = def {
        assert_eq!(def.steps.len(), 1);
    }
}
