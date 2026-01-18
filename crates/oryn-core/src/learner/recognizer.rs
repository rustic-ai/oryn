use crate::command::{Command, LearnAction, Target};
use crate::learner::observer::ActionRecord;

#[derive(Debug, Clone)]
pub struct Pattern {
    pub domain: String,
    pub sequence: Vec<Command>, // Representative sequence (first occurrence)
    pub occurrences: Vec<Vec<Command>>, // All found occurrences (for parameter extraction)
    pub confidence: f64,
    pub observation_count: usize,
}

pub struct Recognizer {
    min_observations: usize,
    #[allow(dead_code)]
    min_confidence: f64,
}

/// A structural key for Commands that ignores variable parameters (like typed text)
/// but preserves structural identity (Command type + Target).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum StructuralKey {
    GoTo, // Ignore URL
    Back,
    Forward,
    Refresh,
    Url,
    Observe,
    Html,
    Text,
    Title,
    Screenshot,
    // Actions: Preserve Target, ignore Options/Text
    Click(StructuralTarget),
    Type(StructuralTarget), // Ignore typed text
    Clear(StructuralTarget),
    Press(String),            // Keys usually structural
    Select(StructuralTarget), // Ignore selected value? Usually value is parameter.
    Check(StructuralTarget),
    Uncheck(StructuralTarget),
    Hover(StructuralTarget),
    Focus(StructuralTarget),
    Scroll(Option<StructuralTarget>),
    Wait,
    Extract,
    Cookies,
    Storage,
    Tabs,
    Submit(StructuralTarget),
    Composite(String), // Login, Search, etc. - Name is structural
    BrowserFeatures,
    Packs,
    Intents,
    Learn(String), // Learn action type is structural
    #[allow(dead_code)]
    Other(String),
}

/// A structural key for targets, allowing Hash/Eq (Target has floats? No, Target is safe).
/// But Target is not Hash because of floats/HashMaps... wait Target def in command.rs
/// Target::Id(usize) -> Safe
/// Target::Text(String) -> Safe
/// Target::Role(String) -> Safe
/// Target::Selector(String) -> Safe
/// Target::Near(Box<Target>) -> Safe recursion
/// Target DOES NOT contain floats or HashMaps currently.
/// So we can derive Hash on Target if we add it.
/// But I reverted Hash on Target.
/// So I must manually map Target to StructuralTarget which implements Hash.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum StructuralTarget {
    Id(usize),
    Text(String),
    Role(String),
    Selector(String),
    Relational(String, Box<StructuralTarget>), // Type + Target
}

impl StructuralTarget {
    fn from(t: &Target) -> Self {
        match t {
            Target::Id(id) => StructuralTarget::Id(*id),
            Target::Text(s) => StructuralTarget::Text(s.clone()),
            Target::Role(s) => StructuralTarget::Role(s.clone()),
            Target::Selector(s) => StructuralTarget::Selector(s.clone()),
            Target::Near { target, .. } => StructuralTarget::Relational(
                "Near".into(),
                Box::new(StructuralTarget::from(target)),
            ),
            Target::Inside { target, .. } => StructuralTarget::Relational(
                "Inside".into(),
                Box::new(StructuralTarget::from(target)),
            ),
            Target::After { target, .. } => StructuralTarget::Relational(
                "After".into(),
                Box::new(StructuralTarget::from(target)),
            ),
            Target::Before { target, .. } => StructuralTarget::Relational(
                "Before".into(),
                Box::new(StructuralTarget::from(target)),
            ),
            Target::Contains { target, .. } => StructuralTarget::Relational(
                "Contains".into(),
                Box::new(StructuralTarget::from(target)),
            ),
        }
    }
}

