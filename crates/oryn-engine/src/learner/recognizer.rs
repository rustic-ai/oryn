use super::{LearningConfig, SessionLog};

#[derive(Debug, Clone)]
pub struct Pattern {
    pub steps: Vec<String>,
    pub occurrence_count: usize,
    pub domain: String,
}

pub struct Recognizer {
    config: LearningConfig,
}

impl Recognizer {
    pub fn new(config: LearningConfig) -> Self {
        Self { config }
    }

    /// Find recurring patterns in session history.
    ///
    /// Detailed algorithm:
    /// 1. Group logs by domain.
    /// 2. Extract command sequences.
    /// 3. Find varying length N-grams that appear multiple times.
    ///
    /// For MVP: Simplified detection of exact sequence repetition.
    pub fn find_patterns(&self, history: &[SessionLog]) -> Vec<Pattern> {
        if history.is_empty() || !self.config.enabled {
            return vec![];
        }

        let mut patterns = Vec::new();
        let commands: Vec<String> = history.iter().map(|l| l.command.clone()).collect();
        let domain = history[0].domain.clone();

        // Very naive N-gram search for demonstration
        // Try lengths from min_pattern_length to say 5
        let max_len = 5;
        let min_len = self.config.min_pattern_length.max(2);

        for len in min_len..=max_len {
            if commands.len() < len {
                break;
            }

            // Count occurrences of each N-gram
            let mut counts: std::collections::HashMap<Vec<String>, usize> =
                std::collections::HashMap::new();

            for window in commands.windows(len) {
                *counts.entry(window.to_vec()).or_default() += 1;
            }

            for (seq, count) in counts {
                if count >= self.config.min_observations {
                    patterns.push(Pattern {
                        steps: seq,
                        occurrence_count: count,
                        domain: domain.clone(),
                    });
                }
            }
        }

        // Dedup: if pattern A is substring of pattern B and they have same count, keep B?
        // For MVP, just return all.

        patterns
    }
}
