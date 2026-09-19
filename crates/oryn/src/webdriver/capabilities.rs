//! Capability processing before native context creation.
use super::{CommandError, timeouts::Timeouts};
use serde_json::{Map, Value, json};

pub struct NativeIdentity<'a> {
    pub version: &'a str,
    pub platform: &'a str,
}

fn validate(capabilities: &Map<String, Value>) -> Result<(), CommandError> {
    for (name, value) in capabilities {
        if value.is_null() {
            continue;
        }
        let valid = match name.as_str() {
            "browserName" | "browserVersion" | "platformName" => value.is_string(),
            "acceptInsecureCerts"
            | "setWindowRect"
            | "strictFileInteractability"
            | "webSocketUrl" => value.is_boolean(),
            "pageLoadStrategy" => value
                .as_str()
                .is_some_and(|v| matches!(v, "none" | "eager" | "normal")),
            "unhandledPromptBehavior" => {
                super::prompts::PromptPolicy::parse(value)?;
                true
            }
            "timeouts" => {
                let fields = value
                    .as_object()
                    .ok_or_else(|| CommandError::invalid("timeouts must be an object"))?;
                if fields
                    .keys()
                    .any(|key| !matches!(key.as_str(), "script" | "pageLoad" | "implicit"))
                {
                    return Err(CommandError::invalid("unknown timeout capability"));
                }
                Timeouts::default().updated(value)?;
                true
            }
            "proxy" => {
                let proxy = value
                    .as_object()
                    .ok_or_else(|| CommandError::invalid("proxy must be an object"))?;
                // Native policy supports direct routing here. Proxy capability
                // validation is distinct from whether it can be satisfied.
                let kind = proxy.get("proxyType").and_then(Value::as_str);
                kind.is_some_and(|v| {
                    matches!(v, "direct" | "manual" | "pac" | "autodetect" | "system")
                }) && proxy.iter().all(|(key, value)| match key.as_str() {
                    "proxyType" => true,
                    "proxyAutoconfigUrl" | "ftpProxy" | "httpProxy" | "sslProxy" | "socksProxy" => {
                        value.is_string()
                    }
                    "socksVersion" => value.as_u64().is_some_and(|n| n <= 255),
                    "noProxy" => value
                        .as_array()
                        .is_some_and(|items| items.iter().all(Value::is_string)),
                    _ => false,
                })
            }
            _ if name.contains(':') => true,
            _ => false,
        };
        if !valid {
            return Err(CommandError::invalid(format!("invalid capability {name}")));
        }
    }
    Ok(())
}

