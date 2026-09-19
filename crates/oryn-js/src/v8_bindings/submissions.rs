//! Native ownership of accepted default actions across scheduler checkpoints.
use super::*;

const MAX_PENDING_ACTIONS: usize = 1024;

pub type SelectNavigationTarget =
    for<'s, 'i> fn(
        &mut v8::PinScope<'s, 'i>,
        &V8BindingState,
        &oryn_dom::DocumentProvenance,
        &str,
        &str,
        bool,
    ) -> Result<Option<SelectedNavigationTarget>, V8BindingError>;
pub type DispatchSelectedLink = for<'s, 'i> fn(
    &mut v8::PinScope<'s, 'i>,
    &SelectedNavigationTarget,
    &oryn_dom::DocumentProvenance,
    &str,
    Option<&str>,
    bool,
    Option<crate::navigation::HyperlinkAuditToken>,
) -> Result<(), V8BindingError>;
pub type PrepareHyperlinkAudit = for<'s, 'i> fn(
    &mut v8::PinScope<'s, 'i>,
    &V8BindingState,
    &str,
    &[String],
) -> Result<
    Option<std::sync::Arc<dyn crate::navigation::HyperlinkAudit>>,
    V8BindingError,
>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedNavigationTarget {
    pub receipt: crate::navigation::NavigationTargetReceipt,
    pub page_id: u64,
    pub local_realm: Option<RealmId>,
    pub local_frame: Option<(RealmId, u64, NodeId, u64)>,
}

#[derive(Clone, Copy)]
pub(super) enum FormOperation {
    EntryList,
    SubmitEvent,
}

impl FormOperation {
    fn records(self, state: &V8BindingState) -> &RefCell<Vec<Rc<oryn_dom::TrackedNodeLease>>> {
        match self {
            Self::EntryList => &state.entry_lists_in_progress,
            Self::SubmitEvent => &state.submission_events_in_progress,
        }
    }

    pub(super) fn active(
        self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        form: NodeId,
    ) -> bool {
        let arena = state.dom.borrow().arena().id();
        let owners = scope
            .get_slot::<BindingStateRegistry>()
            .map(|registry| {
                registry
                    .instances
                    .borrow()
                    .values()
                    .filter_map(RcWeak::upgrade)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| vec![state.clone()]);
        owners.iter().any(|owner| {
            self.records(owner).borrow().iter().any(|lease| {
                let (dom, node) = lease.current();
                Rc::ptr_eq(&dom, &state.dom) && lease.identity().arena == arena && node == form
            })
        })
    }
}

pub(super) struct FormOperationGuard {
    state: Rc<V8BindingState>,
    operation: FormOperation,
    lease: Rc<oryn_dom::TrackedNodeLease>,
}
impl FormOperationGuard {
    pub(super) fn begin(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        form: NodeId,
        operation: FormOperation,
    ) -> Result<Option<Self>, V8BindingError> {
        if operation.active(scope, state, form) {
            return Ok(None);
        }
        let lease = Rc::new(oryn_dom::TrackedNodeLease::new(&state.dom, form)?);
        let mut records = operation.records(state).borrow_mut();
        records
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("form operation guard"))?;
        records.push(lease.clone());
        drop(records);
        Ok(Some(Self {
            state: state.clone(),
            operation,
            lease,
        }))
    }
}
impl Drop for FormOperationGuard {
    fn drop(&mut self) {
        self.operation
            .records(&self.state)
            .borrow_mut()
            .retain(|lease| !Rc::ptr_eq(lease, &self.lease));
    }
}

/// Capture relationship policy from native attributes before queuing navigation.
/// Explicit noreferrer wins over opener; blank targets imply noopener unless
/// the author explicitly requests opener.
pub fn target_noopener(target: &str, rel: &str) -> bool {
    let rel = String::from_utf16_lossy(&oryn_dom::dom_string::units(rel));
    let tokens = rel
        .split_ascii_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<BTreeSet<_>>();
    tokens.contains("noopener")
        || tokens.contains("noreferrer")
        || (target.eq_ignore_ascii_case("_blank") && !tokens.contains("opener"))
}

pub use crate::navigation::effective_navigation_target;

/// The noreferrer link type overrides even an explicit referrer policy.
pub fn link_referrer_policy(policy: Option<&str>, rel: &str) -> Option<String> {
    let rel = String::from_utf16_lossy(&oryn_dom::dom_string::units(rel));
    if rel
        .split_ascii_whitespace()
        .any(|token| token.eq_ignore_ascii_case("noreferrer"))
    {
        Some("no-referrer".into())
    } else {
        policy.map(str::to_owned)
    }
}

fn submission_target(
    dom: &oryn_dom::DomRuntime,
    document: NodeId,
    form: NodeId,
    submitter: Option<NodeId>,
) -> Result<String, DomError> {
    let mut target = submitter
        .map(|node| dom.submitter_parameter(node, "formtarget"))
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
    Ok(normalize_target(target.unwrap_or("")))
}

fn normalize_target(target: &str) -> String {
    let units = oryn_dom::dom_string::units(target);
    if units.contains(&u16::from(b'<')) && units.iter().any(|unit| matches!(*unit, 9 | 10 | 13)) {
        "_blank".into()
    } else {
        target.to_owned()
    }
}

fn same_form(transaction: &SubmissionTransaction, state: &V8BindingState, form: NodeId) -> bool {
    let (dom, node) = transaction._form._lease.current();
    Rc::ptr_eq(&dom, &state.dom)
        && transaction._form._lease.identity().arena == state.dom.borrow().arena().id()
        && node == form
}

fn check_capacity(state: &V8BindingState) -> Result<(), V8BindingError> {
    if state
        .pending_default_actions
        .borrow()
        .len()
        .saturating_add(state.reserved_default_actions.get())
        >= MAX_PENDING_ACTIONS
    {
        return Err(V8BindingError::Unsupported {
            interface: "EventTarget".into(),
            member: "QuotaExceededError: default-action queue capacity".into(),
        });
    }
    Ok(())
}

fn check_submission_capacity_for_owner(
    state: &V8BindingState,
    owner: &V8BindingState,
    form: NodeId,
) -> Result<(), V8BindingError> {
    if state.pending_default_actions.borrow().iter().any(
        |action| matches!(action, PendingDefaultAction::Submit(old) if same_form(old, owner, form)),
    ) {
        return Ok(());
    }
    check_capacity(state)
}

pub struct AcceptedLinkNavigation {
    pub realm: RealmId,
    pub operation: Option<crate::navigation_api::NativeNavigationToken>,
    pub destination: Option<crate::navigation_api::NavigationHistorySnapshot>,
    pub script_authority: Option<crate::navigation::ScriptNavigationAdmission>,
}

pub struct LinkNavigation {
    pub audit: Option<std::sync::Arc<dyn crate::navigation::HyperlinkAudit>>,
    pub document_source: Option<std::sync::Arc<oryn_dom::DocumentProvenance>>,
    pub admission: Option<AcceptedLinkNavigation>,
    pub url: String,
    pub target: String,
    pub referrer_policy: Option<String>,
    pub noopener: bool,
    pub download: Option<String>,
    pub navigation_target: SubmissionTarget,
    pub navigation_source: crate::navigation_api::NativeNavigationSource,
}

pub struct RootedTarget {
    pub navigation: Option<LinkNavigation>,
    pub node: NodeId,
    pub user_initiated: bool,
    _lease: Rc<oryn_dom::TrackedNodeLease>,
}

