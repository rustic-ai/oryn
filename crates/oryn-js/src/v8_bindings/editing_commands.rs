//! Owned admission and resumption of author-observable native editing.
use super::*;
use std::{future::Future, pin::Pin};
#[derive(Clone)]
pub(super) enum Command {
    Execute {
        command: String,
        value: String,
        plaintext: bool,
    },
    Text(String),
    ReplaceSelection(String),
    Html(String),
    Delete {
        forward: bool,
        unit: Option<String>,
    },
    LineBreak,
    Paragraph,
}
impl Command {
    pub(super) fn build(
        self,
        context: Rc<editing_continuation::Context>,
    ) -> Pin<Box<dyn Future<Output = Result<(), V8BindingError>>>> {
        Box::pin(async move {
            let state: &V8BindingState = &context.state;
            let preserve_typing = match &self {
                Self::Delete { .. } | Self::LineBreak | Self::Paragraph => true,
                Self::Execute { command, .. } => matches!(
                    command.as_str(),
                    "delete"
                        | "forwarddelete"
                        | "insertlinebreak"
                        | "insertparagraph"
                        | "formatblock"
                        | "insertorderedlist"
                        | "insertunorderedlist"
                ),
                _ => false,
            };
            let overrides = preserve_typing.then(|| state.editing_overrides.borrow().clone());
            match self {
                Self::ReplaceSelection(text) => {
                    editing_algorithms::replace_selection_with_text(&context, &text).await?
                }
                Self::Text(text) => {
                    editing_algorithms::insert_editing_text(&context, &text).await?
                }
                Self::Html(html) => {
                    editing_algorithms::replace_selection_with_html(&context, &html, true).await?
                }
                Self::Delete { forward, unit } => {
                    if let Some(unit) = unit
                        && state.selection_anchor.get() == state.selection_focus.get()
                    {
                        selection_movement::modify(
                            state,
                            "extend",
                            if forward { "forward" } else { "backward" },
                            &unit,
                        )?;
                    }
                    editing_algorithms::delete_selection_direction(&context, forward).await?;
                }
                Self::LineBreak => editing_algorithms::insert_editing_line_break(&context).await?,
                Self::Paragraph => editing_algorithms::split_editing_paragraph(&context).await?,
                Self::Execute {
                    command,
                    value,
                    plaintext,
                } => {
                    match command.as_str() {
                        "inserttext" => {
                            editing_algorithms::insert_editing_text(&context, &value).await?
                        }
                        "inserthtml" if plaintext => {
                            let parsed = oryn_dom::html::parse_fragment_dom_string_with_context(
                                &value,
                                markup5ever::QualName::new(
                                    None,
                                    "http://www.w3.org/1999/xhtml".into(),
                                    "div".into(),
                                ),
                                Vec::new(),
                                false,
                            );
                            let text = parsed.dom.text_content(parsed.document)?;
                            editing_algorithms::replace_selection_with_text(&context, &text)
                                .await?;
                        }
                        "inserthtml" => {
                            editing_algorithms::replace_selection_with_html(&context, &value, true)
                                .await?
                        }
                        "insertlinebreak" => {
                            editing_algorithms::insert_editing_line_break(&context).await?
                        }
                        "insertparagraph" => {
                            editing_algorithms::split_editing_paragraph(&context).await?
                        }
                        "inserthorizontalrule" => {
                            editing_algorithms::replace_selection_with_html(&context, "<hr>", false)
                                .await?
                        }
                        "insertimage" => {
                            editing_algorithms::replace_selection_with_html(
                                &context,
                                &format!("<img src=\"{}\">", html_attribute_escape(&value)),
                                false,
                            )
                            .await?;
                        }
                        "delete" => {
                            editing_algorithms::delete_selection_direction(&context, false).await?
                        }
                        "forwarddelete" => {
                            editing_algorithms::delete_selection_direction(&context, true).await?
                        }
                        "bold" | "italic" | "underline" | "strikethrough" | "subscript"
                        | "superscript" | "fontname" | "fontsize" | "forecolor" | "backcolor"
                        | "hilitecolor" | "createlink" => {
                            let collapsed = state.selection_anchor.get().is_some()
                                && state.selection_anchor.get() == state.selection_focus.get();
                            if collapsed && command != "createlink" {
                                // Collapsed formatting records typing state. Materialize
                                // markup only when text is inserted, after any line break.
                                let key = if command == "backcolor" {
                                    "hilitecolor"
                                } else {
                                    &command
                                };
                                let boolean = matches!(
                                    command.as_str(),
                                    "bold"
                                        | "italic"
                                        | "underline"
                                        | "strikethrough"
                                        | "subscript"
                                        | "superscript"
                                );
                                let pending = if boolean { "true" } else { &value };
                                context.typing_override(key, pending).await?;
                            } else {
                                editing_algorithms::apply_inline_format(&context, &command, &value)
                                    .await?;
                            }
                        }
                        "removeformat" | "unlink" => {
                            editing_algorithms::remove_selection_format(&context).await?
                        }
                        "formatblock"
                        | "defaultparagraphseparator"
                        | "indent"
                        | "outdent"
                        | "insertorderedlist"
                        | "insertunorderedlist"
                        | "justifyleft"
                        | "justifycenter"
                        | "justifyright"
                        | "justifyfull" => {
                            editing_algorithms::apply_block_command(&context, &command, &value)
                                .await?;
                        }
                        _ => {}
                    }
                }
            }
            // Internal range movement belongs to the edit. Publish preserved
            // typing state through the continuation so undo and retry keep it.
            if let Some(overrides) = overrides {
                context.overrides(overrides).await?;
            }
            Ok(())
        })
    }
}
struct Queued {
    owner: u64,
    job: editing_continuation::Continuation,
    retry: std::time::Instant,
}
#[derive(Default)]
pub(super) struct Registry {
    queue: VecDeque<Queued>,
    polling: bool,
}
pub(super) fn registry(scope: &mut v8::PinScope) -> RcWeak<RefCell<Registry>> {
    if let Some(registry) = scope.get_slot::<Rc<RefCell<Registry>>>() {
        return Rc::downgrade(registry);
    }
    let registry = Rc::new(RefCell::new(Registry::default()));
    scope.set_slot(registry.clone());
    Rc::downgrade(&registry)
}
fn state_rc(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
) -> Result<Rc<V8BindingState>, V8BindingError> {
    scope
        .get_slot::<BindingStateRegistry>()
        .and_then(|registry| {
            registry
                .instances
                .borrow()
                .get(&state.instance_id)
                .and_then(RcWeak::upgrade)
        })
        .ok_or(V8BindingError::Allocation("editing document identity"))
}
pub(super) fn run(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    label: &str,
    command: Command,
) -> Result<bool, V8BindingError> {
    let registry = state
        .editing_continuations
        .upgrade()
        .ok_or(V8BindingError::Allocation("native editing registry"))?;
    {
        let mut queue = registry.borrow_mut();
        if queue.queue.len() >= 32 {
            return Err(V8BindingError::Allocation(
                "native editing admission capacity",
            ));
        }
        queue
            .queue
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("native editing queue"))?;
    }
    let Some(mut job) = editing_continuation::Continuation::new(
        state_rc(scope, state)?,
        label.into(),
        move |context| command.build(context),
    )?
    else {
        return Ok(false);
    };
    match job.poll(scope) {
        Ok(Some(changed)) => Ok(changed),
        result => {
            registry.borrow_mut().queue.push_back(Queued {
                owner: state.instance_id,
                job,
                retry: std::time::Instant::now() + std::time::Duration::from_millis(10),
            });
            match result {
                Err(error) => Err(error),
                _ => Err(V8BindingError::Allocation("native editing suspended")),
            }
        }
    }
}
pub(super) fn delay(state: &V8BindingState) -> Option<std::time::Duration> {
    if state.execution_suspended() || state.execution_retired() {
        return None;
    }
    let registry = state.editing_continuations.upgrade()?;
    let now = std::time::Instant::now();

    registry
        .borrow()
        .queue
        .iter()
        .filter(|job| job.owner == state.instance_id)
        .map(|job| job.retry.saturating_duration_since(now))
        .min()
}
// Exact empty-queue check; future retries and other owners remain active.
pub(super) fn queue_empty(state: &V8BindingState) -> bool {
    state
        .editing_continuations
        .upgrade()
        .is_none_or(|registry| registry.borrow().queue.is_empty())
}

pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
) -> Result<usize, V8BindingError> {
    let Some(registry) = state.editing_continuations.upgrade() else {
        return Ok(0);
    };
    if state.execution_suspended() || registry.borrow().polling {
        return Ok(0);
    }
    registry.borrow_mut().polling = true;
    struct Guard(Rc<RefCell<Registry>>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.borrow_mut().polling = false;
        }
    }
    let _guard = Guard(registry.clone());
    let count = registry.borrow().queue.len();
    let mut complete = 0;
    for _ in 0..count {
        let Some(mut queued) = registry.borrow_mut().queue.pop_front() else {
            break;
        };
        if queued.owner != state.instance_id || queued.retry > std::time::Instant::now() {
            registry.borrow_mut().queue.push_back(queued);
            continue;
        }
        match queued.job.poll(scope) {
            Ok(Some(_)) => complete += 1,
            result => {
                queued.retry = std::time::Instant::now() + std::time::Duration::from_millis(10);
                registry.borrow_mut().queue.push_back(queued);
                result?;
            }
        }
    }
    Ok(complete)
}
pub(super) fn retire(state: &V8BindingState) {
    if let Some(registry) = state.editing_continuations.upgrade() {
        let mut registry = registry.borrow_mut();
        for queued in registry
            .queue
            .iter_mut()
            .filter(|queued| queued.owner == state.instance_id)
        {
            queued.job.cancel();
        }
        registry
            .queue
            .retain(|queued| queued.owner != state.instance_id);
    }
}
