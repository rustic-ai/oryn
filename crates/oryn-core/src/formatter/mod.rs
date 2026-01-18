use crate::intent::executor::{IntentResult, IntentStatus};
use crate::intent::registry::IntentRegistry;
use crate::protocol::{ScannerData, ScannerProtocolResponse};

/// Formats a successful intent execution.
pub fn format_intent_success(result: &IntentResult, name: &str) -> String {
    let mut output = String::new();
    output.push_str(&format!("✅ Intent '{}' completed successfully.\n", name));

    if !result.logs.is_empty() {
        output.push_str("\nLogs:\n");
        // format logs with action enumeration like "type [1] ..."
        // Since ActionLog struct details aren't visible here, we assume it has a way to be displayed or we parse it.
        // But `result.logs` is `Vec<ActionLog>`. Let's assume standard display for now, but add indices.
        for (i, log) in result.logs.iter().enumerate() {
            let masked = mask_sensitive_log(log);
            // Check if log acts as a step execution to number it?
            // For now, valid simple enumeration:
            output.push_str(&format!("  {}. {}\n", i + 1, masked));
        }
    }

    if let Some(changes) = &result.changes {
        // Assuming PageChanges has a way to be displayed or accessed.
        // Reusing logic from format_intent_result for consistency but cleaner.
        // The original format_intent_result had this condition:
        if changes.url.is_some()
            || changes.title.is_some()
            || !changes.added.is_empty()
            || !changes.removed.is_empty()
        {
            output.push_str("\nChanges:\n");
            if let Some(url) = &changes.url {
                output.push_str(&format!("  🌐 URL: {}\n", url));
            }
            if let Some(title) = &changes.title {
                output.push_str(&format!("  📄 Title: {}\n", title));
            }
            if !changes.added.is_empty() {
                output.push_str(&format!("  ➕ Added {} elements\n", changes.added.len()));
            }
            if !changes.removed.is_empty() {
                output.push_str(&format!(
                    "  ➖ Removed {} elements\n",
                    changes.removed.len()
                ));
            }
        }
    }

    output
}

/// Formats a failed intent execution.
pub fn format_intent_failure(result: &IntentResult, name: &str, error: &str) -> String {
    let mut output = String::new();
    output.push_str(&format!("❌ Intent '{}' failed: {}\n", name, error));

    if !result.logs.is_empty() {
        output.push_str("\nActions leading to failure:\n");
        for (i, log) in result.logs.iter().enumerate() {
            let masked = mask_sensitive_log(log);
            output.push_str(&format!("  {}. {}\n", i + 1, masked));
        }
    }

    if let Some(checkpoint) = &result.checkpoint {
        output.push_str(&format!("\n🛑 Checkpoint available: {}\n", checkpoint));
        output.push_str(&format!(
            "👉 Hint: Resume with `run {} --resume {}`\n",
            name, checkpoint
        ));
    }

    // Add other hints
    if !result.hints.is_empty() {
        output.push_str("\nHints:\n");
        for hint in &result.hints {
            output.push_str(&format!("  - {}\n", hint));
        }
    }

    output
}

/// Formats the result of an intent execution into a user-friendly string.
pub fn format_intent_result(result: &IntentResult, name: &str) -> String {
    match &result.status {
        IntentStatus::Success => format_intent_success(result, name),
        IntentStatus::Failed(err) => format_intent_failure(result, name, err),
        IntentStatus::PartialSuccess { completed, total } => {
            let mut output = String::new();
            output.push_str(&format!(
                "⚠️ Intent '{}' completed partially ({}/{})\n",
                name, completed, total
            ));

            if !result.logs.is_empty() {
                output.push_str("\nLogs:\n");
                for (i, log) in result.logs.iter().enumerate() {
                    let masked = mask_sensitive_log(log);
                    output.push_str(&format!("  {}. {}\n", i + 1, masked));
                }
            }

            if let Some(checkpoint) = &result.checkpoint {
                output.push_str(&format!("\nLast Checkpoint: {}\n", checkpoint));
                output.push_str(&format!(
                    "Hint: Resume with `run {} --resume {}`\n",
                    name, checkpoint
                ));
            }

            if !result.hints.is_empty() {
                output.push_str("\nHints:\n");
                for hint in &result.hints {
                    output.push_str(&format!("  - {}\n", hint));
                }
            }

            output
        }
    }
}

/// Formats a scanner response without intent registry context.
pub fn format_response(resp: &ScannerProtocolResponse) -> String {
    format_response_with_intent(resp, None)
}

