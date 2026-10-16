//! Owned admission and resumption of author-observable native editing.
use super::*;
use std::{future::Future, pin::Pin};
#[derive(Clone)]
pub(super) enum Command {
    Execute {
        command: String,
        value: String,
        plaintext: bool,
        invocation: Rc<Invocation>,
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
    fn invocation(&self) -> Option<Rc<Invocation>> {
        match self {
            Self::Execute { invocation, .. } => Some(invocation.clone()),
            _ => None,
        }
    }
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
                        context.layout().await?;
                        selection_movement::modify_snapshot(
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
                    invocation: _invocation,
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
// An invocation belongs to a receiver document, not the process or current
// focused editor. The retained state prevents arena identity reuse while live.
pub(super) struct Invocation {
    _owner: Rc<V8BindingState>,
    _document: oryn_dom::TrackedNodeLease,
}
type InvocationKey = (usize, NodeId, Option<(u64, u64)>);
pub(super) fn admit_invocation(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    document: NodeId,
) -> Result<Option<Rc<Invocation>>, V8BindingError> {
    let registry = state
        .editing_continuations
        .upgrade()
        .ok_or(V8BindingError::Allocation("editing invocation registry"))?;
    let owner = active_document_state_for_node(scope, state, document);
    let generation = owner.as_ref().map(|owner| {
        (
            owner.document_generation(),
            owner.document_authority_epoch(),
        )
    });
    let key = (Rc::as_ptr(&state.dom) as usize, document, generation);
    let mut registry = registry.borrow_mut();
    registry
        .invocations
        .retain(|(_, owner)| owner.strong_count() != 0);
    if registry
        .invocations
        .iter()
        .any(|(current, _)| *current == key)
    {
        return Ok(None);
    }
    registry
        .invocations
        .try_reserve(1)
        .map_err(|_| V8BindingError::Allocation("editing invocation admission"))?;
    let lease = oryn_dom::TrackedNodeLease::new(&state.dom, document)?;
    let invocation = Rc::new(Invocation {
        _owner: owner.unwrap_or_else(|| state.clone()),
        _document: lease,
    });
    registry.invocations.push((key, Rc::downgrade(&invocation)));
    Ok(Some(invocation))
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
    invocations: Vec<(InvocationKey, RcWeak<Invocation>)>,
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
    let invocation = command.invocation();
    let Some(mut job) = editing_continuation::Continuation::new(
        state_rc(scope, state)?,
        label.into(),
        move |context| command.build(context),
    )?
    else {
        return Ok(false);
    };
    job.retain_invocation(invocation);
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

#[cfg(test)]
mod invocation_tests {
    use super::*;
    #[test]
    fn queued_retirement_releases_only_its_document_invocation() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<body><iframe id=child></iframe><div id=host contenteditable>x</div></body>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let document = {
            let mut dom = dom.borrow_mut();
            let child = dom.find_by_id("child").unwrap();
            let document = dom.create(NodeKind::Document);
            dom.arena_mut().get_mut(document).unwrap().document_metadata =
                Some(Box::new(oryn_dom::DocumentMetadata::html()));
            let editor = dom.create_element("div");
            dom.set_attribute(editor, "contenteditable", "true")
                .unwrap();
            let text = dom.create_text("y");
            dom.append_child(editor, text).unwrap();
            dom.append_child(document, editor).unwrap();
            dom.set_hosted_document(child, Some(document)).unwrap();
            document
        };
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let root_context = v8::Context::new(scope, Default::default());
        let root = {
            let scope = &mut v8::ContextScope::new(scope, root_context);
            install_generated_bindings(scope, root_context, dom.clone(), RealmId(0), 1).unwrap()
        };
        let child_context = v8::Context::new(scope, Default::default());
        let child = {
            let scope = &mut v8::ContextScope::new(scope, child_context);
            install_generated_bindings_with_root(
                scope,
                child_context,
                dom.clone(),
                RealmId(1),
                1,
                document,
            )
            .unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, root_context);
        assert!(
            super::super::test_eval::eval(
                scope,
                "getSelection().selectAllChildren(document.getElementById('host'));true"
            )
            .is_true()
        );
        let root_token = admit_invocation(scope, &root, root.document_root())
            .unwrap()
            .unwrap();
        // Borrowed receiver and owner realm must resolve the identical admission key.
        let child_token = admit_invocation(scope, &root, document).unwrap().unwrap();
        assert!(admit_invocation(scope, &child, document).unwrap().is_none());
        assert!(
            admit_invocation(scope, &root, root.document_root())
                .unwrap()
                .is_none()
        );
        let weak = Rc::downgrade(&root_token);
        let mut job =
            editing_continuation::Continuation::new(root.clone(), "retire".into(), move |_| {
                Box::pin(async { Ok(()) })
            })
            .unwrap()
            .unwrap();
        job.retain_invocation(Some(root_token));
        let registry = root.editing_continuations.upgrade().unwrap();
        registry.borrow_mut().queue.push_back(Queued {
            owner: root.instance_id,
            job,
            retry: std::time::Instant::now(),
        });
        retire(&root);
        assert!(weak.upgrade().is_none());
        assert!(registry.borrow().queue.is_empty());
        assert!(admit_invocation(scope, &child, document).unwrap().is_none());
        drop(child_token);
        assert!(admit_invocation(scope, &child, document).unwrap().is_some());
        assert!(
            admit_invocation(scope, &root, root.document_root())
                .unwrap()
                .is_some()
        );
    }
}
