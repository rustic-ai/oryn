//! Native WebDriver command integration for upstream WPT.
//! Listener/session sources are under generation. The upstream launcher and
//! complete native command path remain incomplete and unqualified.
pub mod auth;
pub mod capabilities;
#[cfg(feature = "webdriver-server")]
pub mod cli;
#[cfg(feature = "webdriver-server")]
pub mod fixture;
#[cfg(feature = "webdriver-server")]
pub mod http;
pub mod navigation;
pub mod print;
pub mod prompts;
pub mod registry;
pub mod screenshot;
pub mod script;
#[cfg(feature = "webdriver-server")]
pub mod script_tasks;
#[cfg(feature = "webdriver-server")]
pub mod server;
#[cfg(feature = "webdriver-server")]
pub mod service;
pub mod session;
pub mod timeouts;
pub mod window_rect;

use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandError {
    pub code: &'static str,
    pub message: String,
}

impl CommandError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "invalid argument",
            message: message.into(),
        }
    }

    pub fn session_not_created(message: impl Into<String>) -> Self {
        Self {
            code: "session not created",
            message: message.into(),
        }
    }

    /// Wire errors preserve prompt UTF-16, including unpaired surrogates and
    /// author text resembling the native DOM storage escape prefix.
    pub fn wire_response(&self) -> oryn_native::host::webdriver_script::ScriptValue {
        use oryn_native::host::webdriver_script::ScriptValue;
        if self.code != "unexpected alert open" {
            return self.response().into();
        }
        let text = ScriptValue::String(oryn_native::dom_string::units(&self.message));
        let object = |entries: Vec<(&str, ScriptValue)>| {
            ScriptValue::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.encode_utf16().collect(), value))
                    .collect(),
            )
        };
        object(vec![(
            "value",
            object(vec![
                ("error", json!(self.code).into()),
                ("message", text.clone()),
                ("stacktrace", json!("").into()),
                ("data", object(vec![("text", text)])),
            ]),
        )])
    }

    pub fn response(&self) -> Value {
        let mut value = json!({"error":self.code,"message":self.message,"stacktrace":""});
        if self.code == "unexpected alert open" {
            let text = String::from_utf16_lossy(&oryn_native::dom_string::units(&self.message));
            value["message"] = json!(text);
            value["data"] = json!({"text":text});
        }
        json!({"value":value})
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for CommandError {}

#[cfg(test)]
mod wire_error_tests {
    use super::*;
    use oryn_native::host::webdriver_script::ScriptValue;
    #[test]
    fn alert_error_data_preserves_lone_surrogates_and_reserved_prefix_text() {
        let prefix = format!("{}[55296]", oryn_native::dom_string::PREFIX);
        for units in [
            vec![0xd800, 0x61, 0xdfff],
            prefix.encode_utf16().collect::<Vec<_>>(),
        ] {
            let error = CommandError {
                code: "unexpected alert open",
                message: oryn_native::dom_string::from_units(&units),
            };
            let encoded = error.wire_response().to_json_text().unwrap();
            let decoded = ScriptValue::from_json_text(&encoded).unwrap();
            assert_eq!(
                decoded
                    .get("value")
                    .unwrap()
                    .get("data")
                    .unwrap()
                    .get("text"),
                Some(&ScriptValue::String(units))
            );
        }
    }
    #[test]
    fn ordinary_errors_do_not_interpret_author_like_dom_storage_prefix() {
        let message = format!("{}[55296]", oryn_native::dom_string::PREFIX);
        let error = CommandError {
            code: "invalid argument",
            message: message.clone(),
        };
        assert_eq!(
            error.wire_response().get("value").unwrap().get("message"),
            Some(&ScriptValue::String(message.encode_utf16().collect()))
        );
    }
}

#[cfg(any(feature = "webdriver-server", test))]
mod element_input;