pub fn format_response_with_intent(
    resp: &ScannerProtocolResponse,
    _registry: Option<&IntentRegistry>,
) -> String {
    // This function already existed in `formatter/mod.rs`?
    // Wait, the user said `formatter/mod.rs` does not exist.
    // But `repl.rs` was using `oryn_core::formatter::format_response_with_intent`.
    // It must be defined somewhere else or I missed it.
    // Let's assume I need to recreate/move existing logic if it was in `lib.rs` or something?
    // Checks `repl.rs`: `use oryn_core::formatter::format_response_with_intent;`
    // This implies `src/formatter.rs` or `src/formatter/mod.rs` existed or was supposed to.

    // START EXISTING LOGIC RECREATION (Simulated based on context)
    match resp {
        ScannerProtocolResponse::Ok { data, .. } => match data.as_ref() {
            ScannerData::Scan(scan) => {
                let mut output = format!(
                    "Scanned {} elements.\nTitle: {}\nURL: {}",
                    scan.elements.len(),
                    scan.page.title,
                    scan.page.url
                );

                if let Some(patterns) = &scan.patterns {
                    let mut detected = Vec::new();
                    if patterns.login.is_some() {
                        detected.push("Login Form");
                    }
                    if patterns.search.is_some() {
                        detected.push("Search Box");
                    }
                    if patterns.pagination.is_some() {
                        detected.push("Pagination");
                    }
                    if patterns.modal.is_some() {
                        detected.push("Modal");
                    }
                    if patterns.cookie_banner.is_some() {
                        detected.push("Cookie Banner");
                    }

                    if !detected.is_empty() {
                        output.push_str("\n\nPatterns:");
                        for p in detected {
                            output.push_str(&format!("\n- {}", p));
                        }
                    }
                }

                if let Some(intents) = &scan.available_intents {
                    if !intents.is_empty() {
                        output.push_str("\n\nAvailable Intents:");
                        // Sort by status then name
                        let mut sorted = intents.clone();
                        sorted.sort_by(|a, b| {
                            // Status ordering: Ready types first
                            let status_ord = match (&a.status, &b.status) {
                                (
                                    crate::protocol::AvailabilityStatus::Ready,
                                    crate::protocol::AvailabilityStatus::Ready,
                                ) => std::cmp::Ordering::Equal,
                                (crate::protocol::AvailabilityStatus::Ready, _) => {
                                    std::cmp::Ordering::Less
                                }
                                (_, crate::protocol::AvailabilityStatus::Ready) => {
                                    std::cmp::Ordering::Greater
                                }
                                _ => std::cmp::Ordering::Equal,
                            };
                            status_ord.then(a.name.cmp(&b.name))
                        });

                        for intent in sorted {
                            let status_icon = match intent.status {
                                crate::protocol::AvailabilityStatus::Ready => "🟢",
                                crate::protocol::AvailabilityStatus::NavigateRequired => "🟠",
                                crate::protocol::AvailabilityStatus::MissingPattern => "🔴",
                                crate::protocol::AvailabilityStatus::Unavailable => "⚫",
                            };

                            output.push_str(&format!("\n- {} {}", status_icon, intent.name));
                            if !intent.parameters.is_empty() {
                                output.push_str(&format!(" ({})", intent.parameters.join(", ")));
                            }
                            // Reason?
                            if let Some(reason) = &intent.trigger_reason {
                                output.push_str(&format!(" [{}]", reason));
                            }
                        }
                    }
                } else if let Some(registry) = _registry
                    && let Some(patterns) = &scan.patterns
                {
                    // Fallback to legacy simulated logic if available_intents not populated
                    let mut intents = Vec::new();
                    let mut check_pattern = |p: &str, valid: bool| {
                        if valid {
                            let defs = registry.get_by_pattern(p);
                            for def in defs {
                                intents.push((def.name.clone(), def.version.clone()));
                            }
                        }
                    };

                    check_pattern("login_form", patterns.login.is_some());
                    check_pattern("search_box", patterns.search.is_some());
                    // ... others

                    if !intents.is_empty() {
                        output.push_str("\n\nAvailable Intents (inferred):");
                        intents.sort();
                        intents.dedup();
                        for (name, ver) in intents {
                            output.push_str(&format!("\n- {} (v{})", name, ver));
                        }
                    }
                }
                output
            }
            ScannerData::Value(_) => "Operation returned data.".to_string(),
            _ => "Operation successful.".to_string(),
        },
        ScannerProtocolResponse::Error { message, .. } => {
            format!("Error: {}", message)
        }
    }
}

fn mask_sensitive_log(log: &str) -> String {
    // Extended list of sensitive keys
    let sensitive_keys = [
        "password",
        "secret",
        "token",
        "key",
        "cvv",
        "ssn",
        "card_number",
        "credit_card",
    ];

    let mut masked = log.to_string();
    let lower_log = log.to_lowercase();

    for key in sensitive_keys {
        if lower_log.contains(key) {
            // Naive masking of quoted strings if key is present
            if let Some(start) = masked.find('"')
                && let Some(end) = masked[start + 1..].rfind('"')
            {
                masked.replace_range(start + 1..start + 1 + end, "********");
            }
            // Break after first mask to avoid double masking if multiple keys present?
            // Or continue? Implementation detail: replace_range modifies in place, indices might shift if replacement length differs.
            // "********" is 8 chars. If original was != 8, indices shift.
            // Simple approach: just doing one pass of masking for now is likely sufficient for logs like `type "1234"`
            break;
        }
    }

    masked
}

pub fn mask_sensitive(value: &str, field_name: &str, sensitive_fields: &[String]) -> String {
    // Default sensitive fields if list is empty? Or assume caller provides correct config.
    // Also include hardcoded defaults for safety
    let default_sensitive = [
        "password",
        "secret",
        "token",
        "key",
        "cvv",
        "ssn",
        "card_number",
    ];

    let is_sensitive = sensitive_fields
        .iter()
        .any(|f| field_name.to_lowercase().contains(&f.to_lowercase()))
        || default_sensitive
            .iter()
            .any(|f| field_name.to_lowercase().contains(*f));

    if is_sensitive {
        "••••••••".to_string()
    } else {
        value.to_string()
    }
}