impl RootedTarget {
    /// Identity-only observation; custody and release stay with this target.
    pub(super) fn native_lease(&self) -> Rc<oryn_dom::TrackedNodeLease> {
        self._lease.clone()
    }
    pub(super) fn new(state: &V8BindingState, node: NodeId) -> Result<Self, V8BindingError> {
        let lease = Rc::new(oryn_dom::TrackedNodeLease::new(&state.dom, node)?);
        Ok(Self {
            node,
            user_initiated: false,
            _lease: lease,
            navigation: None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionSource {
    Submit,
    RequestSubmit,
    Activation,
}

pub(super) struct SubmissionPlan {
    form: Rc<oryn_dom::TrackedNodeLease>,
    pending: Cell<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubmissionTarget {
    Parent(SelectedNavigationTarget),
    Denied,
    Root {
        realm: RealmId,
        generation: u64,
    },
    Frame {
        parent: RealmId,
        parent_generation: u64,
        frame: NodeId,
        connection: u64,
    },
    New,
}

fn parent_state(state: &Rc<V8BindingState>) -> Option<Rc<V8BindingState>> {
    let (parent, frame, connection) = state.frame_viewport_parent.borrow().clone()?;
    let parent = parent.upgrade()?;
    let live = !parent.execution_retired()
        && connection.is_some()
        && parent.dom.borrow().frame_context_generation(frame) == connection
        && parent.dom.borrow().is_connected(frame).unwrap_or(false);
    live.then_some(parent)
}

fn capture_target_state(state: &Rc<V8BindingState>) -> SubmissionTarget {
    if let Some((parent, frame, Some(connection))) = state.frame_viewport_parent.borrow().clone()
        && let Some(parent) = parent.upgrade()
    {
        return SubmissionTarget::Frame {
            parent: parent.realm(),
            parent_generation: parent.document_generation(),
            frame,
            connection,
        };
    }
    SubmissionTarget::Root {
        realm: state.realm(),
        generation: state.document_generation(),
    }
}

fn capture_target(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    name: &str,
) -> Result<SubmissionTarget, V8BindingError> {
    let mut top = state.clone();
    let mut ancestors = BTreeSet::new();
    while let Some(parent) = parent_state(&top) {
        if !ancestors.insert(top.realm()) {
            return Err(V8BindingError::Allocation("form target parent cycle"));
        }
        top = parent;
    }
    match name.to_ascii_lowercase().as_str() {
        "" | "_self" => return Ok(capture_target_state(state)),
        "_parent" => {
            return Ok(capture_target_state(
                &parent_state(state).unwrap_or_else(|| state.clone()),
            ));
        }
        "_top" => return Ok(capture_target_state(&top)),
        "_blank" => return Ok(SubmissionTarget::New),
        _ => {}
    }
    let states = scope
        .get_slot::<BindingStateRegistry>()
        .map(|registry| {
            registry
                .realms
                .borrow()
                .values()
                .filter_map(RcWeak::upgrade)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec![state.clone()]);
    let mut visited = BTreeSet::new();
    for root in [state.clone(), top] {
        let mut pending = vec![root];
        while let Some(owner) = pending.pop() {
            if owner.execution_retired() || !visited.insert(owner.realm()) {
                continue;
            }
            if visited.len() > 4096 {
                return Err(V8BindingError::Allocation("form target tree capacity"));
            }
            if owner.native_window_target_name() == name {
                return Ok(capture_target_state(&owner));
            }
            let frames = {
                let dom = owner.dom.borrow();
                let mut nodes = vec![owner.document_root()];
                let mut frames = Vec::new();
                while let Some(node) = nodes.pop() {
                    let record = dom.arena().get(node)?;
                    if dom.is_element(node, "http://www.w3.org/1999/xhtml", "iframe")
                        || dom.is_element(node, "http://www.w3.org/1999/xhtml", "frame")
                    {
                        frames.push(node);
                    }
                    let shadow = dom.attached_shadow_root(node, false)?;
                    nodes.extend(
                        record
                            .children
                            .iter()
                            .rev()
                            .copied()
                            .filter(|child| Some(*child) != shadow),
                    );
                    if let Some(shadow) = shadow {
                        nodes.push(shadow);
                    }
                }
                frames
            };
            for frame in frames.into_iter().rev() {
                if let Some(child) = states.iter().find(|child| {
                    child.frame_viewport_parent.borrow().as_ref().is_some_and(
                        |(parent, candidate, _)| {
                            *candidate == frame
                                && parent
                                    .upgrade()
                                    .is_some_and(|parent| Rc::ptr_eq(&parent, &owner))
                        },
                    ) && parent_state(child).is_some()
                        && !child.execution_retired()
                }) {
                    pending.push(child.clone());
                }
            }
        }
    }
    Ok(SubmissionTarget::New)
}

fn select_target(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    document: NodeId,
    name: &str,
    url: &str,
    noopener: bool,
) -> Result<SubmissionTarget, V8BindingError> {
    let effective = effective_navigation_target(name, noopener);
    let target = capture_target(scope, state, effective)?;
    if !matches!(target, SubmissionTarget::New) {
        return Ok(target);
    }
    // A new noopener context has no existing identity to retain. Select it only
    // after GET serialization has finalized the URL (or POST has its resource),
    // so an exact prerender candidate can be activated without an extra popup.
    if noopener && effective.eq_ignore_ascii_case("_blank") {
        return Ok(SubmissionTarget::New);
    }
    let Some(select) = state.navigation_target_selector.get() else {
        return Ok(target);
    };
    let source = state
        .document_provenance(document)?
        .ok_or(V8BindingError::Allocation(
            "navigation source provenance unavailable",
        ))?;
    Ok(match select(scope, state, &source, name, url, noopener)? {
        Some(receipt) => SubmissionTarget::Parent(receipt),
        None => SubmissionTarget::Denied,
    })
}

pub struct SubmissionTransaction {
    pub form: NodeId,
    pub form_data: v8::Global<v8::Object>,
    pub submitter: Option<NodeId>,
    pub document: oryn_dom::DocumentId,
    pub document_generation: u64,
    pub navigation_target: SubmissionTarget,
    pub origin_unloaded: bool,
    pub source: SubmissionSource,
    pub user_initiated: bool,
    pub navigation_source: crate::navigation_api::NativeNavigationSource,
    pub navigation: LocationNavigation,
    pub target: String,
    pub noopener: bool,
    plan: Rc<SubmissionPlan>,
    _form: RootedTarget,
    _submitter: Option<RootedTarget>,
    _document: RootedTarget,
}

impl SubmissionTransaction {
    /// The planned task uses the form's node document at execution time,
    /// independently of the original submission document and frozen payload.
    pub fn current_form_owner(&self) -> (Rc<RefCell<oryn_dom::DomRuntime>>, NodeId) {
        self._form._lease.current()
    }

    /// Clear the form's planned-task slot before navigation begins. Extracted
    /// host batches retain this receipt so later submissions can cancel them.
    pub fn start_planned_navigation(&self) -> bool {
        self.plan.pending.replace(false)
    }
}

pub(super) fn cancel_planned(state: &V8BindingState) {
    for record in state.planned_submissions.borrow_mut().drain(..) {
        if let Some(record) = record.upgrade() {
            record.pending.set(false);
        }
    }
}

pub(super) fn queue(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    form: NodeId,
    submitter: Option<NodeId>,
    source: SubmissionSource,
) -> Result<(), V8BindingError> {
    queue_with_involvement(scope, state, form, submitter, source, false)
}

pub(super) fn queue_with_involvement(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    form: NodeId,
    submitter: Option<NodeId>,
    source: SubmissionSource,
    user_initiated: bool,
) -> Result<(), V8BindingError> {
    let document = owner_document_for_node(state, form)?;
    queue_from_origin(
        scope,
        state,
        document,
        state,
        form,
        submitter.map(|node| (state.clone(), node)),
        source,
        user_initiated,
    )
}

fn queue_from_origin(
    scope: &mut v8::PinScope,
    origin: &Rc<V8BindingState>,
    origin_document: NodeId,
    state: &Rc<V8BindingState>,
    form: NodeId,
    submitter_owner: Option<(Rc<V8BindingState>, NodeId)>,
    source: SubmissionSource,
    user_initiated: bool,
) -> Result<(), V8BindingError> {
    let Some(mut queue) = Queue::new(
        scope,
        origin,
        origin_document,
        state,
        form,
        submitter_owner,
        source,
        user_initiated,
    )?
    else {
        return Ok(());
    };
    {
        let mut jobs = origin.form_submission_jobs.borrow_mut();
        if jobs.queue.len() >= MAX_PENDING_ACTIONS {
            return Err(V8BindingError::Allocation("retained form submission limit"));
        }
        jobs.queue
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("retained form submission queue"))?;
    }
    let result = queue.poll(scope);
    if !matches!(result, Ok(true)) && queue.retry_safe() {
        let mut jobs = origin.form_submission_jobs.borrow_mut();
        jobs.queue.push_back(queue);
        jobs.retry = Some(Instant::now() + Duration::from_millis(10));
    }
    result.map(|_| ())
}
#[cfg(test)]
thread_local! {static FAIL_FORM_BODY:Cell<bool>=const{Cell::new(false)};}
struct PreparedNavigation {
    owner: Rc<V8BindingState>,
    document: NodeId,
    url: String,
    transaction: Box<SubmissionTransaction>,
    method: String,
    enctype: String,
    selected: bool,
    body_ready: bool,
}
enum Prepared {
    None,
    Dialog(dialog::Job),
    Navigation(PreparedNavigation),
}
pub(super) struct Queue {
    origin: Rc<V8BindingState>,
    origin_document: NodeId,
    data_owner: Rc<V8BindingState>,
    identity: oryn_dom::DocumentId,
    form_object: v8::Global<v8::Object>,
    source_object: v8::Global<v8::Object>,
    form_lease: RootedTarget,
    submitter_lease: Option<RootedTarget>,
    document_lease: RootedTarget,
    encoding: &'static encoding_rs::Encoding,
    creation: Option<form_data::Creation>,
    data: Option<v8::Global<v8::Object>>,
    source: SubmissionSource,
    user_initiated: bool,
    prepared: Option<Prepared>,
    done: bool,
    terminal: Option<String>,
}
fn lease_copy(lease: &RootedTarget) -> RootedTarget {
    RootedTarget {
        navigation: None,
        node: lease.node,
        user_initiated: lease.user_initiated,
        _lease: lease._lease.clone(),
    }
}
impl Queue {
    pub(super) fn new(
        scope: &mut v8::PinScope,
        origin: &Rc<V8BindingState>,
        origin_document: NodeId,
        state: &Rc<V8BindingState>,
        form: NodeId,
        submitter_owner: Option<(Rc<V8BindingState>, NodeId)>,
        source: SubmissionSource,
        user_initiated: bool,
    ) -> Result<Option<Self>, V8BindingError> {
        let Some(active_owner) = active_document_state_for_node(scope, state, form) else {
            return Ok(None);
        };
        let state = &active_owner;
        if !origin.forms_allowed.get()
            || FormOperation::EntryList.active(scope, state, form)
            || !state.dom.borrow().is_connected(form)?
        {
            return Ok(None);
        }
        check_submission_capacity_for_owner(origin, state, form)?;
        let document = owner_document_for_node(state, form)?;
        let identity = origin.dom.borrow().arena().document_id(origin_document)?;
        let form_lease = RootedTarget::new(state, form)?;
        let submitter_lease = submitter_owner
            .as_ref()
            .map(|(owner, node)| RootedTarget::new(owner, *node))
            .transpose()?;
        let document_lease = RootedTarget::new(origin, origin_document)?;
        let form_object = wrap_node(scope, state, form)?;
        let source_object = match &submitter_owner {
            Some((owner, node)) => wrap_node(scope, owner, *node)?,
            None => form_object,
        };
        let source_object = v8::Global::new(scope, source_object);
        let submitter = submitter_owner
            .as_ref()
            .filter(|(owner, _)| Rc::ptr_eq(&owner.dom, &state.dom))
            .map(|(_, node)| *node);
        let encoding = {
            let dom = state.dom.borrow();
            dom.attribute_in_null_namespace(form, "accept-charset")?
                .into_iter()
                .flat_map(str::split_ascii_whitespace)
                .find_map(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
                .unwrap_or(hyperlinks::document_encoding(state, document)?)
                .output_encoding()
        };

        let creation =
            form_data::Creation::new(scope, state, Some(form), submitter, None, encoding)?;
        Ok(Some(Self {
            origin: origin.clone(),
            origin_document,
            data_owner: state.clone(),
            identity,
            form_object: v8::Global::new(scope, form_object),
            source_object,
            form_lease,
            submitter_lease,
            document_lease,
            encoding,
            creation: Some(creation),
            data: None,
            source,
            user_initiated,
            prepared: None,
            done: false,
            terminal: None,
        }))
    }
    fn prepare(&self, scope: &mut v8::PinScope) -> Result<Prepared, V8BindingError> {
        let origin = &self.origin;
        let origin_document = self.origin_document;
        let state = &self.data_owner;
        let identity = self.identity;
        let form_object = v8::Local::new(scope, &self.form_object);
        let source_object = self.source_object.clone();
        let form_lease = lease_copy(&self.form_lease);
        let submitter_lease = self.submitter_lease.as_ref().map(lease_copy);
        let document_lease = lease_copy(&self.document_lease);
        let data = v8::Local::new(scope, self.data.as_ref().unwrap());
        let source = self.source;
        let user_initiated = self.user_initiated;

        // HTML rechecks navigability, not equality with the original document.
        // Keep the source document separately while following the live form.
        let Some((current_owner, form)) = node_identity::argument(scope, form_object.into())?
        else {
            return Ok(Prepared::None);
        };
        let Some(active_owner) = active_document_state_for_node(scope, &current_owner, form) else {
            return Ok(Prepared::None);
        };
        let state = &active_owner;
        if state.execution_retired() || !state.dom.borrow().is_connected(form)? {
            return Ok(Prepared::None);
        }
        let current_submitter = submitter_lease.as_ref().map(|lease| lease._lease.current());
        let submitter = current_submitter.as_ref().map(|(_, node)| *node);
        let parameter_form = match &current_submitter {
            Some((owner, node)) => owner
                .borrow()
                .form_owner(*node)?
                .map(|form| (owner.clone(), form)),
            None => Some((state.dom.clone(), form)),
        };
        // Author callbacks have completed. Parameter reads below are native and do
        // not invoke replaceable form properties or FormData iteration methods.
        let (method, action, enctype, target, no_referrer) = {
            let dom = state.dom.borrow();
            let override_parameter = |name: &str| -> Result<Option<String>, V8BindingError> {
                current_submitter
                    .as_ref()
                    .map(|(owner, node)| {
                        let dom = owner.borrow();
                        if !dom.is_submit_button(*node)? {
                            return Ok(None);
                        }
                        dom.submitter_parameter(*node, name)
                            .map(|value| value.map(str::to_owned))
                    })
                    .transpose()
                    .map(Option::flatten)
                    .map_err(Into::into)
            };
            let parameter = |override_name: &str, name: &str| -> Result<String, V8BindingError> {
                let inherited = parameter_form
                    .as_ref()
                    .map(|(owner, node)| {
                        owner
                            .borrow()
                            .attribute_in_null_namespace(*node, name)
                            .map(|value| value.map(str::to_owned))
                    })
                    .transpose()?
                    .flatten();
                Ok(override_parameter(override_name)?
                    .or(inherited)
                    .unwrap_or_default())
            };
            (
                parameter("formmethod", "method")?,
                parameter("formaction", "action")?,
                parameter("formenctype", "enctype")?,
                match override_parameter("formtarget")? {
                    Some(target) => normalize_target(&target),
                    None => match &parameter_form {
                        Some((owner, node)) => {
                            let dom = owner.borrow();
                            let document = dom
                                .arena()
                                .node_document(*node)?
                                .ok_or(DomError::StaleNode(*node))?;
                            submission_target(&dom, document, *node, None)?
                        }
                        None => String::new(),
                    },
                },
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
                let return_value = current_submitter
                    .as_ref()
                    .map(|(owner, node)| -> Result<Option<String>, V8BindingError> {
                        let dom = owner.borrow();
                        if dom.is_element(*node, "http://www.w3.org/1999/xhtml", "input")
                            && dom
                                .attribute_in_null_namespace(*node, "type")?
                                .is_some_and(|value| value.eq_ignore_ascii_case("image"))
                        {
                            let (x, y) = dom.arena().get(*node)?.image_coordinates;
                            Ok(Some(format!("{x},{y}")))
                        } else if dom.is_submit_button(*node)? {
                            Ok(Some(dom.submitter_value(*node)?))
                        } else {
                            Ok(None)
                        }
                    })
                    .transpose()?
                    .flatten();
                return Ok(Prepared::Dialog(dialog::Job::new(
                    state,
                    dialog,
                    dialog::Action::Close,
                    return_value,
                    None,
                )?));
            }
            return Ok(Prepared::None);
        }
        let source_local = v8::Local::new(scope, &source_object);
        let (submitter_state, submitter_node) =
            node_identity::argument(scope, source_local.into())?
                .ok_or(V8BindingError::Allocation("submission source element"))?;
        let submitter_document = owner_document_for_node(&submitter_state, submitter_node)?;
        let (base, url_encoding) =
            hyperlinks::document_base(scope, &submitter_state, submitter_document)?;
        let url = if action.is_empty() {
            Ok(origin.location_url.borrow().clone())
        } else {
            hyperlinks::parse(&action, Url::parse(&base).ok().as_ref(), url_encoding)
        };
        let Ok(url) = url else {
            return Ok(Prepared::None);
        };
        // Preserve the parsed action separately from query/body preparation. The
        // retained admission stage must use this URL when selecting the navigable.
        let noopener = {
            let dom = state.dom.borrow();
            target_noopener(
                &target,
                dom.attribute_in_null_namespace(form, "rel")?.unwrap_or(""),
            )
        };
        let selection_document = owner_document_for_node(state, form)?;
        let selection_url = url.as_str().to_owned();
        let navigation_target = SubmissionTarget::Denied;
        let is_post =
            method.eq_ignore_ascii_case("post") && matches!(url.scheme(), "http" | "https");
        let source_element = submitter.unwrap_or(form);
        let source_window = origin
            .global_object
            .to_local(scope)
            .ok_or(V8BindingError::Allocation("submission source window"))?;
        let navigation_source = crate::navigation_api::NativeNavigationSource {
            execution_observer: None,
            source_origin_key: None,
            source_domain_key: crate::navigation_api::document_domain_access_key(
                origin,
                origin_document,
            )?,
            source_element: Some(source_element),
            source_element_object: Some(source_object),
            source_window: Some(v8::Global::new(scope, source_window)),
            user_initiated,
            form_data: is_post.then(|| v8::Global::new(scope, data)),
            download: None,
        };
        let plan = Rc::new(SubmissionPlan {
            form: form_lease._lease.clone(),
            pending: Cell::new(true),
        });
        let transaction = SubmissionTransaction {
            form,
            submitter,
            document: identity,
            document_generation: origin.document_generation(),
            navigation_target,
            origin_unloaded: !origin.document_completely_loaded(),
            source,
            user_initiated,
            navigation_source,
            form_data: v8::Global::new(scope, data),
            navigation: LocationNavigation {
                execution_observer: None,
                navigation_operation: None,
                download: None,
                navigation_destination: None,
                script_authority: None,
                form_authority: None,
                url: url.into(),
                replace: false,
                reload: false,
                post: None,
                referrer_policy: no_referrer.then(|| "no-referrer".into()),
            },
            target,
            noopener,
            plan: plan.clone(),
            _form: form_lease,
            _submitter: submitter_lease,
            _document: document_lease,
        };

        Ok(Prepared::Navigation(PreparedNavigation {
            owner: state.clone(),
            document: selection_document,
            url: selection_url,
            transaction: Box::new(transaction),
            method,
            enctype,
            selected: false,
            body_ready: false,
        }))
    }
    fn prepare_body(
        scope: &mut v8::PinScope,
        data_state: &Rc<V8BindingState>,
        data: v8::Local<v8::Object>,
        encoding: &'static encoding_rs::Encoding,
        prepared: &mut PreparedNavigation,
    ) -> Result<(), V8BindingError> {
        #[cfg(test)]
        if FAIL_FORM_BODY.with(|flag| flag.replace(false)) {
            return Err(V8BindingError::Allocation("injected form body preparation"));
        }
        let method = &prepared.method;
        let enctype = &prepared.enctype;
        let mut url = Url::parse(&prepared.url)
            .map_err(|_| V8BindingError::Allocation("retained form action URL"))?;
        let encode: url::form_urlencoded::EncodingOverride<'_> =
            Some(&|text| encoding.encode(text).0);
        let post =
            if method.eq_ignore_ascii_case("post") && matches!(url.scheme(), "http" | "https") {
                let (bytes, content_type) = if enctype.eq_ignore_ascii_case("multipart/form-data") {
                    form_data_body_encoded(scope, data_state, data.into(), encoding)?
                        .ok_or(V8BindingError::JavaScriptException)?
                } else {
                    let pairs = form_data::name_value_pairs(scope, data_state, data)?;
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
                if !method.eq_ignore_ascii_case("post")
                    && matches!(url.scheme(), "http" | "https" | "data")
                {
                    let pairs = form_data::name_value_pairs(scope, data_state, data)?;
                    let query = url::form_urlencoded::Serializer::new(String::new())
                        .encoding_override(encode)
                        .extend_pairs(pairs)
                        .finish();
                    url.set_query(Some(&query));
                }
                None
            };
        // Publish both outputs only after complete serialization; a failure
        // retains the selected navigable and frozen parameter snapshot.
        prepared.transaction.navigation.url = url.into();
        prepared.transaction.navigation.post = post;
        prepared.body_ready = true;
        Ok(())
    }
    pub(super) fn retry_safe(&self) -> bool {
        self.terminal.is_none()
    }
    pub(super) fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        if self.done {
            return Ok(true);
        }
        if let Some(error) = &self.terminal {
            return Err(V8BindingError::Message(error.clone()));
        }
        if self.origin.execution_suspended() || scope.is_execution_terminating() {
            return Ok(false);
        }
        if self.data.is_none() {
            let Some(data) = self.creation.as_mut().unwrap().poll(scope)? else {
                return Ok(false);
            };
            self.data = Some(data);
            self.creation = None;
        }
        if self.prepared.is_none() {
            self.prepared = Some(self.prepare(scope)?);
        }
        match self.prepared.as_mut().unwrap() {
            Prepared::None => {
                self.done = true;
                return Ok(true);
            }
            Prepared::Dialog(job) => {
                if !job.poll(scope)? {
                    return Ok(false);
                }
                self.done = true;
                return Ok(true);
            }
            Prepared::Navigation(prepared) => {
                if !prepared.selected {
                    let target = match select_target(
                        scope,
                        &prepared.owner,
                        prepared.document,
                        &prepared.transaction.target,
                        &prepared.url,
                        prepared.transaction.noopener,
                    ) {
                        Ok(target) => target,
                        Err(error) => {
                            self.terminal = Some(error.to_string());
                            return Err(error);
                        }
                    };
                    prepared.transaction.navigation_target = target;
                    prepared.selected = true;
                }
                if matches!(
                    prepared.transaction.navigation_target,
                    SubmissionTarget::Denied
                ) {
                    self.done = true;
                    return Ok(true);
                }
                if !prepared.body_ready {
                    let data = v8::Local::new(scope, self.data.as_ref().unwrap());
                    Self::prepare_body(scope, &self.data_owner, data, self.encoding, prepared)?;
                }
                let origin = &self.origin;
                let state = &prepared.owner;
                let form = prepared.transaction.form;
                check_submission_capacity_for_owner(origin, state, form)?;
                origin
                    .pending_default_actions
                    .borrow_mut()
                    .try_reserve(1)
                    .map_err(|_| V8BindingError::Allocation("planned submission queue"))?;
                {
                    let mut records = origin.planned_submissions.borrow_mut();
                    records.retain(|record| {
                        record.upgrade().is_some_and(|record| record.pending.get())
                    });
                    records
                        .try_reserve(1)
                        .map_err(|_| V8BindingError::Allocation("planned submission receipt"))?;
                }
                let mut owners = Vec::new();
                if let Some(registry) = scope.get_slot::<BindingStateRegistry>() {
                    let instances = registry.instances.borrow();
                    owners.try_reserve(instances.len()).map_err(|_| {
                        V8BindingError::Allocation("submission supersession owners")
                    })?;
                    owners.extend(instances.values().filter_map(RcWeak::upgrade));
                } else {
                    owners.push(origin.clone());
                }
                let Prepared::Navigation(prepared) = self.prepared.take().unwrap() else {
                    unreachable!()
                };
                let transaction = prepared.transaction;
                let plan = &transaction.plan;
                for owner in owners {
                    owner.planned_submissions.borrow_mut().retain(|record| {
                        let Some(record) = record.upgrade() else {
                            return false;
                        };
                        if record.form.identity() == plan.form.identity() {
                            record.pending.set(false)
                        }
                        record.pending.get()
                    });
                    owner.pending_default_actions.borrow_mut().retain(|action|!matches!(action,PendingDefaultAction::Submit(old)if !old.plan.pending.get()));
                }
                origin
                    .planned_submissions
                    .borrow_mut()
                    .push(Rc::downgrade(plan));
                origin
                    .pending_default_actions
                    .borrow_mut()
                    .push_back(PendingDefaultAction::Submit(transaction));
                self.done = true;
                Ok(true)
            }
        }
    }
    pub(super) fn cancel(&mut self) {
        self.creation = None;
        self.prepared = None;
        self.done = true;
    }
}
impl Drop for Queue {
    fn drop(&mut self) {
        self.cancel()
    }
}
#[derive(Default)]
pub(super) struct QueueRegistry {
    queue: VecDeque<Queue>,
    retry: Option<Instant>,
    polling: bool,
}
pub(super) fn queue_delay(state: &V8BindingState) -> Option<Duration> {
    let jobs = state.form_submission_jobs.borrow();
    (!jobs.queue.is_empty() && !state.execution_suspended()).then(|| {
        jobs.retry.map_or(Duration::ZERO, |deadline| {
            deadline.saturating_duration_since(Instant::now())
        })
    })
}
pub(super) fn poll_queues(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if state.form_submission_jobs.borrow().polling
        || !queue_delay(state).is_some_and(|delay| delay.is_zero())
    {
        return Ok(0);
    }
    let Some(mut job) = state.form_submission_jobs.borrow_mut().queue.pop_front() else {
        return Ok(0);
    };
    state.form_submission_jobs.borrow_mut().polling = true;
    struct Guard<'a>(&'a V8BindingState);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.form_submission_jobs.borrow_mut().polling = false
        }
    }
    let _guard = Guard(state);
    match job.poll(scope) {
        Ok(true) => {
            state.form_submission_jobs.borrow_mut().retry = None;
            Ok(1)
        }
        result => {
            if job.retry_safe() {
                let mut jobs = state.form_submission_jobs.borrow_mut();
                jobs.queue.push_front(job);
                jobs.retry = Some(Instant::now() + Duration::from_millis(10));
            }
            result.map(|_| 0)
        }
    }
}
pub(super) fn retire_queues(state: &V8BindingState) {
    let jobs = std::mem::take(&mut state.form_submission_jobs.borrow_mut().queue);
    drop(jobs);
}

/// SVG href uses the unprefixed attribute when present, including empty values.
fn activation_href(dom: &oryn_dom::DomRuntime, node: NodeId) -> Result<Option<&str>, DomError> {
    if let Some(href) = dom.attribute_in_null_namespace(node, "href")? {
        return Ok(Some(href));
    }
    if dom.is_element(node, "http://www.w3.org/2000/svg", "a") {
        return dom.attribute_ns(node, Some("http://www.w3.org/1999/xlink"), "href");
    }
    Ok(None)
}

/// Capture accepted activation parameters before returning to author script.
/// The host consumes this transaction without consulting mutable DOM attributes.
fn freeze_link(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    user_initiated: bool,
    suffix: Option<&str>,
    target_override: Option<&str>,
) -> Result<Option<LinkNavigation>, V8BindingError> {
    if state.execution_retired() {
        return Ok(None);
    }
    // The dispatch activation record selected this element before callbacks.
    // Its href may change during callbacks, but a different ancestor must not
    // acquire activation when the selected element loses href or moves.
    let node = target;
    if activation_href(&state.dom.borrow(), node)?.is_none() {
        return Ok(None);
    }
    let document = owner_document_for_node(state, node)?;
    // Detached elements in the active document can navigate; elements in an
    // inert DOMParser/createHTMLDocument document cannot.
    if document != state.document_root() {
        return Ok(None);
    }
    let (base, encoding) = hyperlinks::document_base(scope, state, document)?;
    let (raw, target, policy, rel, download) = {
        let dom = state.dom.borrow();
        (
            activation_href(&dom, node)?.unwrap_or("").to_owned(),
            submission_target(&dom, document, node, None)?,
            dom.attribute_in_null_namespace(node, "referrerpolicy")?
                .map(str::to_owned),
            dom.attribute_in_null_namespace(node, "rel")?
                .unwrap_or("")
                .to_owned(),
            dom.attribute_in_null_namespace(node, "download")?
                .map(str::to_owned),
        )
    };
    let target = target_override.map(str::to_owned).unwrap_or(target);
    let Ok(url) = hyperlinks::parse(&raw, Url::parse(&base).ok().as_ref(), encoding) else {
        return Ok(None);
    };
    let noopener = target_noopener(&target, &rel);
    let object = wrap_node(scope, state, node)?;
    let source_element_object = Some(v8::Global::new(scope, object));
    let window = state
        .global_object
        .to_local(scope)
        .ok_or(V8BindingError::Allocation("link source window"))?;
    let navigation_source = crate::navigation_api::NativeNavigationSource {
        execution_observer: None,
        source_origin_key: None,
        source_domain_key: crate::navigation_api::document_domain_access_key(state, document)?,
        source_element: Some(node),
        source_element_object,
        source_window: Some(v8::Global::new(scope, window)),
        user_initiated,
        form_data: None,
        download: download.clone(),
    };
    let mut url: String = url.into();
    if let Some(suffix) = suffix {
        url.push_str(suffix);
    }
    let document_source = state.document_provenance(document)?;
    let pings = {
        let dom = state.dom.borrow();
        let raw = dom.attribute_in_null_namespace(node, "ping")?.unwrap_or("");
        if raw.len() > 64 * 1024 {
            Vec::new()
        } else {
            raw.split_ascii_whitespace()
                .take(16)
                .filter_map(|input| {
                    hyperlinks::parse(input, Url::parse(&base).ok().as_ref(), encoding).ok()
                })
                .filter(|url| {
                    matches!(url.scheme(), "http" | "https")
                        && url.username().is_empty()
                        && url.password().is_none()
                })
                .map(String::from)
                .collect::<Vec<_>>()
        }
    };
    // Auditing is optional; failed native preparation must not cancel following
    // the hyperlink. Its reserved completion lease still discards uncertain plans.
    let audit = if pings.is_empty() {
        None
    } else if let Some(prepare) = state.hyperlink_audit_preparer.get() {
        prepare(scope, state, &url, &pings).unwrap_or(None)
    } else {
        None
    };
    let mut navigation_source = navigation_source;
    if let Some(audit) = &audit {
        let accepted = audit.clone();
        let terminal = audit.clone();
        navigation_source.execution_observer = Some(
            crate::navigation::NavigationExecutionObserver::with_admission(
                move || accepted.accepted(),
                move |_| terminal.discarded(),
            ),
        );
    }
    Ok(Some(LinkNavigation {
        audit,
        document_source,
        admission: None,
        url,
        referrer_policy: link_referrer_policy(policy.as_deref(), &rel),
        noopener,
        target,
        download,
        navigation_target: SubmissionTarget::Denied,
        navigation_source,
    }))
}
#[path = "submissions_link.rs"]
mod link;
pub(super) use link::LinkActivation;

#[cfg(test)]
pub(super) fn queue_click(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<(), V8BindingError> {
    let _ = scope;
    check_capacity(state)?;
    let target = RootedTarget::new(state, node)?;
    state
        .pending_default_actions
        .borrow_mut()
        .push_back(PendingDefaultAction::Click(target));
    Ok(())
}
pub(super) fn queue_click_with_involvement(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    user_initiated: bool,
    suffix: Option<&str>,
) -> Result<(), V8BindingError> {
    if let Some(mut job) = LinkActivation::new(scope, state, node, user_initiated, suffix, None)? {
        if !job.poll(scope)? {
            return Err(V8BindingError::JavaScriptException);
        }
    }
    Ok(())
}

/// Host entry, independent of replaceable page globals and prototypes.
pub fn request_submit(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    form: NodeId,
    submitter: Option<NodeId>,
) -> Result<(), V8BindingError> {
    request_submit_with_involvement(
        scope,
        state,
        form,
        submitter,
        SubmissionSource::RequestSubmit,
        false,
    )
}

pub(super) fn request_submit_with_involvement(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    form: NodeId,
    submitter: Option<NodeId>,
    source: SubmissionSource,
    user_initiated: bool,
) -> Result<(), V8BindingError> {
    // Validation dispatches author callbacks, which may detach nodes and force GC.
    let _form = RootedTarget::new(state, form)?;
    let document = owner_document_for_node(state, form)?;
    let _document = RootedTarget::new(state, document)?;
    let form_object = wrap_node(scope, state, form)?;
    let submitter_object = submitter
        .map(|node| wrap_node(scope, state, node))
        .transpose()?;
    let _submitter = submitter
        .map(|node| RootedTarget::new(state, node))
        .transpose()?;
    if validation::request_submit(scope, state, form, submitter)? {
        let Some((owner, form)) = node_identity::argument(scope, form_object.into())? else {
            return Ok(());
        };
        let submitter = submitter_object
            .map(|object| node_identity::argument(scope, object.into()))
            .transpose()?
            .flatten();
        queue_from_origin(
            scope,
            state,
            document,
            &owner,
            form,
            submitter,
            source,
            user_initiated,
        )?;
    }
    Ok(())
}

/// Native activation retains validation separately from planned-task admission.
pub(super) struct Request {
    origin: Rc<V8BindingState>,
    document: NodeId,
    _document: RootedTarget,
    form: v8::Global<v8::Object>,
    submitter: Option<v8::Global<v8::Object>>,
    validation: Option<retained_validation::Submit>,
    queue: Option<Queue>,
    source: SubmissionSource,
    user_initiated: bool,
    phase: u8,
    terminal: Option<String>,
    done: bool,
}
impl Request {
    pub(super) fn new(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        form: NodeId,
        submitter: Option<NodeId>,
        source: SubmissionSource,
        user_initiated: bool,
    ) -> Result<Self, V8BindingError> {
        let document = owner_document_for_node(state, form)?;
        let document_lease = RootedTarget::new(state, document)?;
        let object = wrap_node(scope, state, form)?;
        let object = v8::Global::new(scope, object);
        let submitter_object = submitter
            .map(|node| wrap_node(scope, state, node).map(|object| v8::Global::new(scope, object)))
            .transpose()?;
        let validation = retained_validation::Submit::new(scope, state, form, submitter)?;
        Ok(Self {
            origin: state.clone(),
            document,
            _document: document_lease,
            form: object,
            submitter: submitter_object,
            validation: Some(validation),
            queue: None,
            source,
            user_initiated,
            phase: 0,
            terminal: None,
            done: false,
        })
    }
    pub(super) fn retry_safe(&self) -> bool {
        self.terminal.is_none() && self.queue.as_ref().is_none_or(Queue::retry_safe)
    }
    pub(super) fn poll(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        if let Some(error) = &self.terminal {
            return Err(V8BindingError::Message(error.clone()));
        }
        if self.done {
            return Ok(true);
        }
        if self.phase == 0 {
            let Some(accepted) = self.validation.as_mut().unwrap().poll(scope)? else {
                return Ok(false);
            };
            self.validation = None;
            if !accepted {
                self.done = true;
                return Ok(true);
            }
            self.phase = 1;
        }
        if self.queue.is_none() {
            let object = v8::Local::new(scope, &self.form);
            let Some((owner, form)) = node_identity::argument(scope, object.into())? else {
                self.done = true;
                return Ok(true);
            };
            let submitter = self
                .submitter
                .as_ref()
                .map(|object| {
                    let object = v8::Local::new(scope, object);
                    node_identity::argument(scope, object.into())
                })
                .transpose()?
                .flatten();
            self.queue = Queue::new(
                scope,
                &self.origin,
                self.document,
                &owner,
                form,
                submitter,
                self.source,
                self.user_initiated,
            )?;
            if self.queue.is_none() {
                self.done = true;
                return Ok(true);
            }
        }
        if !self.queue.as_mut().unwrap().poll(scope)? {
            return Ok(false);
        }
        self.queue = None;
        self.done = true;
        Ok(true)
    }
    pub(super) fn cancel(&mut self) {
        if let Some(validation) = &mut self.validation {
            validation.cancel();
        }
        self.queue = None;
        self.done = true;
    }
}
impl Drop for Request {
    fn drop(&mut self) {
        self.cancel()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_request_retains_formdata_callbacks_and_single_submission() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<form id=f action='https://forms.test/next'><input name=x value=one></form>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(236), 1).unwrap();
        let code=v8::String::new(scope,"globalThis.log=[];f.addEventListener('formdata',e=>{globalThis.original=e;log.push('first');e.formData.append('y','two')},{once:true});f.addEventListener('formdata',e=>{log.push(e===original?'second':'wrong');e.formData.append('z','three')})").unwrap();
        v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let form = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "f")
            .unwrap()
            .unwrap();
        let mut request = Request::new(
            scope,
            &state,
            form,
            None,
            SubmissionSource::RequestSubmit,
            false,
        )
        .unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(request.poll(scope).is_err());
        assert!(state.take_pending_default_actions().is_empty());
        assert!(request.retry_safe());
        assert!(request.poll(scope).unwrap());
        assert!(request.poll(scope).unwrap());
        let actions = state.take_pending_default_actions();
        assert_eq!(actions.len(), 1);
        let PendingDefaultAction::Submit(submit) = actions.into_iter().next().unwrap() else {
            panic!("expected native submission")
        };
        assert_eq!(
            submit.navigation.url,
            "https://forms.test/next?x=one&y=two&z=three"
        );
        let code = v8::String::new(scope, "log.join()==='first,second'").unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }

    #[test]
    fn supersession_cancels_submission_already_extracted_by_host() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<form id=f action='https://forms.test/next'><input name=x value=one></form>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let form = dom.borrow().find_by_id("f").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        queue(scope, &state, form, None, SubmissionSource::Submit).unwrap();
        let mut extracted = state.take_pending_default_actions();
        queue(scope, &state, form, None, SubmissionSource::Submit).unwrap();
        let PendingDefaultAction::Submit(old) = extracted.pop().unwrap() else {
            panic!("submission expected");
        };
        assert!(!old.start_planned_navigation());
        let PendingDefaultAction::Submit(current) =
            state.take_pending_default_actions().pop().unwrap()
        else {
            panic!("submission expected");
        };
        assert!(current.start_planned_navigation());
        assert!(!current.start_planned_navigation());
        queue(scope, &state, form, None, SubmissionSource::Submit).unwrap();
        assert_eq!(state.planned_submissions.borrow().len(), 1);
        let PendingDefaultAction::Submit(stopped) =
            state.take_pending_default_actions().pop().unwrap()
        else {
            panic!("submission expected");
        };
        state.stop_pending_document_load().unwrap();
        assert!(!stopped.start_planned_navigation());
        queue(scope, &state, form, None, SubmissionSource::Submit).unwrap();
        assert_eq!(state.planned_submissions.borrow().len(), 1);
    }

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
    fn stop_cancels_planned_submissions_but_allows_later_submission() {
        crate::v8_bindings::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<form id=f action='/next'><button id=b></button></form>")
                .into_runtime()
                .unwrap(),
        ));
        let form = dom.borrow().find_by_id("f").unwrap();
        let button = dom.borrow().find_by_id("b").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 7).unwrap();
        state.commit_location("https://example.test/start").unwrap();
        queue(
            scope,
            &state,
            form,
            Some(button),
            SubmissionSource::Activation,
        )
        .unwrap();
        queue_click(scope, &state, button).unwrap();
        assert!(
            state
                .pending_default_actions
                .borrow()
                .iter()
                .any(|action| matches!(action, PendingDefaultAction::Submit(_)))
        );
        let navigation = LocationNavigation {
            execution_observer: None,
            navigation_operation: None,
            download: None,
            navigation_destination: None,
            url: "https://example.test/other".into(),
            replace: false,
            reload: false,
            post: None,
            referrer_policy: None,
            form_authority: None,
            script_authority: None,
        };
        state.schedule_navigation(navigation.clone());
        state.stop_pending_document_load().unwrap();
        assert!(state.take_location_navigation().is_none());
        assert_eq!(state.pending_default_actions.borrow().len(), 1);
        assert!(matches!(
            state.pending_default_actions.borrow().front(),
            Some(PendingDefaultAction::Click(_))
        ));
        assert!(!state.execution_retired());
        queue(
            scope,
            &state,
            form,
            Some(button),
            SubmissionSource::Activation,
        )
        .unwrap();
        assert!(matches!(
            state.pending_default_actions.borrow().back(),
            Some(PendingDefaultAction::Submit(_))
        ));
        state.schedule_navigation(navigation);
        assert_eq!(
            state.take_location_navigation().unwrap().url,
            "https://example.test/other"
        );
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
            queue_click(scope, &state, button).unwrap();
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
        assert!(queue_click(scope, &state, button).is_err());
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

