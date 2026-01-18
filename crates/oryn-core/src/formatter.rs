use crate::protocol::{ScanResult, ScannerData, ScannerProtocolResponse};

pub fn format_response(response: &ScannerProtocolResponse) -> String {
    match response {
        ScannerProtocolResponse::Ok { data, warnings } => {
            let mut output = format_data(data);
            if !warnings.is_empty() {
                output.push_str(&format!("\nWarnings: {:?}", warnings));
            }
            output
        }
        ScannerProtocolResponse::Error {
            code,
            message,
            details,
            hint,
        } => {
            let mut output = format!("ERROR [{}]: {}", code, message);
            if let Some(d) = details {
                output.push_str(&format!(" ({:?})", d));
            }
            if let Some(h) = hint {
                output.push_str(&format!("\n# hint: {}", h));
            }
            output
        }
    }
}

fn format_data(data: &ScannerData) -> String {
    match data {
        ScannerData::Scan(result) => format_scan_result(result),
        ScannerData::ScanValidation(result) => format_scan_result(result),
        ScannerData::Action(result) => {
            if result.success {
                format!("OK {}", result.message.as_deref().unwrap_or(""))
            } else {
                format!(
                    "FAILED {}",
                    result.message.as_deref().unwrap_or("Unknown error")
                )
            }
        }
        ScannerData::Value(v) => format!("Value: {}", v),
    }
}

fn format_scan_result(result: &ScanResult) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "@ {} \"{}\"\n",
        result.page.url, result.page.title
    ));

    for el in &result.elements {
        let type_str = &el.element_type;
        let role_part = el
            .role
            .as_deref()
            .map(|r| format!("/{}", r))
            .unwrap_or_default();
        let text_part = el
            .text
            .as_deref()
            .map(|t| format!(" \"{}\"", t.trim()))
            .unwrap_or_default();

        out.push_str(&format!(
            "[{}] {}{}{}\n",
            el.id, type_str, role_part, text_part
        ));
    }

    out
}
