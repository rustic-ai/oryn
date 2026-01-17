//! Error Code Mapping
//!
//! Maps scanner protocol error codes (from JavaScript) to Rust BackendError variants.
//! This ensures consistent error handling aligned with SPEC-SCANNER-PROTOCOL.md.

use crate::backend::BackendError;
use serde_json::Value;

/// Maps a scanner protocol error code and message to a BackendError.
///
/// # Arguments
/// * `code` - The error code string from the scanner (e.g., "ELEMENT_NOT_FOUND")
/// * `message` - The human-readable error message
/// * `details` - Optional additional details as JSON value
///
/// # Returns
/// A BackendError variant that best matches the scanner error code.
pub fn map_scanner_error(code: &str, message: &str, details: Option<&Value>) -> BackendError {
    match code {
        "ELEMENT_NOT_FOUND" => {
            let id = extract_id(details);
            BackendError::ElementNotFound { id }
        }
        "ELEMENT_STALE" => {
            let id = extract_id(details);
            BackendError::ElementStale { id }
        }
        "ELEMENT_NOT_VISIBLE" => {
            let id = extract_id(details);
            BackendError::ElementNotVisible { id }
        }
        "ELEMENT_DISABLED" => {
            let id = extract_id(details);
            BackendError::ElementDisabled { id }
        }
        "ELEMENT_NOT_INTERACTABLE" => {
            let id = extract_id(details);
            BackendError::ElementNotInteractable {
                id,
                reason: message.to_string(),
            }
        }
        "INVALID_ELEMENT_TYPE" => {
            let expected = details
                .and_then(|d| d.get("expected"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let got = details
                .and_then(|d| d.get("got"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            BackendError::InvalidElementType { expected, got }
        }
        "OPTION_NOT_FOUND" => {
            let value = details
                .and_then(|d| d.get("value"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            BackendError::OptionNotFound { value }
        }
        "SELECTOR_INVALID" => {
            let selector = details
                .and_then(|d| d.get("selector"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            BackendError::SelectorInvalid { selector }
        }
        "SCRIPT_ERROR" => BackendError::ScriptError(message.to_string()),
        "TIMEOUT" => BackendError::TimeoutWithContext {
            operation: message.to_string(),
        },
        "NAVIGATION_ERROR" => BackendError::Navigation(message.to_string()),
        "UNKNOWN_COMMAND" => BackendError::UnknownCommand(message.to_string()),
        "INVALID_REQUEST" => BackendError::InvalidRequest(message.to_string()),
        "INTERNAL_ERROR" => BackendError::Other(message.to_string()),
        // Scanner-specific codes not in spec
        "PATTERN_NOT_FOUND" | "NOT_FOUND" => BackendError::Other(message.to_string()),
        "INVALID_PARAMS" => BackendError::InvalidRequest(message.to_string()),
        // Fallback for unknown codes
        _ => BackendError::Other(format!("[{}] {}", code, message)),
    }
}

/// Extract element ID from details JSON.
fn extract_id(details: Option<&Value>) -> u32 {
    details
        .and_then(|d| d.get("id"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_map_element_not_found() {
        let details = json!({"id": 42});
        let err = map_scanner_error("ELEMENT_NOT_FOUND", "Element 42 not found", Some(&details));
        assert!(matches!(err, BackendError::ElementNotFound { id: 42 }));
        assert_eq!(err.code(), "ELEMENT_NOT_FOUND");
    }

    #[test]
    fn test_map_element_stale() {
        let details = json!({"id": 5});
        let err = map_scanner_error("ELEMENT_STALE", "Element removed from DOM", Some(&details));
        assert!(matches!(err, BackendError::ElementStale { id: 5 }));
        assert_eq!(err.code(), "ELEMENT_STALE");
    }

    #[test]
    fn test_map_element_not_visible() {
        let details = json!({"id": 10});
        let err = map_scanner_error("ELEMENT_NOT_VISIBLE", "Element is hidden", Some(&details));
        assert!(matches!(err, BackendError::ElementNotVisible { id: 10 }));
    }

    #[test]
    fn test_map_element_disabled() {
        let details = json!({"id": 7});
        let err = map_scanner_error("ELEMENT_DISABLED", "Element is disabled", Some(&details));
        assert!(matches!(err, BackendError::ElementDisabled { id: 7 }));
    }

    #[test]
    fn test_map_element_not_interactable() {
        let details = json!({"id": 3});
        let err = map_scanner_error(
            "ELEMENT_NOT_INTERACTABLE",
            "Element is covered",
            Some(&details),
        );
        match err {
            BackendError::ElementNotInteractable { id, reason } => {
                assert_eq!(id, 3);
                assert_eq!(reason, "Element is covered");
            }
            _ => panic!("Expected ElementNotInteractable"),
        }
    }

    #[test]
    fn test_map_invalid_element_type() {
        let details = json!({"expected": "select", "got": "input"});
        let err = map_scanner_error("INVALID_ELEMENT_TYPE", "Wrong element type", Some(&details));
        match err {
            BackendError::InvalidElementType { expected, got } => {
                assert_eq!(expected, "select");
                assert_eq!(got, "input");
            }
            _ => panic!("Expected InvalidElementType"),
        }
    }

    #[test]
    fn test_map_option_not_found() {
        let details = json!({"value": "Option A"});
        let err = map_scanner_error("OPTION_NOT_FOUND", "Option not found", Some(&details));
        match err {
            BackendError::OptionNotFound { value } => {
                assert_eq!(value, "Option A");
            }
            _ => panic!("Expected OptionNotFound"),
        }
    }

    #[test]
    fn test_map_selector_invalid() {
        let details = json!({"selector": ".invalid[["});
        let err = map_scanner_error("SELECTOR_INVALID", "Invalid CSS selector", Some(&details));
        match err {
            BackendError::SelectorInvalid { selector } => {
                assert_eq!(selector, ".invalid[[");
            }
            _ => panic!("Expected SelectorInvalid"),
        }
    }

    #[test]
    fn test_map_timeout() {
        let err = map_scanner_error("TIMEOUT", "Wait condition timed out", None);
        match &err {
            BackendError::TimeoutWithContext { operation } => {
                assert_eq!(operation, "Wait condition timed out");
            }
            _ => panic!("Expected TimeoutWithContext"),
        }
        assert_eq!(err.code(), "TIMEOUT");
    }

    #[test]
    fn test_map_script_error() {
        let err = map_scanner_error("SCRIPT_ERROR", "ReferenceError: x is not defined", None);
        match err {
            BackendError::ScriptError(msg) => {
                assert!(msg.contains("ReferenceError"));
            }
            _ => panic!("Expected ScriptError"),
        }
    }

    #[test]
    fn test_map_navigation_error() {
        let err = map_scanner_error("NAVIGATION_ERROR", "Page load timeout", None);
        match err {
            BackendError::Navigation(msg) => {
                assert_eq!(msg, "Page load timeout");
            }
            _ => panic!("Expected Navigation"),
        }
    }

    #[test]
    fn test_map_unknown_code_fallback() {
        let err = map_scanner_error("SOME_NEW_CODE", "Something happened", None);
        match err {
            BackendError::Other(msg) => {
                assert!(msg.contains("SOME_NEW_CODE"));
                assert!(msg.contains("Something happened"));
            }
            _ => panic!("Expected Other"),
        }
    }

    #[test]
    fn test_recovery_hints() {
        let err = BackendError::ElementNotFound { id: 1 };
        assert!(err.recovery_hint().contains("scan"));

        let err = BackendError::ElementNotVisible { id: 1 };
        assert!(err.recovery_hint().contains("Scroll"));

        let err = BackendError::Timeout;
        assert!(err.recovery_hint().contains("timeout"));
    }
}