/// Validate every firstMatch candidate before matching any candidate. A malformed
/// later entry must not be hidden by an earlier compatible one.
pub fn negotiate(
    request: &Value,
    identity: NativeIdentity<'_>,
) -> Result<Map<String, Value>, CommandError> {
    let capabilities = request
        .get("capabilities")
        .and_then(Value::as_object)
        .ok_or_else(|| CommandError::invalid("capabilities must be an object"))?;
    let empty = Map::new();
    let always = match capabilities.get("alwaysMatch") {
        None => &empty,
        Some(value) => value
            .as_object()
            .ok_or_else(|| CommandError::invalid("alwaysMatch must be an object"))?,
    };
    validate(always)?;
    let default_first = vec![json!({})];
    let first = match capabilities.get("firstMatch") {
        None => &default_first,
        Some(value) => value
            .as_array()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| CommandError::invalid("firstMatch must be a nonempty array"))?,
    };
    let mut candidates = Vec::with_capacity(first.len());
    for candidate in first {
        let candidate = candidate
            .as_object()
            .ok_or_else(|| CommandError::invalid("firstMatch entries must be objects"))?;
        validate(candidate)?;
        let mut merged = always.clone();
        for (key, value) in candidate {
            if merged.contains_key(key) {
                return Err(CommandError::invalid(format!("duplicate capability {key}")));
            }
            merged.insert(key.clone(), value.clone());
        }
        merged.retain(|_, value| !value.is_null());
        candidates.push(merged);
    }
    for mut candidate in candidates {
        let identity_matches = [
            ("browserName", "oryn"),
            ("browserVersion", identity.version),
            ("platformName", identity.platform),
        ]
        .iter()
        .all(|(key, actual)| {
            candidate
                .get(*key)
                .and_then(Value::as_str)
                .is_none_or(|v| v.is_empty() || v == *actual)
        });
        if !identity_matches
            || candidate.get("acceptInsecureCerts") == Some(&Value::Bool(true))
            || candidate.get("webSocketUrl") == Some(&Value::Bool(true))
            || candidate.get("proxy").is_some_and(|proxy| {
                proxy.get("proxyType").and_then(Value::as_str) != Some("direct")
            })
        {
            continue;
        }
        // Unknown extension capabilities are ignored, not advertised as native
        // functionality. Validation/merge conflicts still precede matching.
        candidate.retain(|key, _| !key.contains(':'));
        candidate.insert("browserName".into(), json!("oryn"));
        candidate.insert("browserVersion".into(), json!(identity.version));
        candidate.insert("platformName".into(), json!(identity.platform));
        candidate.insert("acceptInsecureCerts".into(), json!(false));
        candidate.insert("setWindowRect".into(), json!(true));
        candidate.insert("userAgent".into(), json!("Oryn/0.1"));
        candidate.entry("proxy").or_insert(json!({}));
        let timeouts = match candidate.get("timeouts") {
            Some(value) => Timeouts::default().updated(value)?,
            None => Timeouts::default(),
        };
        candidate.insert("timeouts".into(), timeouts.to_json());
        candidate
            .entry("pageLoadStrategy")
            .or_insert(json!("normal"));
        candidate
            .entry("strictFileInteractability")
            .or_insert(json!(false));
        candidate
            .entry("unhandledPromptBehavior")
            .or_insert(json!("dismiss and notify"));
        return Ok(candidate);
    }
    Err(CommandError::session_not_created(
        "no capability candidate matches the configured native runtime",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(value: Value) -> Result<Map<String, Value>, CommandError> {
        negotiate(
            &value,
            NativeIdentity {
                version: "candidate-1",
                platform: "mac",
            },
        )
    }
    #[test]
    fn selects_first_native_candidate_without_mutating_input() {
        let request = json!({"capabilities":{"alwaysMatch":{"acceptInsecureCerts":false},"firstMatch":[
            {"browserName":"chrome"},{"browserName":"oryn","platformName":"mac"}]}});
        let original = request.clone();
        let result = run(request.clone()).unwrap();
        assert_eq!(result["browserName"], "oryn");
        assert_eq!(result["browserVersion"], "candidate-1");
        assert_eq!(request, original);
    }
    #[test]
    fn validates_all_candidates_and_merge_conflicts_before_selection() {
        for value in [
            json!({}),
            json!({"capabilities":{"firstMatch":[]}}),
            json!({"capabilities":{"firstMatch":[{},false]}}),
            json!({"capabilities":{"firstMatch":[{},{"acceptInsecureCerts":"false"}]}}),
            json!({"capabilities":{"alwaysMatch":{"browserName":"oryn"},"firstMatch":[{"browserName":"oryn"}]}}),
            json!({"capabilities":{"alwaysMatch":{"timeouts":{"implicit":-1}}}}),
        ] {
            assert_eq!(run(value).unwrap_err().code, "invalid argument");
        }
    }
    #[test]
    fn incompatible_native_policy_is_not_silently_accepted() {
        for capability in [
            json!({"acceptInsecureCerts":true}),
            json!({"webSocketUrl":true}),
            json!({"browserVersion":"other"}),
            json!({"proxy":{"proxyType":"system"}}),
        ] {
            assert_eq!(
                run(json!({"capabilities":{"alwaysMatch":capability}}))
                    .unwrap_err()
                    .code,
                "session not created"
            );
        }
    }
    #[test]
    fn session_timeout_configuration_merges_defaults_and_rejects_unknown_fields() {
        let result = run(
            json!({"capabilities":{"alwaysMatch":{"timeouts":{"pageLoad":null,"implicit":2.0}}}}),
        )
        .unwrap();
        assert_eq!(
            result["timeouts"],
            json!({"script":30000,"pageLoad":null,"implicit":2})
        );
        for timeouts in [
            json!({"unknown":false}),
            json!({"pageLoad":10,"invalid":10}),
        ] {
            assert_eq!(
                run(json!({"capabilities":{"alwaysMatch":{"timeouts":timeouts}}}))
                    .unwrap_err()
                    .code,
                "invalid argument"
            );
        }
        let defaults = run(json!({"capabilities":{}})).unwrap();
        assert_eq!(defaults["timeouts"], Timeouts::default().to_json());
    }
    #[test]
    fn required_native_capability_defaults_and_unknown_extensions() {
        let defaults = run(json!({"capabilities":{}})).unwrap();
        assert_eq!(defaults["proxy"], json!({}));
        assert_eq!(defaults["setWindowRect"], true);
        assert_eq!(defaults["userAgent"], "Oryn/0.1");
        for extension in [
            Value::Null,
            json!(false),
            json!(123),
            json!("abc"),
            json!([]),
            json!({"key":"value"}),
        ] {
            for key in ["alwaysMatch", "firstMatch"] {
                let capabilities = if key == "alwaysMatch" {
                    json!({"alwaysMatch":{"test:extension":extension.clone()}})
                } else {
                    json!({"firstMatch":[{"test:extension":extension.clone()}]})
                };
                let result = run(json!({"capabilities":capabilities})).unwrap();
                assert!(!result.contains_key("test:extension"));
            }
        }
        assert_eq!(run(json!({"capabilities":{"alwaysMatch":{"test:extension":1},"firstMatch":[{"test:extension":2}]}})).unwrap_err().code,"invalid argument");
    }
    #[test]
    fn prompt_objects_are_preserved_and_late_invalid_handlers_reject_negotiation() {
        let policy = json!({"default":"accept","alert":"ignore","beforeUnload":"dismiss"});
        let result =
            run(json!({"capabilities":{"alwaysMatch":{"unhandledPromptBehavior":policy.clone()}}}))
                .unwrap();
        assert_eq!(result["unhandledPromptBehavior"], policy);
        let empty =
            run(json!({"capabilities":{"alwaysMatch":{"unhandledPromptBehavior":{}}}})).unwrap();
        assert_eq!(empty["unhandledPromptBehavior"], json!({}));
        let invalid = run(
            json!({"capabilities":{"firstMatch":[{}, {"unhandledPromptBehavior":{"alert":true}}]}}),
        );
        assert_eq!(invalid.unwrap_err().code, "invalid argument");
    }
    #[test]
    fn null_values_are_removed_and_script_timeout_can_be_null() {
        let result = run(
            json!({"capabilities":{"alwaysMatch":{"browserName":null,"timeouts":{"script":null}}}}),
        )
        .unwrap();
        assert_eq!(result["browserName"], "oryn");
        assert!(result["timeouts"]["script"].is_null());
    }
}
