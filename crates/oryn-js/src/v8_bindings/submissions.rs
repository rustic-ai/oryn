//! Native ownership of accepted default actions across scheduler checkpoints.
use super::*;

const MAX_PENDING_ACTIONS: usize = 1024;

fn submission_target(
    dom: &oryn_dom::DomRuntime,
    document: NodeId,
    form: NodeId,
    submitter: Option<NodeId>,
) -> Result<String, DomError> {
    let mut target = submitter
        .map(|node| dom.attribute_in_null_namespace(node, "formtarget"))
        .transpose()?
        .flatten()
        .or(dom.attribute_in_null_namespace(form, "target")?);
    if target.is_none() {
        let mut pending = dom
            .children(document)?
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>();
        while let Some(node) = pending.pop() {
            // Frame documents still share the arena during R4 migration, but
            // their base elements never supply this document's target.
            if matches!(dom.arena().get(node)?.kind, NodeKind::Document) {
                continue;
            }
            if dom.is_element(node, "http://www.w3.org/1999/xhtml", "base") {
                target = dom.attribute_in_null_namespace(node, "target")?;
                if target.is_some() {
                    break;
                }
            }
            pending.extend(dom.children(node)?.iter().rev().copied());
        }
    }
    let target = target.unwrap_or("");
    Ok(
        if target.contains('<') && target.contains(['\t', '\n', '\r']) {
            "_blank".into()
        } else {
            target.to_owned()
        },
    )
}

fn check_capacity(state: &V8BindingState) -> Result<(), V8BindingError> {
    if state.pending_default_actions.borrow().len() >= MAX_PENDING_ACTIONS {
        return Err(V8BindingError::Unsupported {
            interface: "EventTarget".into(),
            member: "QuotaExceededError: default-action queue capacity".into(),
        });
    }
    Ok(())
}

fn check_submission_capacity(state: &V8BindingState, form: NodeId) -> Result<(), V8BindingError> {
    if state
        .pending_default_actions
        .borrow()
        .iter()
        .any(|action| matches!(action, PendingDefaultAction::Submit(old) if old.form == form))
    {
        return Ok(());
    }
    check_capacity(state)
}

pub struct RootedTarget {
    pub node: NodeId,
    arena: oryn_dom::ArenaId,
    dom: oryn_dom::SharedDomRuntime,
}

impl RootedTarget {
    pub(super) fn new(state: &V8BindingState, node: NodeId) -> Result<Self, V8BindingError> {
        let mut dom = state.dom.borrow_mut();
        dom.retain_native(node)?;
        Ok(Self {
            node,
            arena: dom.arena().id(),
            dom: state.dom.clone(),
        })
    }
}

