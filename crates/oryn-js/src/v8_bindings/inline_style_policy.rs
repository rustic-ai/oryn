//! Native inline style authorization. Retain the exact source decision until a
//! DOM edit changes it; later meta policies do not retroactively disable CSS.
type InlineStylePolicy =
    Rc<dyn Fn(&V8BindingState, &str, Option<&str>, bool) -> Result<bool, String>>;

use super::*;
pub(super) type Policy = dyn Fn(&V8BindingState, &str, Option<&str>, bool) -> Result<bool, String>;
static RETAINED_STYLE_SOURCE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
pub(super) struct Check {
    document: NodeId,
    epoch: u64,
    revision: u64,
    source: String,
    nonce: Option<String>,
    cssom: bool,
    charged: usize,
}
impl PartialEq for Check {
    fn eq(&self, other: &Self) -> bool {
        self.document == other.document
            && self.epoch == other.epoch
            && self.revision == other.revision
            && self.source == other.source
            && self.nonce == other.nonce
            && self.cssom == other.cssom
    }
}
impl Eq for Check {}
impl Drop for Check {
    fn drop(&mut self) {
        RETAINED_STYLE_SOURCE.fetch_sub(self.charged, std::sync::atomic::Ordering::AcqRel);
    }
}
impl Check {
    fn retain(&mut self) -> Result<(), V8BindingError> {
        let bytes = self
            .source
            .len()
            .saturating_add(self.nonce.as_ref().map_or(0, String::len));
        RETAINED_STYLE_SOURCE
            .fetch_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |used| {
                    used.checked_add(bytes)
                        .filter(|next| *next <= 32 * 1024 * 1024)
                },
            )
            .map_err(|_| V8BindingError::Allocation("shared inline style policy source budget"))?;
        self.charged = bytes;
        Ok(())
    }
}