#[cfg(test)]
mod relationship_tests {
    use super::*;
    #[test]
    fn noreferrer_overrides_explicit_policy_as_a_complete_rel_token() {
        assert_eq!(
            link_referrer_policy(Some("unsafe-url"), "opener NoReFeRrEr"),
            Some("no-referrer".into())
        );
        assert_eq!(
            link_referrer_policy(Some(""), "noreferrer"),
            Some("no-referrer".into())
        );
        assert_eq!(
            link_referrer_policy(None, "noreferrer"),
            Some("no-referrer".into())
        );
        assert_eq!(
            link_referrer_policy(Some("origin"), "not-noreferrer"),
            Some("origin".into())
        );
        assert_eq!(link_referrer_policy(None, "noopener"), None);
    }
    #[test]
    fn native_rel_tokens_preserve_blank_default_and_explicit_opener_precedence() {
        assert!(target_noopener("_BLANK", ""));
        assert!(!target_noopener("_blank", "opener"));
        assert!(target_noopener("named", "NOOPENER opener"));
        assert!(target_noopener("named", "noreferrer opener"));
        assert!(!target_noopener("named", ""));
        assert!(!target_noopener("_self", "not-noopener"));
    }
}

#[cfg(test)]
mod parent_selection_tests {
    use super::*;
    #[test]
    fn noopener_blank_defers_new_context_selection_until_get_or_post_is_finalized() {
        fn premature(
            _scope: &mut v8::PinScope,
            _state: &V8BindingState,
            _source: &oryn_dom::DocumentProvenance,
            _name: &str,
            _url: &str,
            _noopener: bool,
        ) -> Result<Option<SelectedNavigationTarget>, V8BindingError> {
            panic!("new noopener context selected before finalized form body")
        }
        super::super::tests::initialize_v8();
        for method in ["get", "post"] {
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let html = format!(
                "<form id=f action='/form?old=query' target=_BLANK method={method}><input name=x value=one></form>"
            );
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse(&html).into_runtime().unwrap(),
            ));
            let state = install_generated_bindings(scope, context, dom, RealmId(245), 1).unwrap();
            state
                .commit_location("https://selection.test/start")
                .unwrap();
            install_source(&state);
            state.set_navigation_target_selector(premature);
            let form = state.dom.borrow().find_by_id("f").unwrap();
            let mut queue = Queue::new(
                scope,
                &state,
                state.document_root(),
                &state,
                form,
                None,
                SubmissionSource::Submit,
                false,
            )
            .unwrap()
            .unwrap();
            FAIL_FORM_BODY.with(|flag| flag.set(true));
            assert!(queue.poll(scope).is_err());
            assert!(queue.retry_safe());
            assert!(state.pending_default_actions.borrow().is_empty());
            let text=v8::String::new(scope,"f.action='/changed';f.target='changed';f.method='dialog';f.firstElementChild.value='two'").unwrap();
            v8::Script::compile(scope, text, None)
                .unwrap()
                .run(scope)
                .unwrap();
            assert!(queue.poll(scope).unwrap());
            assert!(queue.poll(scope).unwrap());
            let actions = state.pending_default_actions.borrow();
            assert_eq!(actions.len(), 1);
            let PendingDefaultAction::Submit(transaction) = actions.front().unwrap() else {
                panic!("missing transaction")
            };
            assert!(matches!(
                transaction.navigation_target,
                SubmissionTarget::New
            ));
            assert!(transaction.noopener);
            assert_eq!(transaction.target, "_BLANK");
            if method == "get" {
                assert_eq!(
                    transaction.navigation.url,
                    "https://selection.test/form?x=one"
                );
                assert!(transaction.navigation.post.is_none());
            } else {
                assert_eq!(
                    transaction.navigation.url,
                    "https://selection.test/form?old=query"
                );
                assert_eq!(
                    transaction.navigation.post.as_ref().unwrap().bytes,
                    b"x=one"
                );
            }
        }
    }
    #[test]
    fn body_retry_keeps_selected_target_and_frozen_form_parameters() {
        thread_local! {static SELECTIONS:Cell<usize>=const{Cell::new(0)};}
        fn counted(
            scope: &mut v8::PinScope,
            state: &V8BindingState,
            source: &oryn_dom::DocumentProvenance,
            name: &str,
            url: &str,
            noopener: bool,
        ) -> Result<Option<SelectedNavigationTarget>, V8BindingError> {
            SELECTIONS.with(|count| count.set(count.get() + 1));
            select(scope, state, source, name, url, noopener)
        }
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<form id=f action=/form target=receipt method=post><input name=x value=one></form>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(244), 1).unwrap();
        state
            .commit_location("https://selection.test/start")
            .unwrap();
        install_source(&state);
        state.set_navigation_target_selector(counted);
        SELECTIONS.with(|count| count.set(0));
        let form = state.dom.borrow().find_by_id("f").unwrap();
        let mut queue = Queue::new(
            scope,
            &state,
            state.document_root(),
            &state,
            form,
            None,
            SubmissionSource::Submit,
            false,
        )
        .unwrap()
        .unwrap();
        FAIL_FORM_BODY.with(|flag| flag.set(true));
        assert!(queue.poll(scope).is_err());
        assert!(queue.retry_safe());
        SELECTIONS.with(|count| assert_eq!(count.get(), 1));
        assert!(state.pending_default_actions.borrow().is_empty());
        let source=v8::String::new(scope,"f.action='/changed';f.target='changed';f.method='get';f.enctype='text/plain';f.firstElementChild.value='two'").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        assert!(queue.poll(scope).unwrap());
        assert!(queue.poll(scope).unwrap());
        SELECTIONS.with(|count| assert_eq!(count.get(), 1));
        let actions = state.pending_default_actions.borrow();
        assert_eq!(actions.len(), 1);
        let PendingDefaultAction::Submit(transaction) = actions.front().unwrap() else {
            panic!("missing form transaction")
        };
        assert_eq!(transaction.target, "receipt");
        assert_eq!(transaction.navigation.url, "https://selection.test/form");
        let post = transaction.navigation.post.as_ref().unwrap();
        assert_eq!(post.bytes, b"x=one");
        assert_eq!(post.content_type, "application/x-www-form-urlencoded");
    }
    fn select(
        _scope: &mut v8::PinScope,
        _state: &V8BindingState,
        source: &oryn_dom::DocumentProvenance,
        name: &str,
        url: &str,
        noopener: bool,
    ) -> Result<Option<SelectedNavigationTarget>, V8BindingError> {
        assert_eq!(source.capability, b"test-native-provenance");
        if name == "deny" {
            return Ok(None);
        }
        Ok(Some(SelectedNavigationTarget {
            receipt: crate::navigation::NavigationTargetReceipt(
                serde_json::to_vec(&(name, url, noopener)).unwrap(),
            ),
            page_id: 7,
            local_realm: None,
            local_frame: None,
        }))
    }
    fn install_source(state: &V8BindingState) {
        state.set_document_provenance_capture(Rc::new(|_, _, _| {
            Ok(oryn_dom::DocumentProvenance {
                capability: b"test-native-provenance".to_vec(),
                origin: "https://selection.test".into(),
                origin_key: "tuple:https://selection.test".into(),
                inert: false,
            })
        }));
        state.set_navigation_target_selector(select);
    }
    #[test]
    fn remote_link_is_dispatched_during_activation_without_a_second_default_action() {
        fn dispatch(
            scope: &mut v8::PinScope,
            selected: &SelectedNavigationTarget,
            source: &oryn_dom::DocumentProvenance,
            url: &str,
            policy: Option<&str>,
            user: bool,
            _audit: Option<crate::navigation::HyperlinkAuditToken>,
        ) -> Result<(), V8BindingError> {
            assert_eq!(selected.page_id, 7);
            assert_eq!(source.capability, b"test-native-provenance");
            assert_eq!(url, "https://selection.test/link");
            assert_eq!(policy, Some("no-referrer"));
            assert!(!user);
            let text = v8::String::new(scope, "order.push('dispatch')").unwrap();
            v8::Script::compile(scope, text, None)
                .unwrap()
                .run(scope)
                .unwrap();
            Ok(())
        }
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<a id=link href=/link target=receipt rel=noreferrer>go</a>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state
            .commit_location("https://selection.test/start")
            .unwrap();
        install_source(&state);
        state.set_selected_link_dispatcher(dispatch);
        let source=v8::String::new(scope,"globalThis.order=['before'];document.getElementById('link').click();order.push('after');document.getElementById('link').addEventListener('click',e=>e.preventDefault());document.getElementById('link').click();order.join(',')").unwrap();
        let result = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        assert_eq!(result.to_rust_string_lossy(scope), "before,dispatch,after");
        assert!(state.pending_default_actions.borrow().is_empty());
    }
    #[test]
    fn link_and_form_keep_parent_selection_before_later_mutations() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<a id=link href=/link target=receipt>go</a><form id=f action=/form target=receipt><input name=x value=one></form>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state
            .commit_location("https://selection.test/start")
            .unwrap();
        install_source(&state);
        let source=v8::String::new(scope,"document.getElementById('link').click();document.getElementById('f').submit();document.getElementById('link').target='changed';document.getElementById('f').target='changed';document.getElementById('f').action='/later'").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let actions = state.pending_default_actions.borrow();
        assert_eq!(actions.len(), 2);
        let selected = actions
            .iter()
            .map(|action| match action {
                PendingDefaultAction::Click(click) => {
                    &click.navigation.as_ref().unwrap().navigation_target
                }
                PendingDefaultAction::Submit(form) => &form.navigation_target,
            })
            .map(|target| {
                let SubmissionTarget::Parent(receipt) = target else {
                    panic!("parent selection not retained")
                };
                serde_json::from_slice::<(String, String, bool)>(&receipt.receipt.0).unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            selected,
            vec![
                (
                    "receipt".into(),
                    "https://selection.test/link".into(),
                    false
                ),
                (
                    "receipt".into(),
                    "https://selection.test/form".into(),
                    false
                )
            ]
        );
        let PendingDefaultAction::Submit(form) = actions.back().unwrap() else {
            panic!("form transaction missing")
        };
        assert_eq!(
            form.navigation.url, "https://selection.test/form?x=one",
            "entity/query serialization follows selection without losing the payload"
        );
    }
    #[test]
    fn denied_selection_does_not_enqueue_or_supersede_an_accepted_form() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<form id=f action=/form target=receipt></form><a id=link href=/link target=deny>go</a>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state
            .commit_location("https://selection.test/start")
            .unwrap();
        install_source(&state);
        let source=v8::String::new(scope,"let f=document.getElementById('f');f.submit();f.target='deny';f.submit();document.getElementById('link').click()").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let actions = state.pending_default_actions.borrow();
        assert_eq!(actions.len(), 1);
        let PendingDefaultAction::Submit(form) = actions.front().unwrap() else {
            panic!("accepted form missing")
        };
        assert!(form.plan.pending.get());
        assert_eq!(form.target, "receipt");
    }
}

