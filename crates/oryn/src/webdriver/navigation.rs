//! Navigation request conversion; native lifecycle control owns readiness.
use super::{CommandError, timeouts::Timeouts};
use oryn_native::navigation_control::{
    NavigationCommand, NavigationRequest, NavigationStop, PageLoadStrategy,
};
use serde_json::{Map, Value};

pub fn request(
    command: NavigationCommand,
    capabilities: &Map<String, Value>,
    timeouts: Timeouts,
) -> Result<NavigationRequest, CommandError> {
    let strategy = match capabilities
        .get("pageLoadStrategy")
        .and_then(Value::as_str)
        .unwrap_or("normal")
    {
        "none" => PageLoadStrategy::None,
        "eager" => PageLoadStrategy::Eager,
        "normal" => PageLoadStrategy::Normal,
        _ => return Err(CommandError::invalid("unknown page load strategy")),
    };
    Ok(NavigationRequest {
        command,
        strategy,
        timeout_ms: timeouts.page_load,
    })
}
/// Run after verifying the current top-level window. A detached selected frame
/// does not prevent a top-level Navigate To command from replacing the document.
pub fn navigate_command(value: &Value) -> Result<NavigationCommand, CommandError> {
    let url = value
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::invalid("url must be a string"))?;
    let url =
        url::Url::parse(url).map_err(|_| CommandError::invalid("url must be an absolute URL"))?;
    Ok(NavigationCommand::Navigate { url: url.into() })
}
pub fn error(error: NavigationStop) -> CommandError {
    let code = match &error {
        NavigationStop::Timeout => "timeout",
        NavigationStop::CertificateRejected(_) => "insecure certificate",
        NavigationStop::Retired => "no such window",
        _ => "unknown error",
    };
    CommandError {
        code,
        message: error.to_string(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn navigation_keeps_each_strategy_and_null_page_load_timeout() {
        for (name, expected) in [
            ("none", PageLoadStrategy::None),
            ("eager", PageLoadStrategy::Eager),
            ("normal", PageLoadStrategy::Normal),
        ] {
            let capabilities = json!({"pageLoadStrategy":name})
                .as_object()
                .unwrap()
                .clone();
            let result = request(
                NavigationCommand::Refresh,
                &capabilities,
                Timeouts {
                    page_load: None,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(result.strategy, expected);
            assert_eq!(result.timeout_ms, None);
        }
    }
    #[test]
    fn url_conversion_requires_a_string_and_absolute_url_without_excluding_native_schemes() {
        for value in [
            Value::Null,
            json!({}),
            json!({"url":false}),
            json!({"url":"relative"}),
            json!({"url":"http://["}),
        ] {
            assert_eq!(
                navigate_command(&value).unwrap_err().code,
                "invalid argument"
            );
        }
        for url in [
            "about:blank",
            "data:text/html,hello",
            "http://example.test/",
            "https://example.test/",
            "file:///tmp/page.html",
        ] {
            assert!(navigate_command(&json!({"url":url})).is_ok());
        }
    }
}

#[cfg(test)]
mod certificate_webdriver_tests {
    use super::*;
    #[test]
    fn certificate_navigation_stop_has_exact_webdriver_code_after_wire_roundtrip() {
        let stop = NavigationStop::CertificateRejected("UnknownIssuer".into());
        let stop = serde_json::from_slice(&serde_json::to_vec(&stop).unwrap()).unwrap();
        let mapped = error(stop);
        assert_eq!(mapped.code, "insecure certificate");
        assert_eq!(mapped.response()["value"]["error"], "insecure certificate");
        for stop in [
            NavigationStop::Failed("server certificate rejected".into()),
            NavigationStop::Failed("DNS resolution failed".into()),
        ] {
            assert_eq!(error(stop).code, "unknown error");
        }
        assert_eq!(error(NavigationStop::Timeout).code, "timeout");
        assert_eq!(error(NavigationStop::Retired).code, "no such window");
    }
}