impl Drop for RootedTarget {
    fn drop(&mut self) {
        let mut dom = self.dom.borrow_mut();
        // Replacing the whole arena already destroyed its root accounting.
        if dom.arena().id() == self.arena {
            let result = dom.release_native(self.node);
            debug_assert!(
                result.is_ok(),
                "default action lost its native lease: {result:?}"
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionSource {
    Submit,
    RequestSubmit,
    Activation,
}

pub struct SubmissionTransaction {
    pub form: NodeId,
    pub submitter: Option<NodeId>,
    pub document: oryn_dom::DocumentId,
    pub document_generation: u64,
    pub source: SubmissionSource,
    pub navigation: LocationNavigation,
    pub target: String,
    _form: RootedTarget,
    _submitter: Option<RootedTarget>,
    _document: RootedTarget,
}

pub(super) fn queue(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    form: NodeId,
    submitter: Option<NodeId>,
    source: SubmissionSource,
) -> Result<(), V8BindingError> {
    if !state.forms_allowed.get()
        || state.entry_lists_in_progress.borrow().contains(&form)
        || !state.dom.borrow().is_connected(form)?
    {
        return Ok(());
    }
    check_submission_capacity(state, form)?;
    let document = owner_document_for_node(state, form)?;
    let identity = state.dom.borrow().arena().document_id(document)?;
    let form_lease = RootedTarget::new(state, form)?;
    let submitter_lease = submitter
        .map(|node| RootedTarget::new(state, node))
        .transpose()?;
    let document_lease = RootedTarget::new(state, document)?;
    let encoding = {
        let dom = state.dom.borrow();
        dom.attribute_in_null_namespace(form, "accept-charset")?
            .into_iter()
            .flat_map(str::split_ascii_whitespace)
            .find_map(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
            .unwrap_or(hyperlinks::document_encoding(state, document)?)
            .output_encoding()
    };
    let encode: url::form_urlencoded::EncodingOverride<'_> = Some(&|text| encoding.encode(text).0);
    let data = form_data::create(scope, state, Some(form), submitter, None, encoding)?;
    if !state.dom.borrow().is_connected(form)? || owner_document_for_node(state, form)? != document
    {
        return Ok(());
    }
    // Author callbacks have completed. Parameter reads below are native and do
    // not invoke replaceable form properties or FormData iteration methods.
    let (method, action, enctype, target, no_referrer) = {
        let dom = state.dom.borrow();
        let parameter = |override_name: &str, name: &str| -> Result<String, V8BindingError> {
            Ok(submitter
                .and_then(|node| {
                    dom.attribute_in_null_namespace(node, override_name)
                        .ok()
                        .flatten()
                })
                .or(dom.attribute_in_null_namespace(form, name)?)
                .unwrap_or("")
                .to_owned())
        };
        (
            parameter("formmethod", "method")?,
            parameter("formaction", "action")?,
            parameter("formenctype", "enctype")?,
            submission_target(&dom, document, form, submitter)?,
            dom.attribute_in_null_namespace(form, "rel")?
                .unwrap_or("")
                .split_ascii_whitespace()
                .any(|s| s.eq_ignore_ascii_case("noreferrer")),
        )
    };
    if method.eq_ignore_ascii_case("dialog") {
        let dialog = {
            let dom = state.dom.borrow();
            dom.arena()
                .ancestors(form)?
                .into_iter()
                .find(|node| dom.is_element(*node, "http://www.w3.org/1999/xhtml", "dialog"))
        };
        if let Some(dialog) = dialog {
            let mut dom = state.dom.borrow_mut();
            if dom.attribute_in_null_namespace(dialog, "open")?.is_none() {
                return Ok(());
            }
            if let Some(submitter) = submitter {
                let value = if dom.is_element(submitter, "http://www.w3.org/1999/xhtml", "input")
                    && dom
                        .attribute_in_null_namespace(submitter, "type")?
                        .is_some_and(|s| s.eq_ignore_ascii_case("image"))
                {
                    let (x, y) = dom.arena().get(submitter)?.image_coordinates;
                    format!("{x},{y}")
                } else {
                    dom.control_value(submitter)?
                };
                dom.set_attribute(dialog, "data-oryn-return-value", value)?;
            }
            dom.remove_attribute_ns(dialog, None, "open")?;
            dom.remove_attribute_ns(dialog, None, "data-oryn-modal")?;
            drop(dom);
            let event = create_event_object(scope, state, "close", false, false)?;
            dispatch_event(scope, state, dialog, event)?;
        }
        return Ok(());
    }
    let base = document_base_url(scope, state, true)?;
    let url = if action.is_empty() {
        Ok(state.location_url.borrow().clone())
    } else {
        Url::parse(&base).and_then(|base| base.join(&action))
    };
    let Ok(mut url) = url else {
        return Ok(());
    };
    let post = if method.eq_ignore_ascii_case("post") && matches!(url.scheme(), "http" | "https") {
        let (bytes, content_type) = if enctype.eq_ignore_ascii_case("multipart/form-data") {
            form_data_body_encoded(scope, state, data.into(), encoding)?
                .ok_or(V8BindingError::JavaScriptException)?
        } else {
            let pairs = form_data::name_value_pairs(scope, state, data)?;
            if enctype.eq_ignore_ascii_case("text/plain") {
                (
                    encoding
                        .encode(
                            &pairs
                                .into_iter()
                                .map(|(name, value)| format!("{name}={value}\r\n"))
                                .collect::<String>(),
                        )
                        .0
                        .into_owned(),
                    "text/plain".into(),
                )
            } else {
                (
                    url::form_urlencoded::Serializer::new(String::new())
                        .encoding_override(encode)
                        .extend_pairs(pairs)
                        .finish()
                        .into_bytes(),
                    "application/x-www-form-urlencoded".into(),
                )
            }
        };
        Some(crate::navigation::PostResource {
            bytes,
            content_type,
        })
    } else {
        if !method.eq_ignore_ascii_case("post") && matches!(url.scheme(), "http" | "https" | "data")
        {
            let pairs = form_data::name_value_pairs(scope, state, data)?;
            let query = url::form_urlencoded::Serializer::new(String::new())
                .encoding_override(encode)
                .extend_pairs(pairs)
                .finish();
            url.set_query(Some(&query));
        }
        None
    };
    let transaction = SubmissionTransaction {
        form,
        submitter,
        document: identity,
        document_generation: state.document_generation,
        source,
        navigation: LocationNavigation {
            url: url.into(),
            replace: false,
            reload: false,
            post,
            referrer_policy: no_referrer.then(|| "no-referrer".into()),
        },
        target,
        _form: form_lease,
        _submitter: submitter_lease,
        _document: document_lease,
    };
    check_submission_capacity(state, form)?;
    // A later planned navigation from this form supersedes its predecessor.
    state
        .pending_default_actions
        .borrow_mut()
        .retain(|action| !matches!(action, PendingDefaultAction::Submit(old) if old.form == form));
    state
        .pending_default_actions
        .borrow_mut()
        .push_back(PendingDefaultAction::Submit(Box::new(transaction)));
    Ok(())
}

pub(super) fn queue_click(state: &V8BindingState, node: NodeId) -> Result<(), V8BindingError> {
    check_capacity(state)?;
    let target = RootedTarget::new(state, node)?;
    state
        .pending_default_actions
        .borrow_mut()
        .push_back(PendingDefaultAction::Click(target));
    Ok(())
}

/// Host entry, independent of replaceable page globals and prototypes.
pub fn request_submit(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    form: NodeId,
    submitter: Option<NodeId>,
) -> Result<(), V8BindingError> {
    // Validation dispatches author callbacks, which may detach nodes and force GC.
    let _form = RootedTarget::new(state, form)?;
    let _submitter = submitter
        .map(|node| RootedTarget::new(state, node))
        .transpose()?;
    if validation::request_submit(scope, state, form, submitter)? {
        queue(
            scope,
            state,
            form,
            submitter,
            SubmissionSource::RequestSubmit,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_use_first_base_and_preserve_explicit_empty_overrides() {
        let mut dom = oryn_dom::html::parse(
            "<base target=first><base target=second><form id=f><button id=b></button></form>",
        )
        .into_runtime()
        .unwrap();
        let form = dom.find_by_id("f").unwrap();
        let button = dom.find_by_id("b").unwrap();
        let target = |dom: &oryn_dom::DomRuntime| {
            submission_target(dom, dom.document(), form, Some(button)).unwrap()
        };
        assert_eq!(target(&dom), "first");
        dom.set_attribute(form, "target", "").unwrap();
        assert_eq!(target(&dom), "");
        dom.set_attribute(form, "target", "form").unwrap();
        assert_eq!(target(&dom), "form");
        dom.set_attribute(button, "formtarget", "").unwrap();
        assert_eq!(target(&dom), "");
        for value in ["<\n", "\t<", "<\r"] {
            dom.set_attribute(button, "formtarget", value).unwrap();
            assert_eq!(target(&dom), "_blank");
        }
        for value in ["<", "\n", "normal"] {
            dom.set_attribute(button, "formtarget", value).unwrap();
            assert_eq!(target(&dom), value);
        }
    }

    #[test]
    fn queued_submission_retains_detached_nodes_and_rejects_overflow() {
        crate::v8_bindings::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<form id=f><button id=b></button></form>")
                .into_runtime()
                .unwrap(),
        ));
        let form = dom.borrow().find_by_id("f").unwrap();
        let button = dom.borrow().find_by_id("b").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 7).unwrap();
        queue(
            scope,
            &state,
            form,
            Some(button),
            SubmissionSource::Activation,
        )
        .unwrap();
        for _ in 1..MAX_PENDING_ACTIONS {
            queue_click(&state, button).unwrap();
        }
        // Supersession replaces an existing slot even when the queue is full.
        queue(
            scope,
            &state,
            form,
            Some(button),
            SubmissionSource::Activation,
        )
        .unwrap();
        dom.borrow_mut().remove(form).unwrap();
        assert!(dom.borrow().arena().get(button).is_ok());
        assert!(queue_click(&state, button).is_err());
        assert_eq!(
            state.pending_default_actions.borrow().len(),
            MAX_PENDING_ACTIONS
        );
        let PendingDefaultAction::Submit(transaction) = state
            .pending_default_actions
            .borrow_mut()
            .pop_back()
            .unwrap()
        else {
            panic!("expected submission")
        };
        assert_eq!(transaction.document_generation, 7);
        assert_eq!(transaction.submitter, Some(button));
        state.pending_default_actions.borrow_mut().clear();
        assert!(dom.borrow().arena().get(button).is_ok());
        drop(transaction);
    }
}
