//! Validated WebDriver user-prompt policy. Native prompt state is authoritative.
use super::CommandError;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptKind {
    Alert,
    BeforeUnload,
    Confirm,
    Prompt,
}
impl PromptKind {
    fn key(self) -> &'static str {
        match self {
            Self::Alert => "alert",
            Self::BeforeUnload => "beforeUnload",
            Self::Confirm => "confirm",
            Self::Prompt => "prompt",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptAction {
    Accept,
    Dismiss,
    Ignore,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromptHandler {
    pub action: PromptAction,
    pub notify: bool,
}
impl PromptHandler {
    fn parse(value: &Value) -> Result<Self, CommandError> {
        let (action, notify) = match value.as_str() {
            Some("accept") => (PromptAction::Accept, false),
            Some("dismiss") => (PromptAction::Dismiss, false),
            Some("accept and notify") => (PromptAction::Accept, true),
            Some("dismiss and notify") => (PromptAction::Dismiss, true),
            Some("ignore") => (PromptAction::Ignore, true),
            _ => return Err(CommandError::invalid("invalid user prompt handler")),
        };
        Ok(Self { action, notify })
    }
}

/// Preserve the negotiated representation while resolving each native prompt's
/// handler. An absent per-type entry inherits default, then dismiss and notify.
/// Classic WebDriver always accepts beforeunload prompts.
#[derive(Clone, Debug)]
pub struct PromptPolicy {
    capability: Value,
}
impl Default for PromptPolicy {
    fn default() -> Self {
        Self {
            capability: Value::String("dismiss and notify".into()),
        }
    }
}
impl PromptPolicy {
    pub fn parse(value: &Value) -> Result<Self, CommandError> {
        match value {
            Value::String(_) => {
                PromptHandler::parse(value)?;
            }
            Value::Object(handlers) => {
                for (kind, handler) in handlers {
                    if !matches!(
                        kind.as_str(),
                        "alert" | "beforeUnload" | "confirm" | "default" | "prompt"
                    ) {
                        return Err(CommandError::invalid(format!(
                            "unknown user prompt type {kind}"
                        )));
                    }
                    PromptHandler::parse(handler)?;
                }
            }
            _ => {
                return Err(CommandError::invalid(
                    "unhandledPromptBehavior must be a string or object",
                ));
            }
        }
        Ok(Self {
            capability: value.clone(),
        })
    }
    pub fn capability(&self) -> &Value {
        &self.capability
    }
    pub fn handler(&self, kind: PromptKind) -> PromptHandler {
        if kind == PromptKind::BeforeUnload {
            return PromptHandler {
                action: PromptAction::Accept,
                notify: false,
            };
        }
        let value = match &self.capability {
            Value::Object(handlers) => handlers.get(kind.key()).or_else(|| handlers.get("default")),
            value => Some(value),
        };
        value
            .and_then(|value| PromptHandler::parse(value).ok())
            .unwrap_or(PromptHandler {
                action: PromptAction::Dismiss,
                notify: true,
            })
    }
}

fn native_error(error: oryn_native::user_prompts::PromptError) -> CommandError {
    use oryn_native::user_prompts::PromptError;
    let code = match error {
        PromptError::NoPrompt | PromptError::StalePrompt => "no such alert",
        PromptError::WrongKind => "element not interactable",
        PromptError::Retired => "no such window",
        _ => "unknown error",
    };
    CommandError {
        code,
        message: format!("native user prompt: {error:?}"),
    }
}
fn controller(
    page: &oryn_native::PageHandle,
) -> Result<std::sync::Arc<oryn_native::user_prompts::PromptController>, CommandError> {
    page.prompt_controller().map_err(|error| CommandError {
        code: "no such window",
        message: error.to_string(),
    })
}
fn pending(
    controller: &oryn_native::user_prompts::PromptController,
) -> Result<oryn_native::user_prompts::PendingPrompt, CommandError> {
    controller
        .inspect()
        .map_err(native_error)?
        .ok_or(CommandError {
            code: "no such alert",
            message: "no user prompt is open".into(),
        })
}
/// These operations use the parent controller directly, without taking the native
/// actor or worker command lock held by the suspended JavaScript call.
pub fn text(
    page: &oryn_native::PageHandle,
) -> Result<oryn_native::host::webdriver_script::ScriptValue, CommandError> {
    Ok(oryn_native::host::webdriver_script::ScriptValue::String(
        oryn_native::dom_string::units(&pending(&controller(page)?)?.request.text),
    ))
}
pub fn accept(page: &oryn_native::PageHandle) -> Result<(), CommandError> {
    let controller = controller(page)?;
    let prompt = pending(&controller)?;
    controller
        .accept(prompt.request.token)
        .map_err(native_error)
}
pub fn dismiss(page: &oryn_native::PageHandle) -> Result<(), CommandError> {
    let controller = controller(page)?;
    let prompt = pending(&controller)?;
    controller
        .dismiss(prompt.request.token)
        .map_err(native_error)
}
pub fn set_text(
    page: &oryn_native::PageHandle,
    value: &oryn_native::host::webdriver_script::ScriptValue,
) -> Result<(), CommandError> {
    use oryn_native::host::webdriver_script::ScriptValue;
    let Some(ScriptValue::String(input)) = value.get("text") else {
        return Err(CommandError::invalid("prompt text must be a string"));
    };
    let controller = controller(page)?;
    let prompt = pending(&controller)?;
    controller
        .set_input(
            prompt.request.token,
            oryn_native::dom_string::from_units(input),
        )
        .map_err(native_error)
}

/// Apply the negotiated handler before a command that handles user prompts.
/// Returns the exact prompt text when WebDriver must notify the client, after
/// resolving accept/dismiss where requested. Ignore leaves the prompt pending.
pub fn handle_pending(
    page: &oryn_native::PageHandle,
    policy: &PromptPolicy,
) -> Result<Option<String>, CommandError> {
    use oryn_native::user_prompts::PromptKind as NativeKind;
    let controller = controller(page)?;
    let Some(prompt) = controller.inspect().map_err(native_error)? else {
        return Ok(None);
    };
    let kind = match prompt.request.kind {
        NativeKind::Alert => PromptKind::Alert,
        NativeKind::Confirm => PromptKind::Confirm,
        NativeKind::Prompt => PromptKind::Prompt,
        NativeKind::BeforeUnload => PromptKind::BeforeUnload,
    };
    let handler = policy.handler(kind);
    match handler.action {
        PromptAction::Accept => controller
            .accept(prompt.request.token)
            .map_err(native_error)?,
        PromptAction::Dismiss => controller
            .dismiss(prompt.request.token)
            .map_err(native_error)?,
        PromptAction::Ignore => {}
    }
    Ok(handler.notify.then_some(prompt.request.text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn per_type_overrides_default_without_changing_negotiated_object() {
        let value =
            json!({"default":"accept", "alert":"ignore", "beforeUnload":"dismiss and notify"});
        let policy = PromptPolicy::parse(&value).unwrap();
        assert_eq!(policy.capability(), &value);
        assert_eq!(
            policy.handler(PromptKind::Alert),
            PromptHandler {
                action: PromptAction::Ignore,
                notify: true
            }
        );
        assert_eq!(
            policy.handler(PromptKind::Confirm),
            PromptHandler {
                action: PromptAction::Accept,
                notify: false
            }
        );
        assert_eq!(
            policy.handler(PromptKind::BeforeUnload),
            PromptHandler {
                action: PromptAction::Accept,
                notify: false
            }
        );
    }
    #[test]
    fn empty_object_inherits_default_and_legacy_strings_cover_all_prompt_types() {
        let empty = PromptPolicy::parse(&json!({})).unwrap();
        for kind in [PromptKind::Alert, PromptKind::Confirm, PromptKind::Prompt] {
            assert_eq!(
                empty.handler(kind),
                PromptHandler {
                    action: PromptAction::Dismiss,
                    notify: true
                }
            );
            for (name, action, notify) in [
                ("accept", PromptAction::Accept, false),
                ("dismiss", PromptAction::Dismiss, false),
                ("accept and notify", PromptAction::Accept, true),
                ("dismiss and notify", PromptAction::Dismiss, true),
                ("ignore", PromptAction::Ignore, true),
            ] {
                assert_eq!(
                    PromptPolicy::parse(&json!(name)).unwrap().handler(kind),
                    PromptHandler { action, notify }
                );
            }
        }
    }
    #[test]
    fn malformed_handlers_are_rejected_before_session_creation() {
        for value in [
            json!(null),
            json!(false),
            json!([]),
            json!("unknown"),
            json!({"alert":null}),
            json!({"default":{}}),
            json!({"unknown":"accept"}),
            json!({"confirm":"ACCEPT"}),
        ] {
            assert_eq!(
                PromptPolicy::parse(&value).unwrap_err().code,
                "invalid argument"
            );
        }
    }
}
