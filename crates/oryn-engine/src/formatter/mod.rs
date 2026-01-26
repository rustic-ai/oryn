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
                let mut output = format!("@ {} \"{}\"\n", scan.page.url, scan.page.title);

                for el in &scan.elements {
                    // e.g. [1] input/email "Username" {required}
                    let type_str = if let Some(role) = &el.role {
                        format!("{}/{}", el.element_type, role)
                    } else {
                        el.element_type.clone()
                    };

                    let label = el.text.clone().or(el.label.clone()).unwrap_or_default();

                    // Build state flags
                    let mut flags = Vec::new();
                    if el.state.checked {
                        flags.push("checked");
                    }
                    if el.state.selected {
                        flags.push("selected");
                    }
                    if el.state.disabled {
                        flags.push("disabled");
                    }
                    if el.state.readonly {
                        flags.push("readonly");
                    }

                    let flags_str = if flags.is_empty() {
                        String::new()
                    } else {
                        format!(" {{{}}}", flags.join(", "))
                    };

                    output.push_str(&format!(
                        "[{}] {} {:?}{}\n",
                        el.id, type_str, label, flags_str
                    ));
                }

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
                        output.push_str("\nPatterns:");
                        for p in detected {
                            output.push_str(&format!("\n- {}", p));
                        }
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
