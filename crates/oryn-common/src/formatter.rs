use crate::protocol::{ChangeType, ElementChange, ScannerData, ScannerProtocolResponse};

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

                    // Add value suffix if present
                    let value_suffix = if let Some(ref val) = el.value
                        && !val.is_empty()
                    {
                        let display_val = mask_sensitive(val, &el.element_type, &[]);
                        format!(" = {:?}", display_val)
                    } else if el.element_type == "checkbox" || el.element_type == "radio" {
                        // Show checked state as value
                        if el.state.checked {
                            " = checked".to_string()
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };

                    // Format with or without position data based on full_mode
                    if scan.full_mode {
                        output.push_str(&format!(
                            "[{}] {} {:?} @ ({:.0},{:.0}) {:.0}x{:.0}{}{}\n",
                            el.id,
                            type_str,
                            label,
                            el.rect.x,
                            el.rect.y,
                            el.rect.width,
                            el.rect.height,
                            flags_str,
                            value_suffix
                        ));
                    } else {
                        output.push_str(&format!(
                            "[{}] {} {:?}{}{}\n",
                            el.id, type_str, label, flags_str, value_suffix
                        ));
                    }
                }

                if let Some(patterns) = &scan.patterns {
                    let mut pattern_lines = Vec::new();

                    if let Some(login) = &patterns.login {
                        let conf_pct = (login.confidence * 100.0) as u32;
                        let note = if login.confidence < 0.7 {
                            " (Note: Unusual structure, verify before use)"
                        } else {
                            ""
                        };
                        pattern_lines
                            .push(format!("Login Form ({}% confidence){}", conf_pct, note));
                    }

                    if patterns.search.is_some() {
                        pattern_lines.push("Search Box".to_string());
                    }
                    if patterns.pagination.is_some() {
                        pattern_lines.push("Pagination".to_string());
                    }
                    if patterns.modal.is_some() {
                        pattern_lines.push("Modal".to_string());
                    }
                    if patterns.cookie_banner.is_some() {
                        pattern_lines.push("Cookie Banner".to_string());
                    }

                    if !pattern_lines.is_empty() {
                        output.push_str("\nPatterns:");
                        for p in pattern_lines {
                            output.push_str(&format!("\n- {}", p));
                        }
                    }
                }

                // Format changes if present
                if let Some(changes) = &scan.changes
                    && !changes.is_empty()
                {
                    output.push_str("\n\n# changes\n");
                    for change in changes {
                        output.push_str(&format_change(change));
                    }
                }

                output
            }
            ScannerData::Value(v) => format!("Value: {}", v),
            ScannerData::Action(a) => {
                let mut output = format!("ok {}\n", a.message.as_deref().unwrap_or("action"));

                if let Some(true) = a.navigation {
                    output.push_str("\n# navigation detected\n");
                }

                if let Some(changes) = &a.dom_changes
                    && (changes.added > 0 || changes.removed > 0)
                {
                    output.push_str(&format!(
                        "\n# changes: +{} -{} elements\n",
                        changes.added, changes.removed
                    ));
                }

                if let Some(value) = &a.value {
                    output.push_str(&format!("\n# value: {:?}\n", value));
                }

                output
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

/// Format a single element change for display.
fn format_change(change: &ElementChange) -> String {
    let id = change.id;
    let old = change.old_value.as_deref().unwrap_or("");
    let new = change.new_value.as_deref().unwrap_or("");

    match change.change_type {
        ChangeType::Appeared => format!("+ [{}] appeared: {:?}\n", id, new),
        ChangeType::Disappeared => format!("- [{}] disappeared: {:?}\n", id, old),
        ChangeType::TextChanged => format!("~ [{}] text: {:?} → {:?}\n", id, old, new),
        ChangeType::StateChanged => format!("~ [{}] state changed\n", id),
        ChangeType::PositionChanged => format!("~ [{}] moved\n", id),
    }
}
