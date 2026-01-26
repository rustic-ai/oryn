use oryn_common::protocol::{ScannerData, ScannerProtocolResponse};

/// Default sensitive field names that should be masked in output.
const DEFAULT_SENSITIVE_FIELDS: &[&str] = &[
    "password",
    "secret",
    "token",
    "key",
    "cvv",
    "ssn",
    "card_number",
    "credit_card",
];

pub fn format_response(resp: &ScannerProtocolResponse) -> String {
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
            }
        },
        ScannerProtocolResponse::Error { message, .. } => format!("Error: {}", message),
    }
}

pub fn mask_sensitive_log(log: &str) -> String {
    let mut masked = log.to_string();
    let lower_log = log.to_lowercase();

    for key in DEFAULT_SENSITIVE_FIELDS {
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
    let lower_field = field_name.to_lowercase();

    let is_sensitive = sensitive_fields
        .iter()
        .any(|f| lower_field.contains(&f.to_lowercase()))
        || DEFAULT_SENSITIVE_FIELDS
            .iter()
            .any(|f| lower_field.contains(*f));

    if is_sensitive {
        "••••••••".to_string()
    } else {
        value.to_string()
    }
}
