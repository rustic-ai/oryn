use crate::intent::definition::{
    ActionStep, ActionType, IntentDefinition, IntentOptions, IntentTier, IntentTriggers, MatchType,
    Step, TargetKind, TargetSpec, TryDef, TryStepWrapper,
};
use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("Syntax error: {0}")]
    Syntax(String),
    #[error("Missing block: {0}")]
    MissingBlock(String),
}

/// Parses simplified "define" syntax into an IntentDefinition.
///
/// Syntax Example:
/// define intent_name:
///   description: "Description"
///   steps:
///     - click "Button"
///     - type "Input" "Value"
pub fn parse_define(input: &str) -> Result<IntentDefinition, ParseError> {
    // Basic line-based parsing for MVP
    let mut lines = input.lines().map(|s| s.trim()).filter(|s| !s.is_empty());

    // 1. Header: "define <name>:"
    let header = lines
        .next()
        .ok_or(ParseError::Syntax("Empty input".into()))?;
    if !header.starts_with("define ") || !header.ends_with(':') {
        return Err(ParseError::Syntax(
            "Expected 'define <name>:' header".into(),
        ));
    }

    let name = header
        .trim_start_matches("define ")
        .trim_end_matches(':')
        .trim()
        .to_string();

    let mut steps = Vec::new();
    let mut description = String::new();

    // State machine for blocks
    let mut current_block = ""; // "description", "steps"

    for line in lines {
        if line.starts_with("description:") {
            description = line
                .trim_start_matches("description:")
                .trim()
                .trim_matches('"')
                .to_string();
            current_block = "description";
        } else if line.starts_with("steps:") {
            current_block = "steps";
        } else if line.starts_with("- ") && current_block == "steps" {
            let cmd = line.trim_start_matches("- ").trim();
            steps.push(parse_step_shorthand(cmd)?);
        }
    }

    Ok(IntentDefinition {
        name,
        description: Some(description),
        version: "1.0".to_string(), // Default version
        tier: IntentTier::Discovered,
        triggers: IntentTriggers {
            keywords: vec![], // Parse keywords later?
            ..Default::default()
        },
        parameters: vec![], // TODO: Support params
        steps,
        success: None,
        failure: None,
        options: IntentOptions {
            timeout: 30000,
            ..Default::default()
        },
    })
}

fn parse_step_shorthand(cmd: &str) -> Result<Step, ParseError> {
    // Check for " or " sequence for fallbacks
    // Use recursive implementation.
    // "A or B or C" -> Try { steps: [A], catch: [Try { steps: [B], catch: [C] }] }

    // Naive split by " or ". Note: strings might contain " or ", but simplified syntax usually quotes args.
    // We assume " or " outside quotes is the separator.
    // For now, simpler split.
    let parts: Vec<&str> = cmd.split(" or ").collect();
    if parts.len() > 1 {
        let first = parse_single_step(parts[0].trim())?;
        let remaining = parts[1..].join(" or ");
        let fallback = parse_step_shorthand(&remaining)?;

        return Ok(Step::Try(TryStepWrapper {
            try_: TryDef {
                steps: vec![first],
                catch: vec![fallback],
            },
        }));
    }

    parse_single_step(cmd)
}

fn parse_single_step(cmd: &str) -> Result<Step, ParseError> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() {
        return Err(ParseError::Syntax("Empty step".into()));
    }

    match parts[0] {
        "click" => {
            // click "Target"
            let target = parse_quoted_arg(cmd)
                .ok_or(ParseError::Syntax("Missing target for click".into()))?;
            Ok(Step::Action(ActionStep {
                action: ActionType::Click,
                target: Some(TargetSpec {
                    kind: TargetKind::Text {
                        text: target,
                        match_type: MatchType::Contains,
                    },
                    fallback: None,
                }),
                options: HashMap::new(),
            }))
        }
        "type" => {
            // type "Target" "Value" OR type role "Value"
            // Split by whitespace first to check for "type role ..."
            // But we need to handle quotes properly.

            // Heuristic: If first arg is NOT quoted, it might be a role (like 'email', 'phone')
            // If it is quoted, it matches text.

            // Re-parse logic:
            // 1. Check if first arg starts with quote.
            let rest_cmd = cmd.trim_start_matches("type").trim();
            if rest_cmd.starts_with('"') {
                // Old behavior: type "Target" "Value"
                let (target, rest) = parse_quoted_arg_with_rest(cmd)
                    .ok_or(ParseError::Syntax("Missing target for type".into()))?;
                let value = parse_quoted_arg(&rest)
                    .ok_or(ParseError::Syntax("Missing value for type".into()))?;

                let mut options = HashMap::new();
                options.insert("text".to_string(), serde_json::Value::String(value));

                Ok(Step::Action(ActionStep {
                    action: ActionType::Type,
                    target: Some(TargetSpec {
                        kind: TargetKind::Text {
                            text: target,
                            match_type: MatchType::Contains,
                        },
                        fallback: None,
                    }),
                    options,
                }))
            } else {
                // Potential role syntax: type email "value"
                // Split by space to get role
                let parts: Vec<&str> = rest_cmd.splitn(2, ' ').collect();
                if parts.len() < 2 {
                    return Err(ParseError::Syntax(
                        "Invalid type syntax: expected role and value".into(),
                    ));
                }
                let role = parts[0];
                let value_part = parts[1];
                let value = parse_quoted_arg(value_part)
                    .ok_or(ParseError::Syntax("Missing value for type".into()))?;

                let mut options = HashMap::new();
                options.insert("text".to_string(), serde_json::Value::String(value));

                Ok(Step::Action(ActionStep {
                    action: ActionType::Type,
                    target: Some(TargetSpec {
                        kind: TargetKind::Role {
                            role: role.to_string(),
                        },
                        fallback: None,
                    }),
                    options,
                }))
            }
        }
        "wait" => {
            // wait visible "Target"
            if parts.len() < 3 {
                return Err(ParseError::Syntax("Invalid wait syntax".into()));
            }
            let condition = parts[1]; // visible, hidden
            let target = parse_quoted_arg(cmd)
                .ok_or(ParseError::Syntax("Missing target for wait".into()))?;

            let mut options = HashMap::new();
            options.insert(
                "condition".to_string(),
                serde_json::Value::String(condition.to_string()),
            );

            Ok(Step::Action(ActionStep {
                action: ActionType::Wait,
                target: Some(TargetSpec {
                    kind: TargetKind::Text {
                        text: target,
                        match_type: MatchType::Contains,
                    },
                    fallback: None,
                }),
                options,
            }))
        }
        _ => Err(ParseError::Syntax(format!("Unknown action: {}", parts[0]))),
    }
}

/// Extracts content inside first pair of quotes.
fn parse_quoted_arg(s: &str) -> Option<String> {
    let start = s.find('"')?;
    let end = s[start + 1..].find('"')?;
    Some(s[start + 1..start + 1 + end].to_string())
}

/// Extracts content inside first pair of quotes and returns the rest of the string.
fn parse_quoted_arg_with_rest(s: &str) -> Option<(String, String)> {
    let start = s.find('"')?;
    let end = s[start + 1..].find('"')?;
    let content = s[start + 1..start + 1 + end].to_string();
    let rest = s[start + 1 + end + 1..].to_string();
    Some((content, rest))
}
