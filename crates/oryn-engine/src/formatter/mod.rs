// use crate::intent::executor::{IntentResult, IntentStatus};
// use oryn_common::intent::registry::IntentRegistry;
use oryn_common::protocol::{ScannerData, ScannerProtocolResponse};

// Intent formatting disabled during refactor
/*
pub fn format_intent_success(result: &IntentResult, name: &str) -> String { ... }
pub fn format_intent_failure(result: &IntentResult, name: &str, error: &str) -> String { ... }
pub fn format_intent_result(result: &IntentResult, name: &str) -> String { ... }
*/

/// Formats a scanner response without intent registry context.
pub fn format_response(resp: &ScannerProtocolResponse) -> String {
    // format_response_with_intent(resp, None)
    // Inline simple format since intent registry is disabled
    match resp {
        ScannerProtocolResponse::Ok { data, .. } => match data.as_ref() {
            ScannerData::Scan(scan) => {
                let mut output = format!(
                    "Scanned {} elements.\nTitle: {}\nURL: {}",
                    scan.elements.len(),
                    scan.page.title,
                    scan.page.url
                );
                // Add patterns/intents info if available in scan result directly?
                // scan.patterns is available.
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

                if let Some(intents) = &scan.available_intents
                    && !intents.is_empty()
                {
                    output.push_str("\n\nAvailable Intents:");
                    for intent in intents {
                        output.push_str(&format!("\n- {} ({:?})", intent.name, intent.status));
                    }
                }

                output
            }
            ScannerData::Value(v) => format!("Value: {}", v),
            ScannerData::Action(a) => {
                format!("Action Result: success={}, msg={:?}", a.success, a.message)
            } // _ => "Operation successful".into(),
        },
        ScannerProtocolResponse::Error { message, .. } => format!("Error: {}", message),
    }
}

/*
pub fn format_response_with_intent(...) -> String { ... }
*/

pub fn mask_sensitive_log(log: &str) -> String {
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
            if let Some(start) = masked.find('"')
                && let Some(end) = masked[start + 1..].rfind('"')
            {
                masked.replace_range(start + 1..start + 1 + end, "********");
            }
            break;
        }
    }
    masked
}

pub fn mask_sensitive(value: &str, field_name: &str, sensitive_fields: &[String]) -> String {
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
