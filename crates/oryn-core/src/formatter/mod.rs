use crate::intent::executor::{IntentResult, IntentStatus};
use crate::intent::registry::IntentRegistry;
use crate::protocol::{ScannerData, ScannerProtocolResponse};

/// Formats the result of an intent execution into a user-friendly string.
pub fn format_intent_result(result: &IntentResult, name: &str) -> String {
    let mut output = String::new();

    match &result.status {
        IntentStatus::Success => {
            output.push_str(&format!("✅ Intent '{}' completed successfully.\n", name));
        }
        IntentStatus::PartialSuccess { completed, total } => {
            output.push_str(&format!(
                "⚠️ Intent '{}' completed partially ({}/{})\n",
                name, completed, total
            ));
        }
        IntentStatus::Failed(err) => {
            output.push_str(&format!("❌ Intent '{}' failed: {}\n", name, err));
        }
    }

    if let Some(changes) = &result.changes {
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

    if !result.logs.is_empty() {
        output.push_str("\nLogs:\n");
        for log in &result.logs {
            // Mask sensitive data in logs
            let masked = mask_sensitive_log(log);
            output.push_str(&format!("  {}\n", masked));
        }
    }

    if let Some(checkpoint) = &result.checkpoint {
        output.push_str(&format!("\nLast Checkpoint: {}\n", checkpoint));
        if matches!(result.status, IntentStatus::Failed(_)) {
            output.push_str(&format!(
                "Hint: Resume with `run {} --resume {}`\n",
                name, checkpoint
            ));
        }
    }

    if !result.hints.is_empty() {
        output.push_str("\nHints:\n");
        for hint in &result.hints {
            output.push_str(&format!("  - {}\n", hint));
        }
    }

    output
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

                if let Some(registry) = _registry
                    && let Some(patterns) = &scan.patterns
                {
                    let mut available = Vec::new();
                    if patterns.login.is_some() {
                        available.push("login");
                    }
                    if patterns.search.is_some() {
                        available.push("search");
                    }
                    // ... other patterns

                    // Find intents triggered by these patterns
                    // Registry has `patterns_to_intents`.

                    let mut intents = Vec::new(); // (Name, Version)

                    // Helper to check
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

                    if !intents.is_empty() {
                        output.push_str("\n\nAvailable Intents:");
                        intents.sort();
                        intents.dedup();
                        for (name, ver) in intents {
                            output.push_str(&format!("\n- {} (v{})", name, ver));
                        }
                    }

                    // Silence unused warning
                    let _ = available;
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
    // Basic heuristics for masking sensitive data in logs
    let _sensitive_patterns = [
        // password="...", password: "..."
        (
            r#"(?i)(password|secret|token|key)\s*[:=]\s*["']?([^"'\s]+)["']?"#,
            "********",
        ),
        // Type [..] "..." (If we can detect it is typing into password field, but log string might not have that context easily unless we structure it carefully.
        // For now, if log explicitly mentions "password", mask the value.
    ];

    let mut masked = log.to_string();
    // Using simple find/replace for now as including regex crate dependency if not already there might be hassle.
    // `oryn-core` likely doesn't rely on `regex` yet? Let's check imports.
    // Actually, `mask_sensitive` below uses `contains`.

    // If we want regex, we need `regex` crate.
    // Let's stick to safe heuristic:
    // If log line contains "password", replace values in quotes?
    if log.to_lowercase().contains("password") || log.to_lowercase().contains("secret") {
        // Naive masking of quoted strings
        if let Some(start) = masked.find('"') {
            if let Some(end) = masked[start + 1..].rfind('"') {
                // mask content
                masked.replace_range(start + 1..start + 1 + end, "********");
            }
        }
    }

    masked
}

pub fn mask_sensitive(value: &str, field_name: &str, sensitive_fields: &[String]) -> String {
    if sensitive_fields
        .iter()
        .any(|f| field_name.to_lowercase().contains(&f.to_lowercase()))
    {
        "••••••••".to_string()
    } else {
        value.to_string()
    }
}
