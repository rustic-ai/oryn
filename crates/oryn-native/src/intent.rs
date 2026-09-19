//! Typed agent intent execution over the native semantic resolver/action engine.

use std::time::{Duration, Instant};

use oryn_common::{
    agent::{ExtractKind, Input, Intent, Target},
    v3::{ActionResult, Observation, ObservationDelta, Revision, SemanticAction},
};
use serde::{Deserialize, Serialize};

use crate::{
    oil::NativeOilSession,
    runtime::{ContentKind, NativeAction, NavigationResult, PageHandle, RuntimeError},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IntentResult {
    Observation {
        observation: Observation,
    },
    GeometryObservation {
        observation: oryn_common::v4::Observation,
    },
    SemanticObservation {
        observation: oryn_common::v5::SemanticObservation,
    },
    SemanticObservationV6 {
        observation: oryn_common::v6::SemanticObservation,
    },
    SemanticDeltaV6 {
        delta: oryn_common::v6::SemanticDelta,
    },
    Delta {
        delta: ObservationDelta,
    },
    Action {
        result: ActionResult,
    },
    Navigation {
        navigation: NavigationResult,
    },
    Structured {
        value: serde_json::Value,
    },
    Text {
        value: String,
    },
    Wait {
        revision: Revision,
    },
}

impl PageHandle {
    /// Execute the shared agent IR. Target resolution delegates to the same
    /// semantic/OIL resolver and all mutations delegate to the existing typed
    /// native action path; this method introduces neither a parser nor a
    /// second scoring/action engine.
    pub async fn perform(&self, intent: Intent) -> Result<IntentResult, RuntimeError> {
        match intent {
            Intent::Navigate { url } => Ok(IntentResult::Navigation {
                navigation: self.goto(url).await?,
            }),
            Intent::Back => Ok(IntentResult::Navigation {
                navigation: self.back().await?,
            }),
            Intent::Forward => Ok(IntentResult::Navigation {
                navigation: self.forward().await?,
            }),
            Intent::Refresh => Ok(IntentResult::Navigation {
                navigation: self.refresh().await?,
            }),
            Intent::Observe { options } => {
                if let Some(from) = options.since {
                    if options.semantic_contract_version == Some(oryn_common::v6::CONTRACT_VERSION)
                    {
                        return Ok(IntentResult::SemanticDeltaV6 {
                            delta: self.observe_semantic_delta(from).await?,
                        });
                    }
                    return Ok(IntentResult::Delta {
                        delta: self.observe_delta(from).await?,
                    });
                }
                if options.semantic_contract_version == Some(oryn_common::v6::CONTRACT_VERSION)
                    || options.scope.is_some()
                {
                    return Ok(IntentResult::SemanticObservationV6 {
                        observation: self
                            .observe_semantics_v6(oryn_common::v6::SemanticObservationOptions {
                                profile: options.profile,
                                scope: options.scope,
                                include_hidden: options.include_hidden,
                                geometry: if options.include_geometry || options.viewport_only {
                                    oryn_common::v6::GeometryPolicy::IncludeWhenAvailable
                                } else {
                                    oryn_common::v6::GeometryPolicy::Omit
                                },
                                include_structures: options.include_structures,
                            })
                            .await?,
                    });
                }
                if options.include_structures {
                    return Ok(IntentResult::SemanticObservation {
                        observation: self.observe_semantics(options.profile).await?,
                    });
                }
                if options.include_geometry || options.viewport_only || options.include_hidden {
                    return Ok(IntentResult::GeometryObservation {
                        observation: self
                            .observe_with(oryn_common::v4::ObservationOptions {
                                profile: options.profile,
                                include_geometry: options.include_geometry || options.viewport_only,
                                viewport_only: options.viewport_only,
                                include_hidden: options.include_hidden,
                            })
                            .await?,
                    });
                }
                Ok(IntentResult::Observation {
                    observation: self.observe_profile(options.profile).await?,
                })
            }
            Intent::Click { target, force } => {
                self.perform_action(target, SemanticAction::Click, None, force)
                    .await
            }
            Intent::Type { target, input } => {
                let mut resolver = NativeOilSession::new(self.clone());
                let reference = resolver
                    .resolve_typed_target(&target, SemanticAction::Type, false)
                    .await
                    .map_err(|error| RuntimeError::Intent(error.to_string()))?;
                let result = match input {
                    Input::Text { value } => {
                        self.execute(NativeAction {
                            target: reference,
                            action: SemanticAction::Type,
                            value: Some(value),
                        })
                        .await?
                    }
                    Input::Secret { label } => {
                        self.fill_secret_label(reference, &label, None).await?
                    }
                };
                Ok(IntentResult::Action { result })
            }
            Intent::Clear { target } => {
                self.perform_action(target, SemanticAction::Clear, None, false)
                    .await
            }
            Intent::Check { target, checked } => {
                self.perform_action(
                    target,
                    if checked {
                        SemanticAction::Check
                    } else {
                        SemanticAction::Uncheck
                    },
                    None,
                    false,
                )
                .await
            }
            Intent::Select { target, value } => {
                self.perform_action(target, SemanticAction::Select, Some(value), false)
                    .await
            }
            Intent::Submit { target } => {
                self.perform_action(target, SemanticAction::Submit, None, false)
                    .await
            }
            Intent::Focus { target } => {
                self.perform_action(target, SemanticAction::Focus, None, false)
                    .await
            }
            Intent::Hover { target } => {
                self.perform_action(target, SemanticAction::Hover, None, false)
                    .await
            }
            Intent::ClipboardRead => {
                let context = self.owning_context().ok_or(RuntimeError::Closed)?;
                Ok(IntentResult::Text {
                    value: context.read_clipboard_text(self.page_id(), None)?,
                })
            }
            Intent::ClipboardWrite { value } => {
                let context = self.owning_context().ok_or(RuntimeError::Closed)?;
                context.write_clipboard_text(self.page_id(), value, None)?;
                Ok(IntentResult::Structured {
                    value: serde_json::json!({"written": true, "clipboard": "context_virtual"}),
                })
            }
            Intent::Upload { target, files } => {
                let mut resolver = NativeOilSession::new(self.clone());
                let reference = resolver
                    .resolve_typed_target(&target, SemanticAction::Click, false)
                    .await
                    .map_err(|error| RuntimeError::Intent(error.to_string()))?;
                let context = self.owning_context().ok_or(RuntimeError::Closed)?;
                let handles = files
                    .iter()
                    .map(|label| context.upload_handle(label))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(IntentResult::Action {
                    result: self.set_files(reference, &handles, None).await?,
                })
            }
            Intent::Extract { kind, selector } => match kind {
                ExtractKind::Url => Ok(IntentResult::Text {
                    value: self.observe().await?.page.url,
                }),
                ExtractKind::Title => Ok(IntentResult::Text {
                    value: self.observe().await?.page.title,
                }),
                kind => Ok(IntentResult::Structured {
                    value: self
                        .extract(
                            match kind {
                                ExtractKind::Text => ContentKind::Text,
                                ExtractKind::Html => ContentKind::Html,
                                ExtractKind::Links => ContentKind::Links,
                                ExtractKind::Images => ContentKind::Images,
                                ExtractKind::Tables => ContentKind::Tables,
                                ExtractKind::Url | ExtractKind::Title => unreachable!(),
                            },
                            selector,
                        )
                        .await?,
                }),
            },
            Intent::WaitRevision { after, timeout_ms } => {
                let deadline = Instant::now() + Duration::from_millis(timeout_ms.min(30_000));
                loop {
                    let revision = self.observe().await?.revision;
                    if revision > after {
                        break Ok(IntentResult::Wait { revision });
                    }
                    if Instant::now() >= deadline {
                        break Err(RuntimeError::WaitUnsatisfied(format!(
                            "revision did not advance beyond {}",
                            after.0
                        )));
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    async fn perform_action(
        &self,
        target: Target,
        action: SemanticAction,
        value: Option<String>,
        force: bool,
    ) -> Result<IntentResult, RuntimeError> {
        let mut resolver = NativeOilSession::new(self.clone());
        let target = resolver
            .resolve_typed_target(&target, action, force)
            .await
            .map_err(|error| RuntimeError::Intent(error.to_string()))?;
        let action = NativeAction {
            target,
            action,
            value,
        };
        let result = if force {
            self.execute_forced(action).await?
        } else {
            self.execute(action).await?
        };
        Ok(IntentResult::Action { result })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_common::agent::{Input, Intent, Target};

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn typed_intent_uses_the_native_semantic_resolver_and_action_engine() {
        let context = crate::runtime::Browser::in_process_with_config(
            crate::runtime::BrowserConfig::default(),
        )
        .new_context();
        let page = context.new_page();
        page.load_html(
            "https://intent.example/app",
            "<button id=continue>Continue</button><output id=result></output><script>document.querySelector('#continue').addEventListener('click',()=>document.querySelector('#result').textContent='done')</script>",
        )
        .await
        .unwrap();
        let IntentResult::Action { result } = page
            .perform(Intent::Click {
                target: Target::Text {
                    text: "Continue".into(),
                },
                force: false,
            })
            .await
            .unwrap()
        else {
            panic!("expected action result")
        };
        assert!(result.revision_after > result.revision_before);
        assert!(result.diagnostics.is_empty());
        assert_eq!(
            page.extract(ContentKind::Text, Some("#result".into()))
                .await
                .unwrap(),
            serde_json::json!("done")
        );
    }

    #[test]
    fn typed_secret_input_contains_a_label_but_no_value_field() {
        let encoded = serde_json::to_value(Intent::Type {
            target: Target::Role {
                role: "textbox".into(),
            },
            input: Input::Secret {
                label: "login-password".into(),
            },
        })
        .unwrap();
        assert_eq!(encoded["input"]["label"], "login-password");
        assert!(encoded["input"].get("value").is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn typed_observe_can_request_structured_semantics() {
        let context = crate::runtime::Browser::in_process_with_config(
            crate::runtime::BrowserConfig::default(),
        )
        .new_context();
        let page = context.new_page();
        page.load_html(
            "https://intent.example/semantic",
            "<nav aria-label=Primary><a href=/home>Home</a></nav>",
        )
        .await
        .unwrap();

        let IntentResult::SemanticObservation { observation } = page
            .perform(Intent::Observe {
                options: oryn_common::agent::ObserveOptions {
                    include_structures: true,
                    ..Default::default()
                },
            })
            .await
            .unwrap()
        else {
            panic!("expected semantic observation")
        };

        assert_eq!(
            observation.contract_version,
            oryn_common::v5::CONTRACT_VERSION
        );
        assert!(observation.structures.iter().any(|structure| {
            structure.kind == oryn_common::v5::SemanticStructureKind::Navigation
        }));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn typed_virtual_clipboard_is_policy_controlled_and_context_isolated() {
        let browser = crate::runtime::Browser::in_process_with_config(
            crate::runtime::BrowserConfig::default(),
        );
        let mut options = crate::runtime::ContextOptions::default();
        options.action_policy.rules.insert(
            crate::security::ConsequentialAction::Clipboard,
            crate::security::PolicyRule::Allow,
        );
        let first = browser.new_context_with_options(options.clone());
        let second = browser.new_context_with_options(options);
        let first_page = first.new_page();
        let second_page = second.new_page();
        first_page
            .load_html("https://clipboard.test/first", "<!doctype html>")
            .await
            .unwrap();
        second_page
            .load_html("https://clipboard.test/second", "<!doctype html>")
            .await
            .unwrap();
        first_page
            .perform(Intent::ClipboardWrite {
                value: "private".into(),
            })
            .await
            .unwrap();
        let IntentResult::Text { value } = first_page.perform(Intent::ClipboardRead).await.unwrap()
        else {
            panic!("expected clipboard text")
        };
        assert_eq!(value, "private");
        let IntentResult::Text { value } =
            second_page.perform(Intent::ClipboardRead).await.unwrap()
        else {
            panic!("expected isolated clipboard text")
        };
        assert!(value.is_empty());
    }
}