impl StructuralKey {
    fn from(cmd: &Command) -> Self {
        match cmd {
            Command::GoTo(_) => StructuralKey::GoTo,
            Command::Back => StructuralKey::Back,
            Command::Forward => StructuralKey::Forward,
            Command::Refresh(_) => StructuralKey::Refresh,
            Command::Url => StructuralKey::Url,
            Command::Observe(_) => StructuralKey::Observe,
            Command::Html(_) => StructuralKey::Html,
            Command::Text(_) => StructuralKey::Text,
            Command::Title => StructuralKey::Title,
            Command::Screenshot(_) => StructuralKey::Screenshot,

            Command::Click(t, _) => StructuralKey::Click(StructuralTarget::from(t)),
            Command::Type(t, _, _) => StructuralKey::Type(StructuralTarget::from(t)),
            Command::Clear(t) => StructuralKey::Clear(StructuralTarget::from(t)),
            Command::Press(k, _) => StructuralKey::Press(k.clone()),
            Command::Select(t, _) => StructuralKey::Select(StructuralTarget::from(t)),
            Command::Check(t) => StructuralKey::Check(StructuralTarget::from(t)),
            Command::Uncheck(t) => StructuralKey::Uncheck(StructuralTarget::from(t)),
            Command::Hover(t) => StructuralKey::Hover(StructuralTarget::from(t)),
            Command::Focus(t) => StructuralKey::Focus(StructuralTarget::from(t)),
            Command::Scroll(t_opt, _) => {
                StructuralKey::Scroll(t_opt.as_ref().map(StructuralTarget::from))
            }
            Command::Submit(t) => StructuralKey::Submit(StructuralTarget::from(t)),

            Command::Wait(_, _) => StructuralKey::Wait,
            Command::Extract(_) => StructuralKey::Extract,
            Command::Cookies(_) => StructuralKey::Cookies,
            Command::Storage(_) => StructuralKey::Storage,
            Command::Tabs(_) => StructuralKey::Tabs,

            Command::Login(_, _, _) => StructuralKey::Composite("Login".into()),
            Command::Search(_, _) => StructuralKey::Composite("Search".into()),
            Command::Dismiss(s, _) => StructuralKey::Composite(format!("Dismiss:{}", s)),
            Command::Accept(s, _) => StructuralKey::Composite(format!("Accept:{}", s)),
            Command::ScrollUntil(_, _, _) => StructuralKey::Composite("ScrollUntil".into()),

            Command::Pdf(_) => StructuralKey::BrowserFeatures,
            Command::Packs | Command::PackLoad(_) | Command::PackUnload(_) => StructuralKey::Packs,
            Command::Intents(_)
            | Command::Define(_)
            | Command::Undefine(_)
            | Command::Export(_, _)
            | Command::RunIntent(_, _) => StructuralKey::Intents,

            Command::Learn(l) => {
                let s = match l {
                    LearnAction::Status => "Status",
                    LearnAction::Refine(_) => "Refine",
                    LearnAction::Save(_) => "Save",
                    LearnAction::Ignore(_) => "Ignore",
                };
                StructuralKey::Learn(s.into())
            }
        }
    }
}

impl Recognizer {
    pub fn new(min_observations: usize, min_confidence: f64) -> Self {
        Self {
            min_observations,
            min_confidence,
        }
    }

    /// Identifies repeated patterns in the action history.
    pub fn find_patterns(&self, history: &[ActionRecord]) -> Vec<Pattern> {
        let n = history.len();
        if n < self.min_observations * 2 {
            return Vec::new();
        }

        let mut patterns = Vec::new();

        // Sliding window for sequence length L
        for len in 2..=std::cmp::min(10, n / self.min_observations) {
            let mut counts: std::collections::HashMap<Vec<StructuralKey>, Vec<usize>> =
                std::collections::HashMap::new();

            for i in 0..=n - len {
                let sequence: Vec<StructuralKey> = history[i..i + len]
                    .iter()
                    .map(|r| StructuralKey::from(&r.command))
                    .collect();
                counts.entry(sequence).or_default().push(i);
            }

            for (_seq, indices) in counts {
                if indices.len() >= self.min_observations {
                    let mut overlap_free_count = 0;
                    let mut last_end = 0;
                    let mut occurrences = Vec::new();

                    for &start_idx in &indices {
                        if start_idx >= last_end {
                            overlap_free_count += 1;
                            last_end = start_idx + len;

                            let raw_seq: Vec<Command> = history[start_idx..start_idx + len]
                                .iter()
                                .map(|r| r.command.clone())
                                .collect();
                            occurrences.push(raw_seq);
                        }
                    }

                    if overlap_free_count >= self.min_observations {
                        patterns.push(Pattern {
                            domain: history[0].domain.clone(),
                            sequence: occurrences[0].clone(), // Representative
                            occurrences,
                            confidence: (overlap_free_count as f64)
                                / (n as f64 / len as f64).max(1.0),
                            observation_count: overlap_free_count,
                        });
                    }
                }
            }
        }

        // Sort by length desc, then frequency desc
        patterns.sort_by(|a, b| {
            b.sequence
                .len()
                .cmp(&a.sequence.len())
                .then(b.observation_count.cmp(&a.observation_count))
        });

        patterns
    }
}