#[cfg(test)]
mod target_domstring_tests {
    use super::*;
    #[test]
    fn target_and_rel_policy_classify_utf16_units_without_altering_names() {
        let encoded = |text: &[u16]| oryn_dom::dom_string::from_units(text);
        let attack = encoded(&[0xd800, b'<' as u16, b'\n' as u16]);
        assert_eq!(normalize_target(&attack), "_blank");
        for delimiter in [9, 10, 13] {
            assert_eq!(
                normalize_target(&encoded(&[0xdfff, b'<' as u16, delimiter])),
                "_blank"
            );
        }
        for units in [
            vec![0xd800, b'<' as u16],
            vec![0xd800, 10],
            vec![0xd800, b'<' as u16, 12],
        ] {
            let name = encoded(&units);
            assert_eq!(normalize_target(&name), name);
        }
        let mut rel = "NOOPENER ".encode_utf16().collect::<Vec<_>>();
        rel.push(0xd800);
        let rel = encoded(&rel);
        assert!(target_noopener("named", &rel));
        let mut rel = "noreferrer ".encode_utf16().collect::<Vec<_>>();
        rel.push(0xdfff);
        let rel = encoded(&rel);
        assert!(target_noopener("named", &rel));
        assert_eq!(
            link_referrer_policy(Some("unsafe-url"), &rel),
            Some("no-referrer".into())
        );
        let mut rel = "opener ".encode_utf16().collect::<Vec<_>>();
        rel.push(0xd800);
        assert!(!target_noopener("_blank", &encoded(&rel)));
        assert!(!target_noopener(
            "named",
            &encoded(&[b'n' as u16, 0xd800, b'o' as u16])
        ));
    }
}