fn stylesheet_text(dom: &DomRuntime, node: NodeId) -> Result<String, V8BindingError> {
    let mut stack = vec![node];
    let mut output = String::new();
    while let Some(node) = stack.pop() {
        let record = dom.arena().get(node)?;
        match &record.kind {
            NodeKind::Text { data } | NodeKind::CDataSection { data } => {
                if output.len().saturating_add(data.len()) > 8 * 1024 * 1024 {
                    return Err(V8BindingError::Allocation("inline stylesheet source size"));
                }
                output.push_str(data);
            }
            NodeKind::Comment { .. } | NodeKind::ProcessingInstruction { .. } => {}
            _ => stack.extend(record.children.iter().rev().copied()),
        }
    }
    Ok(output)
}
impl V8BindingState {
    pub fn set_inline_style_policy(&self, policy: InlineStylePolicy) {
        *self.inline_style_policy.borrow_mut() = Some(policy);
    }
    pub fn stage_snapshot_inline_styles(&self, nodes: &[NodeId]) -> Result<(), V8BindingError> {
        if nodes.len() > 8192 {
            return Err(V8BindingError::Allocation(
                "snapshot stylesheet boundary capacity",
            ));
        }
        let mut pending = self.snapshot_inline_styles.borrow_mut();
        let mut dom = self.dom.borrow_mut();
        for node in nodes {
            let revision = dom.arena().stylesheet_revision(*node)?;
            let owner = dom.arena().is_css_stylesheet_owner(*node)?;
            let record = dom.arena_mut().get_mut(*node)?;
            pending.insert(*node, (record.style_attribute_revision, revision));
            if record.attributes.contains_null("style") {
                record.style_attribute_policy_allowed = false;
            }
            if owner {
                record.stylesheet_parser_ready = false;
            }
        }
        Ok(())
    }
    pub fn release_snapshot_inline_style(&self, node: NodeId) -> Result<(), V8BindingError> {
        if self
            .snapshot_inline_styles
            .borrow_mut()
            .remove(&node)
            .is_some()
        {
            self.dom
                .borrow_mut()
                .arena_mut()
                .get_mut(node)?
                .stylesheet_parser_ready = true;
        }
        self.reconcile_inline_style_policy()
    }
    pub fn reconcile_inline_style_policy(&self) -> Result<(), V8BindingError> {
        if self.execution_retired() {
            return Ok(());
        }
        let Some(policy) = self.inline_style_policy.borrow().clone() else {
            return Ok(());
        };
        let document = self.document_root();
        let epoch = self.document_stream_epoch();
        self.inline_style_checks
            .borrow_mut()
            .retain(|_, check| check.document == document && check.epoch == epoch);
        let nodes = self.dom.borrow().arena().descendants_inclusive(document)?;
        let mut live = BTreeSet::new();
        let mut bytes = 0usize;
        for node in nodes {
            let waiting = self.snapshot_inline_styles.borrow().get(&node).copied();
            if let Some((attribute, sheet)) = waiting {
                let unchanged = {
                    let dom = self.dom.borrow();
                    dom.arena().get(node)?.style_attribute_revision == attribute
                        && dom.arena().stylesheet_revision(node)? == sheet
                };
                if unchanged {
                    continue;
                }
                self.snapshot_inline_styles.borrow_mut().remove(&node);
                self.dom
                    .borrow_mut()
                    .arena_mut()
                    .get_mut(node)?
                    .stylesheet_parser_ready = true;
            }
            let inputs = (|| {
                let dom = self.dom.borrow();
                let record = dom.arena().get(node)?;
                if record
                    .attributes
                    .get_null("style")
                    .is_some_and(|source| source.len() > 8 * 1024 * 1024)
                    || dom
                        .element_nonce_for_csp(node)?
                        .is_some_and(|nonce| nonce.len() > 4096)
                {
                    return Err(V8BindingError::Allocation("inline style policy input size"));
                }
                let sheet = matches!(&record.kind,NodeKind::Element{name} if name.local.as_ref()=="style" && matches!(name.ns.as_ref(),"http://www.w3.org/1999/xhtml"|"http://www.w3.org/2000/svg"));
                Ok((
                    record.attributes.get_null("style").cloned(),
                    if sheet && record.stylesheet_parser_ready {
                        Some(stylesheet_text(&dom, node)?)
                    } else {
                        None
                    },
                    dom.element_nonce_for_csp(node)?.map(str::to_owned),
                    record.style_attribute_cssom,
                    record.style_attribute_revision,
                ))
            })();
            let (attribute, sheet, nonce, cssom, revision) = match inputs {
                Ok(inputs) => inputs,
                Err(error) => {
                    let mut dom = self.dom.borrow_mut();
                    let record = dom.arena_mut().get_mut(node)?;
                    record.style_attribute_policy_allowed = false;
                    record.stylesheet_policy_allowed = false;
                    return Err(error);
                }
            };
            for (attribute, source) in [(true, attribute), (false, sheet)] {
                let Some(source) = source else { continue };
                bytes = bytes
                    .saturating_add(source.len())
                    .saturating_add(nonce.as_ref().map_or(0, String::len));
                if bytes > 32 * 1024 * 1024 {
                    let mut dom = self.dom.borrow_mut();
                    let record = dom.arena_mut().get_mut(node)?;
                    if attribute {
                        record.style_attribute_policy_allowed = false;
                    } else {
                        record.stylesheet_policy_allowed = false;
                    }
                    return Err(V8BindingError::Allocation(
                        "inline style policy source budget",
                    ));
                }
                let key = (node, attribute);
                live.insert(key);
                let mut check = Check {
                    charged: 0,
                    document,
                    epoch,
                    revision: if attribute {
                        revision
                    } else {
                        self.dom.borrow().arena().stylesheet_revision(node)?
                    },
                    source,
                    nonce: if attribute { None } else { nonce.clone() },
                    cssom: attribute && cssom,
                };
                if self.inline_style_checks.borrow().get(&key) == Some(&check) {
                    continue;
                }
                self.inline_style_checks.borrow_mut().remove(&key);
                {
                    let mut dom = self.dom.borrow_mut();
                    let record = dom.arena_mut().get_mut(node)?;
                    if attribute {
                        record.style_attribute_policy_allowed = false;
                    } else {
                        record.stylesheet_policy_allowed = false;
                    }
                }
                check.retain()?;
                let allowed = check.cssom
                    || policy(self, &check.source, check.nonce.as_deref(), attribute)
                        .map_err(V8BindingError::EventCallback)?;
                {
                    let mut dom = self.dom.borrow_mut();
                    let record = dom.arena_mut().get_mut(node)?;
                    if attribute {
                        record.style_attribute_policy_allowed = allowed;
                    } else {
                        record.stylesheet_policy_allowed = allowed;
                    }
                }
                self.layout_cache_key.borrow_mut().take();
                self.inline_style_checks.borrow_mut().insert(key, check);
            }
        }
        self.inline_style_checks
            .borrow_mut()
            .retain(|key, _| live.contains(key));
        Ok(())
    }
}

impl V8BindingState {
    pub fn queue_native_csp_event(
        &self,
        event: crate::platform_control::CspEvent,
    ) -> Result<(), String> {
        if self.execution_retired() {
            return Err("retired CSP document".into());
        }
        let mut tasks = self.native_csp_tasks.borrow_mut();
        if tasks.len() >= 256 {
            return Err("native CSP event capacity".into());
        }
        tasks.push_back((self.document_root(), self.document_authority_epoch(), event));
        Ok(())
    }
    pub fn has_pending_native_csp_event(&self) -> bool {
        !self.execution_retired() && !self.native_csp_tasks.borrow().is_empty()
    }
}
