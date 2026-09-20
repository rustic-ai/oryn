//! WebDriver script request conversion; execution stays inside native realms.
use super::CommandError;
use oryn_native::{
    PageHandle,
    host::webdriver_script::{MAX_SCRIPT_BYTES, ScriptError, ScriptRequest, ScriptValue},
};
#[cfg(test)]
use serde_json::Value;

pub fn request(
    value: &ScriptValue,
    asynchronous: bool,
    timeout_ms: Option<u64>,
) -> Result<ScriptRequest, CommandError> {
    let ScriptValue::Object(_) = value else {
        return Err(CommandError::invalid("script parameters must be an object"));
    };
    let Some(ScriptValue::String(body)) = value.get("script") else {
        return Err(CommandError::invalid("script must be a string"));
    };
    if body.len().saturating_mul(2) > MAX_SCRIPT_BYTES {
        return Err(CommandError::invalid("script source exceeds native limit"));
    }
    let Some(ScriptValue::Array(args)) = value.get("args") else {
        return Err(CommandError::invalid("script args must be an array"));
    };
    Ok(ScriptRequest {
        body: body.clone(),
        args: args.clone(),
        asynchronous,
        timeout_ms,
    })
}

pub(crate) fn error(error: ScriptError) -> CommandError {
    let code = match error.code.as_str() {
        "invalid argument" => "invalid argument",
        "invalid selector" => "invalid selector",
        "element click intercepted" => "element click intercepted",
        "javascript error" => "javascript error",
        "script timeout" => "script timeout",
        "stale element reference" => "stale element reference",
        "detached shadow root" => "detached shadow root",
        "no such element" => "no such element",
        "no such shadow root" => "no such shadow root",
        "no such window" => "no such window",
        "no such frame" => "no such frame",
        "move target out of bounds" => "move target out of bounds",
        "element not interactable" => "element not interactable",
        "unsupported operation" => "unsupported operation",
        _ => "unknown error",
    };
    CommandError {
        code,
        message: error.message,
    }
}
pub async fn execute(
    page: &PageHandle,
    context: oryn_native::host::webdriver_script::ScriptContext,
    value: &ScriptValue,
    asynchronous: bool,
    timeout_ms: Option<u64>,
) -> Result<ScriptValue, CommandError> {
    page.execute_webdriver_script_in_context(request(value, asynchronous, timeout_ms)?, context)
        .await
        .map_err(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn native_element_error_codes_are_not_collapsed_into_unknown_error() {
        for code in [
            "invalid selector",
            "element click intercepted",
            "element not interactable",
            "stale element reference",
            "no such shadow root",
        ] {
            let converted = error(ScriptError {
                code: code.into(),
                message: "native detail".into(),
            });
            assert_eq!(converted.code, code);
            assert_eq!(converted.message, "native detail");
        }
    }
    #[test]
    fn script_arguments_preserve_json_and_timeout_mode() {
        let value =
            json!({"script":"return arguments[0]","args":[{"__proto__":{"x":1}},null,[true,4]]});
        let request = request(&value.clone().into(), true, None).unwrap();
        assert_eq!(
            request.args,
            value["args"]
                .as_array()
                .unwrap()
                .iter()
                .cloned()
                .map(ScriptValue::from)
                .collect::<Vec<_>>()
        );
        assert!(request.asynchronous);
        assert_eq!(request.timeout_ms, None);
    }
    #[test]
    fn malformed_parameters_never_reach_the_native_engine() {
        for value in [
            Value::Null,
            json!({}),
            json!({"script":false,"args":[]}),
            json!({"script":"return 1","args":{}}),
            json!({"script":"return 1"}),
        ] {
            assert_eq!(
                request(&value.into(), false, Some(0)).unwrap_err().code,
                "invalid argument"
            );
        }
    }
}
