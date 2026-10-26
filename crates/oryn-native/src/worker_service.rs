//! Parent registry access for native worker service routes. Author code never
//! chooses the storage partition, security origin, or principal being serviced.

/// Inputs for inherit frame worker policy at url.
pub(crate) struct InheritedFramePolicyInputs<'a> {
    pub(crate) page: PageId,
    pub(crate) frame: String,
    pub(crate) generation: u64,
    pub(crate) epoch: u64,
    pub(crate) parent: &'a oryn_js::message_wire::WindowTarget,
    pub(crate) parent_generation: u64,
    pub(crate) parent_epoch: u64,
    pub(crate) document_url: &'a str,
}
/// Inputs for bind frame worker policy with sandbox.
pub(crate) struct FramePolicyInputs<'a> {
    pub(crate) page: PageId,
    pub(crate) frame: String,
    pub(crate) generation: u64,
    pub(crate) epoch: u64,
    pub(crate) url: &'a str,
    pub(crate) token: &'a str,
    pub(crate) opaque_sandbox: bool,
}
/// Inputs for register activation parent.
pub(crate) struct ActivationParentInputs {
    pub(crate) page: PageId,
    pub(crate) frame: String,
    pub(crate) generation: u64,
    pub(crate) epoch: u64,
    pub(crate) parent: oryn_js::message_wire::WindowTarget,
    pub(crate) parent_generation: u64,
    pub(crate) parent_epoch: u64,
}
/// Inputs for register isolation container.
pub(crate) struct IsolationContainerInputs {
    pub(crate) page: PageId,
    pub(crate) frame: String,
    pub(crate) generation: u64,
    pub(crate) epoch: u64,
    pub(crate) allow: String,
    pub(crate) source: String,
    pub(crate) sandbox: Option<String>,
}
use super::*;
#[path = "parser_encoding_policy.rs"]
mod parser_encoding_policy;
#[cfg(unix)]
#[path = "prerender_blank_activation.rs"]
pub(super) mod prerender_blank_activation;
#[path = "prerender_policy.rs"]
pub(super) mod prerender_policy;
#[path = "window_target_names.rs"]
mod window_target_names;
use crate::agent_registry::{AgentOwner, AgentRegistry};
use oryn_js::{
    message_wire::MessageHub,
    worker_wire::{AgentBootstrap, AgentConnection, AgentId, Creator, WorkerError},
};

// Both initial host admission and later publication use the authenticated
// native policy container's precursor. Local URLs and an opaque serialized
// origin do not erase that committed secure-context input.
pub(super) fn committed_policy_is_potentially_trustworthy(
    origin: &SecurityOrigin,
    policy: Option<&oryn_js::worker_wire::AgentPolicy>,
) -> bool {
    oryn_js::realm::potentially_trustworthy_origin(origin)
        || policy.is_some_and(|policy| {
            let precursor = policy.policy_self_url.as_deref().unwrap_or(&policy.url);
            oryn_js::realm::potentially_trustworthy_origin(&SecurityOrigin::from_url(precursor))
        })
}

fn worker_lock_error(name: &str) -> WorkerError {
    WorkerError::new("InvalidStateError", format!("{name} lock poisoned"))
}

impl WorkerPolicyRegistry {
    fn poll_csp_events(
        &mut self,
        agent: AgentId,
        after: u64,
    ) -> Vec<oryn_js::platform_control::CspEvent> {
        if let Some(events) = self.agent_csp_events.get_mut(&agent) {
            while events.front().is_some_and(|event| event.id <= after) {
                events.pop_front();
            }
            events.iter().take(64).cloned().collect()
        } else {
            Vec::new()
        }
    }
    fn admit_csp_events(
        &mut self,
        agent: AgentId,
        violations: &[crate::csp::CspViolation],
    ) -> Result<(), WorkerError> {
        if violations.is_empty() {
            return Ok(());
        }
        let count = u64::try_from(violations.len())
            .map_err(|_| worker_lock_error("CSP event identity exhausted"))?;
        let last = self
            .csp_event_id
            .checked_add(count)
            .ok_or_else(|| worker_lock_error("CSP event identity exhausted"))?;
        let events = self.agent_csp_events.entry(agent).or_default();
        if events.len().saturating_add(violations.len()) > 256 {
            return Err(worker_lock_error("CSP event capacity exhausted"));
        }
        events
            .try_reserve(violations.len())
            .map_err(|_| worker_lock_error("CSP event allocation"))?;
        for (offset, violation) in violations.iter().enumerate() {
            events.push_back(oryn_js::platform_control::CspEvent {
                id: self.csp_event_id + offset as u64 + 1,
                document_uri: violation.document_uri.clone(),
                referrer: violation.referrer.clone(),
                blocked_uri: violation.worker_event_blocked_uri.as_ref()
                    .unwrap_or(&violation.blocked_uri).clone(),
                effective_directive: violation.effective_directive.clone(),
                original_policy: violation.original_policy.clone(),
                source_file: violation.source_file.clone(),
                sample: violation.sample.clone(),
                disposition: match violation.disposition {
                    crate::csp::CspDisposition::Enforce => "enforce",
                    crate::csp::CspDisposition::Report => "report",
                }
                .into(),
                status_code: violation.status_code,
                line_number: violation.line_number,
                column_number: violation.column_number,
            });
        }
        self.csp_event_id = last;
        Ok(())
    }
}

impl BrowserContext {
    pub(crate) fn report_worker_csp(
        &self,
        owner: &AgentOwner,
        policy: &crate::csp::CspPolicySet,
        violations: Vec<crate::csp::CspViolation>,
        top_level_url: &str,
        retained_job: bool,
    ) -> Result<(), WorkerError> {
        let live = self.native_worker_authority(owner);
        if !retained_job {
            live.as_ref().map_err(|error| error.clone())?;
        }
        if live.is_ok()
            && let AgentOwner::Agent(id) = owner
        {
            self.inner
                .worker_policies
                .lock()
                .map_err(|_| worker_lock_error("CSP event registry"))?
                .admit_csp_events(*id, &violations)?;
        }
        if live.is_ok()
            && let AgentOwner::Window {
                page,
                target,
                generation,
                epoch,
            } = owner
        {
            self.publish_context_event_batch(
                violations
                    .iter()
                    .map(
                        |violation| crate::context_service::ContextEvent::CspViolation {
                            target_page_id: *page,
                            target_creator: Some(oryn_js::worker_wire::Creator::Window {
                                target: target.clone(),
                                document_generation: *generation,
                                stream_epoch: *epoch,
                            }),
                            violation: violation.clone(),
                        },
                    )
                    .collect(),
            )
            .map_err(|error| WorkerError::new("QuotaExceededError", error.to_string()))?;
        }
        for violation in violations {
            // Retained service jobs may outlive a client Document. Their native
            // policy can still report, without addressing a successor Window.
            let diagnostic_page = if live.is_ok() {
                match owner {
                    AgentOwner::Window { page, .. } => Some(*page),
                    _ => None,
                }
            } else {
                None
            };
            for target in policy.report_targets(&violation) {
                self.queue_native_csp_report(
                    diagnostic_page,
                    top_level_url.into(),
                    target,
                    violation.clone(),
                )
                .map_err(|error| WorkerError::new("InvalidStateError", error.to_string()))?;
            }
        }
        if !retained_job {
            self.native_worker_authority(owner)?;
        }
        Ok(())
    }
    pub(crate) fn poll_agent_csp_events(
        &self,
        agent: AgentId,
        after: u64,
    ) -> Result<Vec<oryn_js::platform_control::CspEvent>, WorkerError> {
        self.worker_bootstrap(agent)?;
        let events = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| worker_lock_error("CSP event registry"))?
            .poll_csp_events(agent, after);
        self.worker_bootstrap(agent)?;
        Ok(events)
    }
}
impl BrowserContext {
    #[cfg(test)]
    pub(crate) fn native_media_element_delegated(
        &self,
        owner: &AgentOwner,
    ) -> Result<bool, WorkerError> {
        self.native_capture_features_delegated(owner, &["camera", "microphone"])
    }
    pub(crate) fn native_install_invalid_reason(
        &self,
        owner: &AgentOwner,
    ) -> Result<String, WorkerError> {
        let (_, _, policy) = self.native_worker_authority(owner)?;
        if crate::csp::CspPolicySet::from_agent_policy(&policy)
            .sandbox()
            .is_some()
        {
            return Ok("illegal_sandbox".into());
        }
        Ok(if matches!(
            owner,
            AgentOwner::Window {
                target: oryn_js::message_wire::WindowTarget::Root,
                ..
            }
        ) {
            ""
        } else {
            "illegal_subframe"
        }
        .into())
    }
    pub(crate) fn native_capture_features_delegated(
        &self,
        owner: &AgentOwner,
        features: &[&str],
    ) -> Result<bool, WorkerError> {
        if matches!(
            owner,
            AgentOwner::Window {
                target: oryn_js::message_wire::WindowTarget::Root,
                ..
            }
        ) {
            return Ok(true);
        }
        self.native_worker_authority(owner)?;
        let policies = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| worker_lock_error("media container policy"))?;
        let Some((allow, _, _)) = policies.isolation_containers.get(owner) else {
            return Ok(false);
        };
        Ok(features.iter().all(|feature| {
            allow
                .split(';')
                .any(|directive| directive.split_ascii_whitespace().next() == Some(*feature))
        }))
    }
    /// Evaluate the committed native document ancestry, never author-provided
    /// URLs or a mutable iframe attribute read in the consuming realm.
    pub(crate) fn native_permissions_policy_snapshot(
        &self,
        owner: &AgentOwner,
    ) -> Result<oryn_js::permissions_policy::Snapshot, WorkerError> {
        use oryn_js::permissions_policy::{Constraint, Snapshot};
        let (_, origin, policy) = self.native_worker_authority(owner)?;
        let fields = |headers: &[(String, String)]| {
            headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("permissions-policy"))
                .cloned()
                .collect::<Vec<_>>()
        };
        let mut result = Snapshot {
            origin: origin.serialized.clone(),
            container: false,
            constraints: vec![Constraint::Header {
                origin: origin.serialized,
                headers: fields(&policy.headers),
            }],
        };
        let mut current = owner.clone();
        let mut seen = std::collections::BTreeSet::new();
        loop {
            if !seen.insert(current.clone()) || seen.len() > 256 {
                return Err(WorkerError::new("SecurityError", "invalid policy ancestry"));
            }
            match &current {
                AgentOwner::Window {
                    target: oryn_js::message_wire::WindowTarget::Root,
                    ..
                } => break,
                AgentOwner::Window { .. } => {}
                AgentOwner::Agent(_) => {
                    return Err(WorkerError::new(
                        "SecurityError",
                        "policy inspection requires Window",
                    ));
                }
            }
            let (_, child, _) = self.native_worker_authority(&current)?;
            let (parent, container) = {
                let policies = self
                    .inner
                    .worker_policies
                    .lock()
                    .map_err(|_| worker_lock_error("policy inspection"))?;
                (
                    policies.activation_parents.get(&current).cloned(),
                    policies.isolation_containers.get(&current).cloned(),
                )
            };
            let (Some(parent), Some((allow, source, _))) = (parent, container) else {
                return Err(WorkerError::new(
                    "SecurityError",
                    "policy ancestor unavailable",
                ));
            };
            let (_, origin, policy) = self.native_worker_authority(&parent)?;
            result.constraints.push(Constraint::Gate {
                origin: origin.serialized.clone(),
                target: origin.serialized.clone(),
                headers: fields(&policy.headers),
            });
            result.constraints.push(Constraint::Gate {
                origin: origin.serialized.clone(),
                target: child.serialized.clone(),
                headers: fields(&policy.headers),
            });
            result.constraints.push(Constraint::GateContainer {
                origin: origin.serialized,
                source,
                allow,
                target: child.serialized,
            });
            current = parent;
        }
        self.native_worker_authority(owner)?;
        if !result.validate() {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "policy snapshot bounds",
            ));
        }
        Ok(result)
    }
    pub(crate) fn native_platform_allowed(
        &self,
        owner: &AgentOwner,
        feature: &str,
    ) -> Result<bool, WorkerError> {
        let mut current = owner.clone();
        let mut visited = std::collections::BTreeSet::new();
        loop {
            if !visited.insert(current.clone()) || visited.len() > 256 {
                return Err(WorkerError::new(
                    "SecurityError",
                    "invalid native permission ancestry",
                ));
            }
            if !self.worker_secure_context(&current)? {
                return Ok(false);
            }
            let (_, origin, policy) = self.native_worker_authority(&current)?;
            if origin.opaque
                || (current == *owner
                    && !crate::isolation_permissions::allows_feature(
                        &policy.headers,
                        feature,
                        &origin.serialized,
                        &origin.serialized,
                    ))
            {
                return Ok(false);
            }
            match &current {
                AgentOwner::Window {
                    target: oryn_js::message_wire::WindowTarget::Root,
                    ..
                } => return Ok(true),
                AgentOwner::Window { .. } => {
                    let (parent, container) = {
                        let policies = self
                            .inner
                            .worker_policies
                            .lock()
                            .map_err(|_| worker_lock_error("permission ancestry"))?;
                        (
                            policies.activation_parents.get(&current).cloned(),
                            policies.isolation_containers.get(&current).cloned(),
                        )
                    };
                    let (Some(parent), Some((allow, source, _))) = (parent, container) else {
                        return Ok(false);
                    };
                    let (_, parent_origin, parent_policy) =
                        self.native_worker_authority(&parent)?;
                    if !crate::isolation_permissions::allows_feature(
                        &parent_policy.headers,
                        feature,
                        &parent_origin.serialized,
                        &origin.serialized,
                    ) || !crate::isolation_permissions::container_allows_feature(
                        &allow,
                        feature,
                        &parent_origin.serialized,
                        &source,
                        &origin.serialized,
                    ) {
                        return Ok(false);
                    }
                    current = parent;
                }
                AgentOwner::Agent(_) => return Ok(false),
            }
        }
    }
    pub(crate) fn page_isolation_policy(
        &self,
        page: PageId,
    ) -> Result<crate::agent_isolation::DocumentIsolationPolicy, RuntimeError> {
        Ok(self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| RuntimeError::ContextService("isolation policy lock poisoned".into()))?
            .root_isolation
            .get(&page)
            .copied()
            .unwrap_or_default())
    }
    pub(crate) fn document_reporting_policy(
        &self,
        page: PageId,
    ) -> Result<crate::coep_reporting::Policy, RuntimeError> {
        let url = self.page_url(page)?;
        let policies = self.inner.worker_policies.lock().map_err(|_| {
            RuntimeError::ContextService("reporting policy registry poisoned".into())
        })?;
        Ok(crate::coep_reporting::Policy::new(
            &url,
            policies
                .root_isolation_permissions
                .get(&page)
                .map_or(&[][..], Vec::as_slice),
        ))
    }
    #[cfg(all(test, feature = "v8-host"))]
    pub(crate) fn record_document_isolation(
        &self,
        page: PageId,
        headers: &[(String, String)],
    ) -> Result<(), RuntimeError> {
        let mut policies =
            self.inner.worker_policies.lock().map_err(|_| {
                RuntimeError::ContextService("isolation policy lock poisoned".into())
            })?;
        policies.root_isolation.insert(
            page,
            crate::agent_isolation::DocumentIsolationPolicy::from_headers(headers),
        );
        policies.root_isolation_permissions.insert(
            page,
            headers
                .iter()
                .filter(|(name, _)| {
                    [
                        "permissions-policy",
                        "permissions-policy-report-only",
                        "origin-agent-cluster",
                        "cross-origin-embedder-policy",
                        "cross-origin-embedder-policy-report-only",
                        "reporting-endpoints",
                        "report-to",
                    ]
                    .iter()
                    .any(|field| name.eq_ignore_ascii_case(field))
                })
                .cloned()
                .collect(),
        );
        Ok(())
    }
    pub(crate) fn bind_shared_memory(
        &self,
        owner: &AgentOwner,
        parent: Option<oryn_js::worker_wire::Creator>,
    ) -> Result<bool, WorkerError> {
        let (_, origin, authority_policy) = self.native_worker_authority(owner)?;
        if !crate::isolation_permissions::allows(
            &authority_policy.headers,
            &origin.serialized,
            &origin.serialized,
        ) {
            return Ok(false);
        }
        if origin.opaque || !self.worker_secure_context(owner)? {
            return Ok(false);
        }
        if self
            .inner
            .group
            .shared_memory
            .lock()
            .map_err(|_| worker_lock_error("shared cluster registry"))?
            .isolated(owner)
        {
            return Ok(true);
        }
        let AgentOwner::Window {
            page,
            target,
            generation,
            epoch,
        } = owner
        else {
            return Ok(false);
        };
        let policy = {
            let policies = self
                .inner
                .worker_policies
                .lock()
                .map_err(|_| worker_lock_error("isolation policy"))?;
            match target {
                oryn_js::message_wire::WindowTarget::Root => policies
                    .prerenders
                    .get(&(*page, *generation, *epoch))
                    .and_then(|scope| scope.policy.as_ref())
                    .map(|policy| {
                        crate::agent_isolation::DocumentIsolationPolicy::from_headers(
                            &policy.headers,
                        )
                    })
                    .or_else(|| policies.root_isolation.get(page).copied()),
                oryn_js::message_wire::WindowTarget::Frame(frame) => policies
                    .frame_isolation
                    .get(&(*page, frame.clone(), *generation, *epoch))
                    .copied(),
            }
            .unwrap_or_default()
        };
        let parent = match target {
            oryn_js::message_wire::WindowTarget::Root => {
                if parent.is_some() {
                    return Err(WorkerError::new(
                        "SecurityError",
                        "root isolation parent must come from native opener registry",
                    ));
                }
                if !policy.top_level_isolated(true, true) {
                    return Ok(false);
                }
                None
            }
            oryn_js::message_wire::WindowTarget::Frame(_) => {
                let Some(oryn_js::worker_wire::Creator::Window {
                    target,
                    document_generation,
                    stream_epoch,
                }) = parent
                else {
                    return Ok(false);
                };
                let parent = self
                    .window_worker_creator(*page, &target, document_generation, stream_epoch)
                    .map_err(|error| WorkerError::new("SecurityError", error.to_string()))?
                    .0;
                if parent == *owner {
                    return Err(WorkerError::new(
                        "SecurityError",
                        "frame isolation parent cannot be itself",
                    ));
                }
                if self
                    .inner
                    .worker_policies
                    .lock()
                    .map_err(|_| worker_lock_error("frame isolation ancestry"))?
                    .activation_parents
                    .get(owner)
                    != Some(&parent)
                {
                    return Err(WorkerError::new(
                        "SecurityError",
                        "isolation parent differs from committed native ancestry",
                    ));
                }
                let (_, parent_origin, parent_policy) = self.native_worker_authority(&parent)?;
                let container = self
                    .inner
                    .worker_policies
                    .lock()
                    .map_err(|_| worker_lock_error("container policy"))?
                    .isolation_containers
                    .get(owner)
                    .cloned();
                let Some((allow, source, _)) = container else {
                    return Ok(false);
                };
                if !crate::isolation_permissions::allows(
                    &parent_policy.headers,
                    &parent_origin.serialized,
                    &origin.serialized,
                ) || !crate::isolation_permissions::container_allows(
                    &allow,
                    &parent_origin.serialized,
                    &source,
                    &origin.serialized,
                ) {
                    return Ok(false);
                }
                let isolated = self
                    .inner
                    .group
                    .shared_memory
                    .lock()
                    .map_err(|_| worker_lock_error("shared cluster registry"))?
                    .isolated(&parent);
                if !policy.embedded_isolated(true, isolated, true) {
                    return Ok(false);
                }
                if parent_origin.authority == origin.authority {
                    Some(parent)
                } else {
                    None
                }
            }
        };
        self.native_worker_authority(owner)?;
        let group = match self
            .private_prerender_group(owner)
            .map_err(|error| WorkerError::new("SecurityError", error.to_string()))?
        {
            Some(group) => group,
            None => self
                .native_window_group(*page, *page, None)
                .map_err(|error| WorkerError::new("SecurityError", error.to_string()))?
                .0
                .ok_or_else(|| WorkerError::new("SecurityError", "document group missing"))?,
        };
        let mut registry = self
            .inner
            .group
            .shared_memory
            .lock()
            .map_err(|_| worker_lock_error("shared cluster registry"))?;
        if registry.isolated(owner) {
            return Ok(true);
        }
        match parent {
            Some(parent) => registry.admit_child(&parent, owner.clone(), &origin.authority),
            None => registry.admit_document(owner.clone(), origin.authority, group),
        }
        .map_err(|error| WorkerError::new("SecurityError", error))?;
        Ok(true)
    }
    /// Called inside the already-authorized creation transaction, before the
    /// process admission gate opens. Do not reenter agent/page registries here.
    pub(crate) fn admit_worker_shared_memory(
        &self,
        bootstrap: &AgentBootstrap,
        parent: Option<&AgentOwner>,
    ) -> Result<(), WorkerError> {
        use oryn_js::worker_wire::AgentKind;
        let owner = AgentOwner::Agent(bootstrap.id);
        if !crate::agent_isolation::worker_isolated_capability(bootstrap) {
            return Ok(());
        }
        let mut registry = self
            .inner
            .group
            .shared_memory
            .lock()
            .map_err(|_| worker_lock_error("worker cluster admission"))?;
        match bootstrap.kind {
            AgentKind::Dedicated | AgentKind::AudioWorklet => {
                let Some(parent) = parent else {
                    return Err(worker_lock_error(
                        "dedicated worker requires native creator",
                    ));
                };
                if !registry.isolated(parent) {
                    return Ok(());
                }
                registry.admit_child(parent, owner, &bootstrap.origin.authority)
            }
            AgentKind::Shared | AgentKind::Service => {
                registry.admit_root(owner, bootstrap.origin.authority.clone())
            }
        }
        .map_err(|error| WorkerError::new("SecurityError", error))
    }
    #[cfg(unix)]
    pub(crate) fn shared_memory_request(
        &self,
        owner: &AgentOwner,
        request: oryn_js::shared_memory::SharedMemoryRequest,
    ) -> Result<oryn_js::shared_memory::SharedMemoryResponse, WorkerError> {
        use oryn_js::shared_memory::{SharedMemoryRequest, SharedMemoryResponse};
        if request.requires_current_document() {
            self.native_worker_authority(owner)?;
        }
        if matches!(request, SharedMemoryRequest::CloneOrigin) {
            use sha2::Digest;
            let (_, origin, _) = self.native_worker_authority(owner)?;
            let identity: [u8; 32] = sha2::Sha256::digest(origin.authority.as_bytes()).into();
            return Ok(SharedMemoryResponse {
                isolated: false,
                outcome: oryn_js::shared_memory::SharedMemoryOutcome::CloneOrigin(identity),
                descriptor: None,
                native_descriptor: None,
            });
        }
        if let SharedMemoryRequest::Bind { parent } = request {
            return Ok(SharedMemoryResponse {
                isolated: self.bind_shared_memory(owner, parent)?,
                outcome: Default::default(),
                descriptor: None,
                native_descriptor: None,
            });
        }
        let mut registry = self
            .inner
            .group
            .shared_memory
            .lock()
            .map_err(|_| worker_lock_error("shared memory registry"))?;
        let isolated = registry.isolated(owner);
        let result = match request {
            SharedMemoryRequest::Bind { .. } | SharedMemoryRequest::CloneOrigin => {
                unreachable!("handled native authority admission")
            }
            SharedMemoryRequest::Status => SharedMemoryResponse {
                isolated,
                outcome: Default::default(),
                descriptor: None,
                native_descriptor: None,
            },
            SharedMemoryRequest::Offer { descriptor } => {
                let reference = registry
                    .offer_shared(owner, descriptor)
                    .map_err(|error| WorkerError::new("DataCloneError", error))?;
                SharedMemoryResponse {
                    isolated,
                    outcome: oryn_js::shared_memory::SharedMemoryOutcome::Reference(reference),
                    descriptor: None,
                    native_descriptor: None,
                }
            }
            SharedMemoryRequest::CommitOffers { references } => {
                registry
                    .commit_shared(owner, &references)
                    .map_err(|error| WorkerError::new("DataCloneError", error))?;
                SharedMemoryResponse {
                    isolated,
                    outcome: Default::default(),
                    descriptor: None,
                    native_descriptor: None,
                }
            }
            SharedMemoryRequest::AbortOffer { reference } => {
                registry
                    .abort_shared_offer(owner, &reference)
                    .map_err(|error| WorkerError::new("DataCloneError", error))?;
                SharedMemoryResponse {
                    isolated,
                    outcome: Default::default(),
                    descriptor: None,
                    native_descriptor: None,
                }
            }
            SharedMemoryRequest::OpenDelivery { reference } => {
                let fd = registry
                    .open_shared_delivery(owner, &reference)
                    .map_err(|error| WorkerError::new("DataCloneError", error))?;
                SharedMemoryResponse {
                    isolated,
                    outcome: Default::default(),
                    descriptor: Some(reference.descriptor),
                    native_descriptor: Some(std::sync::Arc::new(fd)),
                }
            }
            SharedMemoryRequest::ReleaseDelivery { reference } => {
                registry
                    .release_shared_delivery(owner, &reference)
                    .map_err(|error| WorkerError::new("DataCloneError", error))?;
                SharedMemoryResponse {
                    isolated,
                    outcome: Default::default(),
                    descriptor: None,
                    native_descriptor: None,
                }
            }
            SharedMemoryRequest::Allocate {
                byte_length,
                maximum_length,
                granularity,
                guard_length,
                wasm,
                memory64,
            } => {
                let (descriptor, fd) = registry
                    .allocate(crate::shared_memory::SharedAllocationInputs {
                        owner,
                        byte_length,
                        maximum_length,
                        granularity,
                        guard_length,
                        wasm,
                        memory64,
                    })
                    .map_err(|error| WorkerError::new("DataCloneError", error))?;
                SharedMemoryResponse {
                    isolated,
                    outcome: Default::default(),
                    descriptor: Some(descriptor),
                    native_descriptor: Some(std::sync::Arc::new(fd)),
                }
            }
            SharedMemoryRequest::Open { descriptor } => {
                let fd = registry
                    .open(owner, descriptor)
                    .map_err(|error| WorkerError::new("DataCloneError", error))?;
                SharedMemoryResponse {
                    isolated,
                    outcome: Default::default(),
                    descriptor: Some(descriptor),
                    native_descriptor: Some(std::sync::Arc::new(fd)),
                }
            }
            SharedMemoryRequest::GrowAbsolute { token, length } => {
                let previous = registry
                    .grow_absolute(owner, token, length)
                    .map_err(|error| WorkerError::new("RangeError", error))?;
                SharedMemoryResponse {
                    isolated,
                    outcome: oryn_js::shared_memory::SharedMemoryOutcome::Length(previous),
                    descriptor: None,
                    native_descriptor: None,
                }
            }
            SharedMemoryRequest::GrowByPages {
                token,
                pages,
                maximum_pages,
            } => {
                let previous = registry
                    .grow_pages(owner, token, pages, maximum_pages)
                    .map_err(|error| WorkerError::new("RangeError", error))?;
                SharedMemoryResponse {
                    isolated,
                    outcome: oryn_js::shared_memory::SharedMemoryOutcome::Length(previous),
                    descriptor: None,
                    native_descriptor: None,
                }
            }
            SharedMemoryRequest::WaitAdmission {
                id,
                descriptor,
                acquire,
                offset,
                width64,
                expected,
                timeout_ns,
            } => {
                let wait = registry
                    .admit_wait(crate::shared_memory::WaitAdmissionInputs {
                        owner,
                        id,
                        descriptor,
                        acquire,
                        offset,
                        width64,
                        expected,
                        timeout_ns,
                    })
                    .map_err(|error| WorkerError::new("InvalidStateError", error))?;
                SharedMemoryResponse {
                    isolated,
                    outcome: oryn_js::shared_memory::SharedMemoryOutcome::Wait(wait),
                    descriptor: None,
                    native_descriptor: None,
                }
            }
            SharedMemoryRequest::CancelAdmission { id } => {
                registry
                    .cancel_wait_admission(owner, id)
                    .map_err(|error| WorkerError::new("InvalidStateError", error))?;
                SharedMemoryResponse {
                    isolated,
                    outcome: Default::default(),
                    descriptor: None,
                    native_descriptor: None,
                }
            }
            SharedMemoryRequest::WaitEnqueue {
                token,
                offset,
                width64,
                expected,
                timeout_ns,
            } => {
                let wait = registry
                    .wait_enqueue(owner, token, offset, width64, expected, timeout_ns)
                    .map_err(|error| WorkerError::new("RangeError", error))?;
                SharedMemoryResponse {
                    isolated,
                    outcome: oryn_js::shared_memory::SharedMemoryOutcome::Wait(wait),
                    descriptor: None,
                    native_descriptor: None,
                }
            }
            SharedMemoryRequest::WaitPoll { id } => {
                let wait = registry
                    .wait_result(owner, id, false)
                    .map_err(|error| WorkerError::new("InvalidStateError", error))?;
                SharedMemoryResponse {
                    isolated,
                    outcome: oryn_js::shared_memory::SharedMemoryOutcome::Wait(wait),
                    descriptor: None,
                    native_descriptor: None,
                }
            }
            SharedMemoryRequest::WaitCancel { id } => {
                let wait = registry
                    .wait_result(owner, id, true)
                    .map_err(|error| WorkerError::new("InvalidStateError", error))?;
                SharedMemoryResponse {
                    isolated,
                    outcome: oryn_js::shared_memory::SharedMemoryOutcome::Wait(wait),
                    descriptor: None,
                    native_descriptor: None,
                }
            }
            SharedMemoryRequest::Notify {
                token,
                offset,
                count,
            } => {
                let count = registry
                    .notify(owner, token, offset, count)
                    .map_err(|error| WorkerError::new("RangeError", error))?;
                SharedMemoryResponse {
                    isolated,
                    outcome: oryn_js::shared_memory::SharedMemoryOutcome::Count(count),
                    descriptor: None,
                    native_descriptor: None,
                }
            }
            SharedMemoryRequest::Release { token } => {
                registry
                    .release(owner, token)
                    .map_err(|error| WorkerError::new("DataCloneError", error))?;
                SharedMemoryResponse {
                    isolated,
                    outcome: Default::default(),
                    descriptor: None,
                    native_descriptor: None,
                }
            }
        };
        Ok(result)
    }
    pub(crate) fn retire_shared_memory(&self, owner: &AgentOwner) {
        self.inner
            .group
            .shared_memory
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .retire(owner);
    }

    pub(crate) fn worker_bootstrap(&self, id: AgentId) -> Result<AgentBootstrap, WorkerError> {
        self.inner
            .agents
            .lock()
            .map_err(|_| worker_lock_error("agent registry"))?
            .bootstrap_snapshot(id)
    }

    /// Resolve the original owner from the random client capability before
    /// validating its current native document lifetime. A page route cannot
    /// act for another page, nor can an agent route act for a Window client.
    pub(crate) fn worker_connection_owner(
        &self,
        source: AgentOwner,
        connection: &AgentConnection,
    ) -> Result<AgentOwner, WorkerError> {
        let owner = self
            .inner
            .agents
            .lock()
            .map_err(|_| worker_lock_error("agent registry"))?
            .connection_owner(connection)?;
        self.validate_worker_client_owner(source, &owner)?;
        Ok(owner)
    }
    pub(crate) fn worker_release_owner(
        &self,
        source: AgentOwner,
        connection: &AgentConnection,
    ) -> Result<Option<AgentOwner>, WorkerError> {
        let owner = self
            .inner
            .agents
            .lock()
            .map_err(|_| worker_lock_error("agent registry"))?
            .release_owner(connection)?;
        if let Some(owner) = &owner {
            self.validate_worker_client_owner(source, owner)?;
        }
        Ok(owner)
    }
    fn validate_worker_client_owner(
        &self,
        source: AgentOwner,
        owner: &AgentOwner,
    ) -> Result<(), WorkerError> {
        let source_matches = match (&source, owner) {
            (AgentOwner::Window { page: a, .. }, AgentOwner::Window { page: b, .. }) => a == b,
            (AgentOwner::Agent(a), AgentOwner::Agent(b)) => a == b,
            _ => false,
        };
        if !source_matches {
            return Err(WorkerError::new(
                "SecurityError",
                "foreign worker client route",
            ));
        }
        match owner {
            AgentOwner::Window {
                page,
                target,
                generation,
                epoch,
            } => {
                self.window_worker_creator(*page, target, *generation, *epoch)
                    .map_err(|error| WorkerError::new("InvalidStateError", error.to_string()))?;
            }
            AgentOwner::Agent(id) => {
                self.worker_bootstrap(*id)?;
            }
        }
        Ok(())
    }

    /// Always extract failed/retired processes while locked, then interrupt and
    /// join with no registry or hub locks held. Retain failed teardown ownership
    /// so shutdown can retry instead of dropping an unaccounted process.
    /// Consume an exact terminal receipt without running unrelated cleanup after
    /// the acknowledgement. The caller can then release its Version owner.
    pub(crate) fn acknowledge_service_retirement(&self, agent: AgentId) -> Result<(), WorkerError> {
        let mut registry = self
            .inner
            .agents
            .lock()
            .map_err(|_| worker_lock_error("agent registry"))?;
        if registry.service_retirement_status(agent)?
            != crate::agent_registry::ServiceRetirementStatus::Retired
        {
            return Err(WorkerError::new(
                "InvalidStateError",
                "service process teardown remains pending",
            ));
        }
        registry.acknowledge_service_retirement(agent)
    }

    pub(crate) fn dispatch_worker_registry<T>(
        &self,
        dispatch: impl FnOnce(&mut AgentRegistry, &mut MessageHub) -> Result<T, WorkerError>,
    ) -> Result<T, WorkerError> {
        let (result, batch) = {
            let mut agents = self
                .inner
                .agents
                .lock()
                .map_err(|_| worker_lock_error("agent registry"))?;
            let mut hub = self
                .inner
                .group
                .messages
                .lock()
                .map_err(|_| worker_lock_error("message hub"))?;
            let result = dispatch(&mut agents, &mut hub);
            let batch = agents.detach_pending_retirement(&mut hub);
            (result, batch)
        };
        let mut batch = batch?;
        let cleanup = batch.terminate();
        // Recover the poisoned lock only to retain ownership. It must not make
        // an otherwise poisoned registry eligible for new admissions.
        self.inner
            .agents
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain_failed_retirement(batch);
        self.inner.agent_retirement_done.notify_all();
        match (result, cleanup) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(cleanup)) => Err(WorkerError::new(
                &error.name,
                format!("{}; teardown: {}", error.message, cleanup.message),
            )),
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::agent_registry::AgentProcess;
    use oryn_js::message_wire::{MessageRequest, WindowTarget};
    use oryn_js::worker_wire::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct ReentrantProcess {
        context: BrowserContext,
        stops: Arc<AtomicUsize>,
    }
    impl AgentProcess for ReentrantProcess {
        fn send(&mut self, _: AgentCommand) -> Result<(), WorkerError> {
            Ok(())
        }
        fn poll(&mut self, _: usize) -> Result<Vec<AgentEvent>, WorkerError> {
            Ok(Vec::new())
        }
        fn terminate(&mut self) -> Result<(), WorkerError> {
            // A real runtime can finish an in-flight broker request while the
            // caller waits for it to stop. These locks must all be available.
            drop(self.context.inner.group.pages.lock().unwrap());
            drop(self.context.inner.agents.lock().unwrap());
            drop(self.context.inner.group.messages.lock().unwrap());
            self.stops.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
    #[test]
    fn exact_document_retirement_joins_without_broker_locks_and_preserves_successor() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .commit_page_url(id, "https://example.test/".into())
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let (owner, partition, origin) = context
            .window_worker_creator(id, &WindowTarget::Root, 1, 0)
            .unwrap();
        let bootstrap = AgentBootstrap {
            cookie_ancestor_urls: Vec::new(),
            import_metadata: Default::default(),
            service_skip_waiting: false,
            resource_digests: Default::default(),
            secure_context: true,
            service_registration: None,
            service_controller: None,
            policy: oryn_js::worker_wire::AgentPolicy::default(),
            top_level_url: None,
            id: AgentId(0),
            kind: AgentKind::Dedicated,
            partition,
            origin,
            script_url: "https://example.test/worker.js".into(),
            source: Vec::new(),
            modules: Default::default(),
            options: Default::default(),
            referrer_policy: "no-referrer".into(),
            limits: Default::default(),
        };
        let stops = Arc::new(AtomicUsize::new(0));
        let probe = stops.clone();
        let child_context = context.clone();
        let connection = context
            .dispatch_worker_registry(|registry, hub| {
                registry.connect_with_ports(hub, owner.clone(), bootstrap, move |_| {
                    Ok(Box::new(ReentrantProcess {
                        context: child_context,
                        stops: probe,
                    }))
                })
            })
            .unwrap();
        assert_eq!(
            context
                .worker_connection_owner(owner.clone(), &connection)
                .unwrap(),
            owner
        );
        context
            .handle_message_wire(
                id,
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert_eq!(stops.load(Ordering::SeqCst), 1);
        assert!(context.worker_bootstrap(connection.agent).is_err());
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .window_worker_creator(id, &WindowTarget::Root, 2, 0)
                .is_ok()
        );
        assert!(
            context
                .window_worker_creator(id, &WindowTarget::Root, 1, 0)
                .is_err()
        );
    }
}

/// Parent-issued response tickets are scoped to their actual loading page.
/// Binding a ticket never accepts replacement policy text from native callers.
/// Native security publication is distinct from the child document.open counter.
/// A generation may never be rebound to another response policy, even at the same URL.
#[derive(Clone, Default)]
pub(super) struct RootDocumentLifetime {
    pub(super) initial_materialized: bool,
    pub(super) epoch: u64,
    high_epoch: u64,
    high_generation: u64,
    commitments: BTreeMap<u64, u64>,
}
impl RootDocumentLifetime {
    pub(super) fn publish(&mut self) -> Result<(), String> {
        let next = self
            .high_epoch
            .checked_add(1)
            .ok_or("root security epoch exhausted")?;
        // Initial about:blank registration can precede its first native policy.
        if self.epoch == 0 && !self.initial_materialized {
            for value in self.commitments.values_mut() {
                *value = next;
            }
        }
        self.epoch = next;
        self.high_epoch = next;
        Ok(())
    }
    pub(super) fn retain_cached_epochs(&mut self, cached: &std::collections::BTreeSet<u64>) {
        // Monotonic high_generation remains the tombstone for removed IDs.
        // Only native live/cache epochs retain authority-bearing records.
        self.commitments
            .retain(|_, epoch| *epoch == self.epoch || cached.contains(epoch));
    }
    /// Allocate an isolated native document lifetime without publishing its
    /// security epoch as the current browsing-context document.
    pub(super) fn reserve_nonactive(&mut self) -> Result<(u64, u64), String> {
        if self.commitments.len() >= 4096 {
            return Err("root lifetime commitment capacity".into());
        }
        let generation = self
            .high_generation
            .checked_add(1)
            .ok_or("root generation exhausted")?;
        let epoch = self
            .high_epoch
            .checked_add(1)
            .ok_or("root security epoch exhausted")?;
        self.commitments.insert(generation, epoch);
        self.high_generation = generation;
        self.high_epoch = epoch;
        Ok((generation, epoch))
    }
    pub(super) fn activate_nonactive(
        &mut self,
        expected_epoch: u64,
        generation: u64,
        epoch: u64,
    ) -> Result<(), String> {
        if self.epoch != expected_epoch
            || epoch == expected_epoch
            || self.commitments.get(&generation) != Some(&epoch)
        {
            return Err("nonactive root authority changed".into());
        }
        self.epoch = epoch;
        Ok(())
    }
    pub(super) fn abandon_nonactive(&mut self, generation: u64, epoch: u64) {
        if epoch != self.epoch && self.commitments.get(&generation) == Some(&epoch) {
            self.commitments.remove(&generation);
        }
    }
    pub(super) fn matches_initial_rollback(&self, expected: Option<u64>) -> bool {
        self.epoch == 0
            && expected.is_none_or(|generation| {
                self.high_generation == generation && self.commitments.get(&generation) == Some(&0)
            })
    }
    pub(super) fn reserve(&mut self) -> Result<u64, String> {
        let generation = self
            .high_generation
            .checked_add(1)
            .ok_or("root generation exhausted")?;
        self.admit(generation)?;
        Ok(generation)
    }
    pub(super) fn admit(&mut self, generation: u64) -> Result<(), String> {
        if let Some(epoch) = self.commitments.get(&generation) {
            return if *epoch == self.epoch {
                Ok(())
            } else {
                Err("root generation belongs to a retired security publication".into())
            };
        }
        if generation <= self.high_generation {
            return Err("root generation has already been retired".into());
        }
        if self.commitments.len() >= 4096 {
            return Err("root lifetime commitment capacity".into());
        }
        self.commitments.insert(generation, self.epoch);
        self.high_generation = generation;
        Ok(())
    }
    pub(super) fn current(&self, generation: u64) -> bool {
        self.commitments.get(&generation) == Some(&self.epoch)
    }
}
#[derive(Default)]
pub(super) struct WorkerPolicyRegistry {
    pub(super) prerenders: BTreeMap<(PageId, u64, u64), prerender_policy::Scope>,
    prerender_completions: BTreeMap<(PageId, u64, u64), prerender_policy::Completion>,
    #[cfg(unix)]
    blank_activations: BTreeMap<(PageId, u64, u64), prerender_blank_activation::Record>,
    parser_encoding: BTreeMap<String, parser_encoding_policy::Checkpoint>,
    pub(super) window_inventories: BTreeMap<
        PageId,
        (
            oryn_js::worker_wire::Creator,
            Vec<(
                String,
                crate::host::webdriver_script::AutomationRealmSnapshot,
            )>,
        ),
    >,
    pub(super) document_domains: document_domain::Registry,
    csp_event_id: u64,
    agent_csp_events:
        BTreeMap<AgentId, std::collections::VecDeque<oryn_js::platform_control::CspEvent>>,
    pub(super) local_documents: BTreeMap<String, PreparedLocalDocument>,
    prepared_origins: BTreeMap<String, oryn_js::worker_wire::AgentOrigin>,
    pub(super) root_lifetimes: BTreeMap<PageId, RootDocumentLifetime>,
    pub(super) root_isolation_permissions: BTreeMap<PageId, Vec<(String, String)>>,
    pub(super) root_isolation: BTreeMap<PageId, crate::agent_isolation::DocumentIsolationPolicy>,
    prepared_isolation: BTreeMap<String, crate::agent_isolation::DocumentIsolationPolicy>,
    frame_isolation:
        BTreeMap<(PageId, String, u64, u64), crate::agent_isolation::DocumentIsolationPolicy>,
    secure_contexts: BTreeMap<AgentOwner, bool>,
    #[cfg(unix)]
    service: Option<std::sync::Arc<crate::service_registry::ServiceRegistry>>,
    pub(super) blobs: BTreeMap<String, NativeBlobResource>,
    activations: BTreeMap<AgentOwner, (Instant, String, String)>,
    pub(super) activation_parents: BTreeMap<AgentOwner, AgentOwner>,
    frame_openers: BTreeMap<(PageId, String), (PageId, oryn_js::message_wire::WindowTarget)>,
    pub(super) isolation_containers: BTreeMap<AgentOwner, (String, String, Option<String>)>,
    sticky_activations: std::collections::BTreeSet<AgentOwner>,
    prepared: BTreeMap<String, (PageId, String, oryn_js::worker_wire::AgentPolicy)>,
    frames: BTreeMap<(PageId, String, u64, u64), oryn_js::worker_wire::AgentPolicy>,
    pub(super) frame_origins: BTreeMap<(PageId, String, u64, u64), (String, String)>,
    pub(super) frame_security_epochs: BTreeMap<(PageId, String, u64, u64), u64>,
    prepared_security_epochs: BTreeMap<String, u64>,
}
impl BrowserContext {
    pub(crate) fn publish_worker_secure_context(
        &self,
        owner: AgentOwner,
        secure: bool,
    ) -> Result<(), WorkerError> {
        if let AgentOwner::Agent(id) = &owner {
            if self.worker_bootstrap(*id)?.secure_context != secure {
                return Err(WorkerError::new(
                    "SecurityError",
                    "agent cannot change committed secure-context state",
                ));
            }
            return Ok(());
        }
        let (_, origin, policy) = self.native_worker_authority(&owner)?;
        let trustworthy = committed_policy_is_potentially_trustworthy(
            &SecurityOrigin::from_url(&origin.serialized),
            Some(&policy),
        );
        if secure && !trustworthy {
            return Err(WorkerError::new(
                "SecurityError",
                "insecure document cannot publish secure context",
            ));
        }
        let owners = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| worker_lock_error("secure context registry"))?
            .secure_contexts
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        let stale = owners
            .into_iter()
            .filter(|owner| self.retained_worker_authority(owner).is_err())
            .collect::<Vec<_>>();
        {
            let mut policies = self
                .inner
                .worker_policies
                .lock()
                .map_err(|_| worker_lock_error("secure context registry"))?;
            for owner in stale {
                policies.secure_contexts.remove(&owner);
            }
            if let Some(previous) = policies.secure_contexts.get(&owner)
                && *previous != secure
            {
                return Err(WorkerError::new(
                    "SecurityError",
                    "committed secure-context state changed",
                ));
            }
            if policies.secure_contexts.len() >= 4096
                && !policies.secure_contexts.contains_key(&owner)
            {
                return Err(WorkerError::new(
                    "QuotaExceededError",
                    "secure-context authority registry",
                ));
            }
            policies.secure_contexts.insert(owner.clone(), secure);
        }
        if let Err(error) = self.native_worker_authority(&owner) {
            self.inner
                .worker_policies
                .lock()
                .map_err(|_| worker_lock_error("secure context registry"))?
                .secure_contexts
                .remove(&owner);
            return Err(error);
        }
        self.register_document_domain_cluster(&owner, secure)
            .map_err(|error| WorkerError::new("SecurityError", error))?;
        Ok(())
    }
    pub(crate) fn worker_secure_context(&self, owner: &AgentOwner) -> Result<bool, WorkerError> {
        match owner {
            AgentOwner::Agent(id) => Ok(self.worker_bootstrap(*id)?.secure_context),
            AgentOwner::Window { .. } => {
                self.native_worker_authority(owner)?;
                Ok(self
                    .inner
                    .worker_policies
                    .lock()
                    .map_err(|_| worker_lock_error("secure context registry"))?
                    .secure_contexts
                    .get(owner)
                    .copied()
                    .unwrap_or(false))
            }
        }
    }
    pub(crate) fn retire_worker_secure_context(&self, owner: &AgentOwner) {
        self.inner
            .media_capture
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retire_owner(owner);
        self.inner
            .platform_control
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retire_owner(owner);
        self.retire_shared_memory(owner);
        let mut policies = self
            .inner
            .worker_policies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        policies.secure_contexts.remove(owner);
        policies.isolation_containers.remove(owner);
        policies.document_domains.documents.remove(owner);
        if let AgentOwner::Agent(id) = owner {
            policies.agent_csp_events.remove(id);
        }
    }
    pub(crate) fn register_isolation_container(
        &self,
        params: IsolationContainerInputs,
    ) -> Result<(), RuntimeError> {
        let IsolationContainerInputs {
            page,
            frame,
            generation,
            epoch,
            allow,
            source,
            sandbox,
        } = params;

        if allow.len() > 16384
            || source.len() > 16384
            || sandbox.as_ref().is_some_and(|value| value.len() > 16384)
            || url::Url::parse(&source).is_err()
        {
            return Err(RuntimeError::ProtocolViolation(
                "invalid native container policy snapshot".into(),
            ));
        }
        let owner = AgentOwner::Window {
            page,
            target: oryn_js::message_wire::WindowTarget::Frame(frame.clone()),
            generation,
            epoch,
        };
        self.native_worker_authority(&owner)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        let parent = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| RuntimeError::ContextService("container ancestry lock poisoned".into()))?
            .activation_parents
            .get(&owner)
            .cloned()
            .ok_or_else(|| {
                RuntimeError::ProtocolViolation("container policy lacks committed parent".into())
            })?;
        let (_, _, parent_policy) = self
            .native_worker_authority(&parent)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        let mut inherited_sandbox =
            crate::csp::CspPolicySet::from_agent_policy(&parent_policy).sandbox_headers();
        if let Some(value) = &sandbox {
            inherited_sandbox.push(("content-security-policy".into(), format!("sandbox {value}")));
        }
        let mut policies =
            self.inner.worker_policies.lock().map_err(|_| {
                RuntimeError::ContextService("container policy lock poisoned".into())
            })?;
        if !policies.activation_parents.contains_key(&owner) {
            return Err(RuntimeError::ProtocolViolation(
                "container policy lacks committed parent".into(),
            ));
        }
        let snapshot = (allow, source, sandbox);
        if let Some(previous) = policies.isolation_containers.get(&owner) {
            if previous != &snapshot {
                return Err(RuntimeError::ProtocolViolation(
                    "committed container policy changed".into(),
                ));
            }
            return Ok(());
        } else {
            if policies.isolation_containers.len() >= 4096 {
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "container policies".into(),
                });
            }
            let policy = policies
                .frames
                .get_mut(&(page, frame, generation, epoch))
                .ok_or_else(|| {
                    RuntimeError::ProtocolViolation("container document policy missing".into())
                })?;
            policy.headers.extend(inherited_sandbox);
            policies
                .isolation_containers
                .insert(owner.clone(), snapshot);
        }
        drop(policies);
        self.commit_deferred_container(&owner)
            .map_err(RuntimeError::ContextService)?;
        Ok(())
    }
    pub(crate) fn update_live_frame_container_policy(
        &self,
        page: PageId,
        creator: &Creator,
        parent: &Creator,
        allow: String,
        source: String,
    ) -> Result<(), RuntimeError> {
        if allow.len() > 16384 || source.len() > 16384 || url::Url::parse(&source).is_err() {
            return Err(RuntimeError::ProtocolViolation(
                "invalid live container policy".into(),
            ));
        }
        let Creator::Window {
            target: target @ oryn_js::message_wire::WindowTarget::Frame(_),
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "live container requires frame owner".into(),
            ));
        };
        let owner = self
            .window_worker_creator(page, target, *document_generation, *stream_epoch)?
            .0;
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = parent
        else {
            return Err(RuntimeError::ProtocolViolation(
                "live container requires parent Window".into(),
            ));
        };
        let parent = self
            .window_worker_creator(page, target, *document_generation, *stream_epoch)?
            .0;
        let mut policies = self.inner.worker_policies.lock().map_err(|_| {
            RuntimeError::ContextService("live container policy lock poisoned".into())
        })?;
        if policies.activation_parents.get(&owner) != Some(&parent) {
            return Err(RuntimeError::ProtocolViolation(
                "live container parent changed".into(),
            ));
        }
        let record = policies
            .isolation_containers
            .get_mut(&owner)
            .ok_or_else(|| {
                RuntimeError::ProtocolViolation("live container owner retired".into())
            })?;
        // Sandbox, origin and cluster choice are committed Document state.
        // Attribute mutation updates only the live Permissions Policy edge.
        record.0 = allow;
        record.1 = source;
        Ok(())
    }
    pub(crate) fn register_activation_parent(
        &self,
        params: ActivationParentInputs,
    ) -> Result<(), RuntimeError> {
        let ActivationParentInputs {
            page,
            frame,
            generation,
            epoch,
            parent,
            parent_generation,
            parent_epoch,
        } = params;

        let owner = AgentOwner::Window {
            page,
            target: oryn_js::message_wire::WindowTarget::Frame(frame),
            generation,
            epoch,
        };
        let parent = AgentOwner::Window {
            page,
            target: parent,
            generation: parent_generation,
            epoch: parent_epoch,
        };
        self.native_worker_authority(&owner)
            .map_err(|e| RuntimeError::ContextService(e.to_string()))?;
        self.native_worker_authority(&parent)
            .map_err(|e| RuntimeError::ContextService(e.to_string()))?;
        let mut policies = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| RuntimeError::ContextService("activation parent registry".into()))?;
        if owner == parent || activation_descendant(&policies.activation_parents, &parent, &owner) {
            return Err(RuntimeError::ProtocolViolation(
                "activation ancestry cycle".into(),
            ));
        }
        if let Some(existing) = policies.activation_parents.get(&owner) {
            if existing != &parent {
                return Err(RuntimeError::ProtocolViolation(
                    "committed activation parent changed".into(),
                ));
            }
        } else {
            if policies.activation_parents.len() >= 4096 {
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "activation parents".into(),
                });
            }
            policies.activation_parents.insert(owner, parent);
        }
        Ok(())
    }
    pub(crate) fn record_native_activation(&self, owner: AgentOwner) -> Result<(), WorkerError> {
        let AgentOwner::Window { page, .. } = &owner else {
            return Err(WorkerError::new(
                "SecurityError",
                "only Window input grants interaction",
            ));
        };
        let (_, source_origin, _) = self.native_worker_authority(&owner)?;
        let retained_candidates = {
            let policies = self
                .inner
                .worker_policies
                .lock()
                .map_err(|_| worker_lock_error("activation grants"))?;
            policies
                .activation_parents
                .keys()
                .chain(policies.activation_parents.values())
                .chain(policies.sticky_activations.iter())
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
        };
        let retired = retained_candidates
            .into_iter()
            .filter(|owner| self.retained_worker_authority(owner).is_err())
            .collect::<std::collections::BTreeSet<_>>();
        let parents = {
            let mut policies = self
                .inner
                .worker_policies
                .lock()
                .map_err(|_| worker_lock_error("activation grants"))?;
            policies
                .activation_parents
                .retain(|owner, parent| !retired.contains(owner) && !retired.contains(parent));
            policies
                .isolation_containers
                .retain(|owner, _| !retired.contains(owner));
            policies
                .sticky_activations
                .retain(|owner| !retired.contains(owner));
            policies.activation_parents.clone()
        };
        let documents = self
            .inner
            .group
            .messages
            .lock()
            .map_err(|_| worker_lock_error("activation documents"))?
            .registered_documents();
        let mut recipients = Vec::new();
        for (candidate_page, target, generation, epoch) in documents {
            if candidate_page != page.0 {
                continue;
            }
            let candidate = AgentOwner::Window {
                page: *page,
                target,
                generation,
                epoch,
            };
            let Ok((partition, origin, _)) = self.native_worker_authority(&candidate) else {
                continue;
            };
            if candidate == owner
                || activation_descendant(&parents, &owner, &candidate)
                || (origin.authority == source_origin.authority
                    && activation_descendant(&parents, &candidate, &owner))
            {
                recipients.push((candidate, partition, origin.authority));
            }
        }
        let now = Instant::now();
        let live = recipients
            .iter()
            .map(|(owner, _, _)| owner.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let mut policies = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| worker_lock_error("activation grants"))?;
        policies.activations.retain(|_, (created, _, _)| {
            now.saturating_duration_since(*created) < Duration::from_secs(5)
        });
        if policies.activations.len()
            + recipients
                .iter()
                .filter(|(owner, _, _)| !policies.activations.contains_key(owner))
                .count()
            > 4096
        {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "activation grant capacity",
            ));
        }
        if policies.sticky_activations.len()
            + recipients
                .iter()
                .filter(|(owner, _, _)| !policies.sticky_activations.contains(owner))
                .count()
            > 4096
        {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "sticky activation capacity",
            ));
        }
        for (recipient, partition, origin) in recipients {
            policies.sticky_activations.insert(recipient.clone());
            policies
                .activations
                .insert(recipient, (now, partition, origin));
        }
        drop(policies);
        // Retirement between inventory capture and grant publication cannot make
        // a stale identity usable; remove it while preserving any successor.
        for recipient in live {
            if self.native_worker_authority(&recipient).is_err() {
                let mut policies = self
                    .inner
                    .worker_policies
                    .lock()
                    .map_err(|_| worker_lock_error("activation grants"))?;
                policies.activations.remove(&recipient);
                policies.sticky_activations.remove(&recipient);
            }
        }
        self.native_worker_authority(&owner)?;
        self.grant_popup_interaction_storage_access(&owner)?;
        Ok(())
    }
    pub(crate) fn sticky_native_activation(&self, owner: &AgentOwner) -> Result<bool, WorkerError> {
        self.native_worker_authority(owner)?;
        Ok(self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| worker_lock_error("activation grants"))?
            .sticky_activations
            .contains(owner))
    }
    pub(crate) fn transient_native_activation(
        &self,
        owner: &AgentOwner,
        consume: bool,
    ) -> Result<bool, WorkerError> {
        if !matches!(owner, AgentOwner::Window { .. }) {
            return Err(WorkerError::new(
                "SecurityError",
                "transient activation requires a Window",
            ));
        }
        self.native_worker_authority(owner)?;
        let active = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| worker_lock_error("activation grants"))?
            .transient_activation(owner, consume, Instant::now());
        self.native_worker_authority(owner)?;
        Ok(active)
    }
    /// A signed retired Document can still be a navigation source, but cannot
    /// borrow a successor's activation. Registry failures remain failures.
    pub(crate) fn navigation_source_activation(
        &self,
        page: PageId,
        creator: &Creator,
    ) -> Result<bool, RuntimeError> {
        use oryn_js::message_wire::WindowTarget;
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Ok(false);
        };
        let pages = self.inner.group.pages.lock().map_err(|_| {
            RuntimeError::ContextService("navigation activation pages poisoned".into())
        })?;
        if !pages.get(&page).is_some_and(|record| record.is_live()) {
            return Ok(false);
        }
        let mut policies = self.inner.worker_policies.lock().map_err(|_| {
            RuntimeError::ContextService("navigation activation policy poisoned".into())
        })?;
        let hub = self.inner.group.messages.lock().map_err(|_| {
            RuntimeError::ContextService("navigation activation documents poisoned".into())
        })?;
        let current = match target {
            WindowTarget::Root => {
                policies
                    .root_lifetimes
                    .get(&page)
                    .is_some_and(|lifetime| lifetime.current(*document_generation))
                    && hub
                        .root_document_authority(page.0, *document_generation, *stream_epoch)
                        .is_ok()
            }
            WindowTarget::Frame(frame) => {
                policies
                    .frame_security_epochs
                    .get(&(page, frame.clone(), *document_generation, *stream_epoch))
                    .copied()
                    == Some(
                        policies
                            .root_lifetimes
                            .get(&page)
                            .map_or(0, |lifetime| lifetime.epoch),
                    )
                    && hub
                        .frame_document_authority(
                            page.0,
                            frame,
                            *document_generation,
                            *stream_epoch,
                        )
                        .is_ok()
            }
        };
        if !current {
            return Ok(false);
        }
        let owner = AgentOwner::Window {
            page,
            target: target.clone(),
            generation: *document_generation,
            epoch: *stream_epoch,
        };
        Ok(policies.transient_activation(&owner, false, Instant::now()))
    }
    #[cfg(all(test, feature = "v8-host"))]
    pub(crate) fn consume_service_interaction(&self, agent: AgentId) -> Result<(), WorkerError> {
        let bootstrap = self.worker_bootstrap(agent)?;
        if bootstrap.kind != oryn_js::worker_wire::AgentKind::Service {
            return Err(WorkerError::new(
                "SecurityError",
                "interaction requires service principal",
            ));
        }
        self.consume_native_service_interaction(
            &crate::service_registry::stable_partition(&bootstrap.partition)?,
            &bootstrap.origin.authority,
        )
    }
    pub(crate) fn consume_native_service_interaction(
        &self,
        stable_partition: &str,
        authority: &str,
    ) -> Result<(), WorkerError> {
        let candidates = {
            let policies = self
                .inner
                .worker_policies
                .lock()
                .map_err(|_| worker_lock_error("activation grants"))?;
            policies
                .activations
                .iter()
                .filter(|(_, (_, partition, origin))| {
                    crate::service_registry::stable_partition(partition)
                        .ok()
                        .as_deref()
                        == Some(stable_partition)
                        && origin.as_str() == authority
                })
                .map(|(owner, (time, _, _))| (owner.clone(), *time))
                .collect::<Vec<_>>()
        };
        let live = candidates
            .into_iter()
            .filter(|(owner, time)| {
                time.elapsed() < Duration::from_secs(5)
                    && self.native_worker_authority(owner).is_ok()
            })
            .collect::<Vec<_>>();
        let mut policies = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| worker_lock_error("activation grants"))?;
        if !live.iter().any(|(owner, time)| {
            policies
                .activations
                .get(owner)
                .is_some_and(|(current, partition, origin)| {
                    current == time
                        && current.elapsed() < Duration::from_secs(5)
                        && crate::service_registry::stable_partition(partition)
                            .ok()
                            .as_deref()
                            == Some(stable_partition)
                        && origin.as_str() == authority
                })
        }) {
            return Err(WorkerError::new(
                "InvalidAccessError",
                "service interaction requires unconsumed native activation",
            ));
        }
        policies.activations.retain(|_, (_, partition, origin)| {
            crate::service_registry::stable_partition(partition)
                .ok()
                .as_deref()
                != Some(stable_partition)
                || origin.as_str() != authority
        });
        Ok(())
    }
    pub(crate) fn service_window_handle(
        &self,
        owner: &AgentOwner,
    ) -> Result<PageHandle, WorkerError> {
        self.native_worker_authority(owner)?;
        let AgentOwner::Window { page, .. } = owner else {
            return Err(WorkerError::new("TypeError", "client is not a Window"));
        };
        self.inner
            .group
            .handle(*page)
            .ok_or_else(|| WorkerError::new("InvalidStateError", "client page retired"))
    }
    pub(crate) fn update_realm_document_url(
        &self,
        owner: &AgentOwner,
        url: &str,
    ) -> Result<(), RuntimeError> {
        if url.len() > 65536 {
            return Err(RuntimeError::ProtocolViolation("realm URL capacity".into()));
        }
        let (_, origin, old_policy) = self
            .native_worker_authority(owner)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        let mut old = url::Url::parse(&old_policy.url)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        let mut next = url::Url::parse(url)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        let tuple = matches!(old.scheme(), "http" | "https")
            && matches!(next.scheme(), "http" | "https")
            && !origin.opaque
            && next.origin().ascii_serialization() == origin.serialized;
        old.set_fragment(None);
        next.set_fragment(None);
        if !tuple && old != next {
            return Err(RuntimeError::ProtocolViolation(
                "same-document URL changed native origin or resource".into(),
            ));
        }
        let AgentOwner::Window {
            page,
            target,
            generation,
            epoch,
        } = owner
        else {
            return Err(RuntimeError::ProtocolViolation(
                "document URL requires Window".into(),
            ));
        };
        let pages = self
            .inner
            .group
            .pages
            .lock()
            .map_err(|_| RuntimeError::ContextService("realm URL pages poisoned".into()))?;
        if !pages.get(page).is_some_and(|record| record.is_live()) {
            return Err(RuntimeError::PageNotFound(*page));
        }
        let mut documents = self
            .inner
            .document_security
            .lock()
            .map_err(|_| RuntimeError::ContextService("realm URL policy poisoned".into()))?;
        let mut policies = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| RuntimeError::ContextService("realm URL registry poisoned".into()))?;
        let hub = self
            .inner
            .group
            .messages
            .lock()
            .map_err(|_| RuntimeError::ContextService("realm URL authority poisoned".into()))?;
        if matches!(target, oryn_js::message_wire::WindowTarget::Root)
            && let Some(scope) = policies.prerenders.get_mut(&(*page, *generation, *epoch))
        {
            if scope.canceling {
                return Err(RuntimeError::NavigationCancelled);
            }
            hub.prerender_document_authority(page.0, &scope.token, scope.lifetime)
                .map_err(RuntimeError::ProtocolViolation)?;
            let policy = scope.policy.as_mut().ok_or_else(|| {
                RuntimeError::ProtocolViolation("prerender policy uncommitted".into())
            })?;
            // The exact native authority check above admitted a same-document
            // URL; preserve its meta origin independently of inherited policy.
            let document_self_url = policy.document_self_url.clone();
            *policy = policy.clone().with_document_url(url.into());
            policy.document_self_url = document_self_url;
            scope.url = url.into();
            return Ok(());
        }
        match target {
            oryn_js::message_wire::WindowTarget::Root => {
                hub.root_document_authority(page.0, *generation, *epoch)
                    .map_err(RuntimeError::ProtocolViolation)?;
                if !policies
                    .root_lifetimes
                    .get(page)
                    .is_some_and(|life| life.current(*generation))
                {
                    return Err(RuntimeError::ProtocolViolation(
                        "realm URL root retired".into(),
                    ));
                }
                documents
                    .get_mut(page)
                    .ok_or(RuntimeError::PageNotFound(*page))?
                    .rebase_inherited_document(url);
            }
            oryn_js::message_wire::WindowTarget::Frame(frame) => {
                let private = hub.prerender_scope_for_document(page.0, target, *generation, *epoch);
                let security_epoch = if let Some((token, lifetime)) = &private {
                    hub.prerender_target_authority(
                        page.0,
                        token,
                        *lifetime,
                        target,
                        *generation,
                        *epoch,
                    )
                    .map_err(RuntimeError::ProtocolViolation)?;
                    lifetime.1
                } else {
                    hub.frame_document_authority(page.0, frame, *generation, *epoch)
                        .map_err(RuntimeError::ProtocolViolation)?;
                    policies
                        .root_lifetimes
                        .get(page)
                        .map_or(0, |life| life.epoch)
                };
                let key = (*page, frame.clone(), *generation, *epoch);
                if policies.frame_security_epochs.get(&key).copied() != Some(security_epoch) {
                    return Err(RuntimeError::ProtocolViolation(
                        "realm URL frame retired".into(),
                    ));
                }
                let policy = policies.frames.get_mut(&key).ok_or_else(|| {
                    RuntimeError::ProtocolViolation("realm URL policy missing".into())
                })?;
                // The exact native authority check above admitted a same-document
                // URL; preserve its meta origin independently of inherited policy.
                let document_self_url = policy.document_self_url.clone();
                *policy = policy.clone().with_document_url(url.into());
                policy.document_self_url = document_self_url;
            }
        }
        Ok(())
    }
    pub(crate) fn install_realm_metadata(
        &self,
        owner: &AgentOwner,
        meta_csp: Option<&str>,
        meta_referrer: Option<&str>,
    ) -> Result<(), RuntimeError> {
        if meta_csp.is_some_and(|value| value.len() > 65536)
            || meta_referrer.is_some_and(|value| value.len() > 65536)
        {
            return Err(RuntimeError::ProtocolViolation(
                "realm metadata capacity".into(),
            ));
        }
        self.native_worker_authority(owner)
            .map_err(|failure| RuntimeError::ContextService(failure.to_string()))?;
        let AgentOwner::Window {
            page,
            target,
            generation,
            epoch,
        } = owner
        else {
            return Err(RuntimeError::ProtocolViolation(
                "document metadata requires Window authority".into(),
            ));
        };
        let pages =
            self.inner.group.pages.lock().map_err(|_| {
                RuntimeError::ContextService("realm metadata pages poisoned".into())
            })?;
        if !pages.get(page).is_some_and(|page| page.is_live()) {
            return Err(RuntimeError::PageNotFound(*page));
        }
        let mut policies =
            self.inner.worker_policies.lock().map_err(|_| {
                RuntimeError::ContextService("realm metadata registry poisoned".into())
            })?;
        let hub = self.inner.group.messages.lock().map_err(|_| {
            RuntimeError::ContextService("realm metadata authority poisoned".into())
        })?;
        if matches!(target, oryn_js::message_wire::WindowTarget::Root)
            && let Some(scope) = policies.prerenders.get_mut(&(*page, *generation, *epoch))
        {
            if scope.canceling {
                return Err(RuntimeError::NavigationCancelled);
            }
            hub.prerender_document_authority(page.0, &scope.token, scope.lifetime)
                .map_err(RuntimeError::ProtocolViolation)?;
            let policy = scope.policy.as_mut().ok_or_else(|| {
                RuntimeError::ProtocolViolation("prerender policy uncommitted".into())
            })?;
            if let Some(value) = meta_csp {
                let mut csp = crate::csp::CspPolicySet::from_agent_policy(policy);
                csp.set_document_self_origin(&scope.origin.serialized);
                if !csp.contains_meta(value) {
                    csp.install_meta(value);
                }
                let additional = policy
                    .headers
                    .iter()
                    .filter(|(name, _)| {
                        !name.eq_ignore_ascii_case("content-security-policy")
                            && !name.eq_ignore_ascii_case("content-security-policy-report-only")
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                *policy = csp.worker_snapshot();
                policy.headers.extend(additional);
            }
            if let Some(value) = meta_referrer.and_then(crate::referrer::ReferrerPolicy::parse_meta)
            {
                policy
                    .headers
                    .retain(|(name, _)| !name.eq_ignore_ascii_case("referrer-policy"));
                policy
                    .headers
                    .push(("referrer-policy".into(), value.as_str().into()));
            }
            return Ok(());
        }
        if matches!(target, oryn_js::message_wire::WindowTarget::Root) {
            hub.root_document_authority(page.0, *generation, *epoch)
                .map_err(RuntimeError::ProtocolViolation)?;
            if !policies
                .root_lifetimes
                .get(page)
                .is_some_and(|lifetime| lifetime.current(*generation))
            {
                return Err(RuntimeError::ProtocolViolation(
                    "root metadata security publication retired".into(),
                ));
            }
            if let Some(value) = meta_csp {
                self.install_meta_security(*page, value)?;
            }
            if let Some(value) = meta_referrer.and_then(crate::referrer::ReferrerPolicy::parse_meta)
            {
                self.inner
                    .document_referrer_policy
                    .lock()
                    .map_err(|_| {
                        RuntimeError::ContextService("realm referrer policy poisoned".into())
                    })?
                    .insert(*page, value);
            }
            return Ok(());
        }
        let oryn_js::message_wire::WindowTarget::Frame(frame) = target else {
            unreachable!()
        };
        let private = hub.prerender_scope_for_document(page.0, target, *generation, *epoch);
        let document_origin = if let Some((token, lifetime)) = &private {
            hub.prerender_target_authority(page.0, token, *lifetime, target, *generation, *epoch)
                .map_err(RuntimeError::ProtocolViolation)?
                .ok_or(RuntimeError::NavigationCancelled)?
        } else {
            hub.frame_document_authority(page.0, frame, *generation, *epoch)
                .map_err(RuntimeError::ProtocolViolation)?
        };
        let security_epoch = private.as_ref().map_or_else(
            || {
                policies
                    .root_lifetimes
                    .get(page)
                    .map_or(0, |lifetime| lifetime.epoch)
            },
            |(_, lifetime)| lifetime.1,
        );
        let key = (*page, frame.clone(), *generation, *epoch);
        if policies.frame_security_epochs.get(&key).copied() != Some(security_epoch) {
            return Err(RuntimeError::ProtocolViolation(
                "retired realm metadata".into(),
            ));
        }
        let policy = policies.frames.get_mut(&key).ok_or_else(|| {
            RuntimeError::ProtocolViolation("frame metadata has no committed policy".into())
        })?;
        if let Some(value) = meta_csp {
            let mut csp = crate::csp::CspPolicySet::from_agent_policy(policy);
            csp.set_document_self_origin(&document_origin.0);
            if !csp.contains_meta(value) {
                csp.install_meta(value);
            }
            let additional = policy
                .headers
                .iter()
                .filter(|(name, _)| {
                    !name.eq_ignore_ascii_case("content-security-policy")
                        && !name.eq_ignore_ascii_case("content-security-policy-report-only")
                })
                .cloned()
                .collect::<Vec<_>>();
            *policy = csp.worker_snapshot();
            policy.headers.extend(additional);
        }
        if let Some(value) = meta_referrer.and_then(crate::referrer::ReferrerPolicy::parse_meta) {
            policy
                .headers
                .retain(|(name, _)| !name.eq_ignore_ascii_case("referrer-policy"));
            policy
                .headers
                .push(("referrer-policy".into(), value.as_str().into()));
        }
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn prepare_frame_worker_policy(
        &self,
        page: PageId,
        url: &str,
        headers: &[(String, String)],
        html: &str,
    ) -> Result<String, RuntimeError> {
        self.prepare_frame_worker_policy_for_creator(page, url, headers, html, None)
    }
    pub(crate) fn prepare_frame_worker_policy_for_creator(
        &self,
        page: PageId,
        url: &str,
        headers: &[(String, String)],
        _html: &str,
        creator: Option<&Creator>,
    ) -> Result<String, RuntimeError> {
        if matches!(creator, Some(Creator::Agent(_))) {
            return Err(RuntimeError::ProtocolViolation(
                "frame response policy requires a document creator".into(),
            ));
        }
        if let Some(Creator::Window {
            target,
            document_generation,
            stream_epoch,
        }) = creator
        {
            self.window_worker_creator(page, target, *document_generation, *stream_epoch)?;
        }
        let policy = crate::csp::CspPolicySet::from_headers(url, headers);
        let mut snapshot = policy.worker_snapshot();
        for (name, value) in headers.iter().filter(|(name, _)| {
            [
                "referrer-policy",
                "cross-origin-opener-policy",
                "cross-origin-embedder-policy",
                "permissions-policy",
                "origin-agent-cluster",
            ]
            .iter()
            .any(|field| name.eq_ignore_ascii_case(field))
        }) {
            snapshot.headers.push((name.clone(), value.clone()));
        }
        let pages = self
            .inner
            .group
            .pages
            .lock()
            .map_err(|_| RuntimeError::ContextService("page registry lock poisoned".into()))?;
        if !pages.get(&page).is_some_and(|record| record.is_live()) {
            return Err(RuntimeError::PageNotFound(page));
        }
        let mut policies =
            self.inner.worker_policies.lock().map_err(|_| {
                RuntimeError::ContextService("worker policy registry poisoned".into())
            })?;
        if policies.prepared.len() >= 4096 {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "frame worker policy tickets".into(),
            });
        }
        let mut secret = [0u8; 32];
        getrandom::fill(&mut secret)
            .map_err(|_| RuntimeError::ContextService("frame policy entropy unavailable".into()))?;
        let token = secret
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if policies.prepared.contains_key(&token) {
            return Err(RuntimeError::ContextService(
                "frame policy ticket collision".into(),
            ));
        }
        let epoch = if let Some(Creator::Window {
            target,
            document_generation,
            stream_epoch,
        }) = creator
        {
            let hub = self.inner.group.messages.lock().map_err(|_| {
                RuntimeError::ContextService("prepared frame owner documents poisoned".into())
            })?;
            if let Some((candidate, lifetime)) = hub.prerender_scope_for_document(
                page.0,
                target,
                *document_generation,
                *stream_epoch,
            ) {
                let scope = policies
                    .prerenders
                    .get(&(page, lifetime.0, lifetime.1))
                    .filter(|scope| scope.token == candidate && !scope.canceling)
                    .ok_or(RuntimeError::NavigationCancelled)?;
                hub.prerender_target_authority(
                    page.0,
                    &candidate,
                    scope.lifetime,
                    target,
                    *document_generation,
                    *stream_epoch,
                )
                .map_err(RuntimeError::ProtocolViolation)?;
                lifetime.1
            } else {
                let security_epoch = policies
                    .root_lifetimes
                    .get(&page)
                    .map_or(0, |lifetime| lifetime.epoch);
                match target {
                    oryn_js::message_wire::WindowTarget::Root => {
                        hub.root_document_authority(page.0, *document_generation, *stream_epoch)
                            .map_err(RuntimeError::ProtocolViolation)?;
                        if !policies
                            .root_lifetimes
                            .get(&page)
                            .is_some_and(|life| life.current(*document_generation))
                        {
                            return Err(RuntimeError::NavigationCancelled);
                        }
                    }
                    oryn_js::message_wire::WindowTarget::Frame(frame) => {
                        hub.frame_document_authority(
                            page.0,
                            frame,
                            *document_generation,
                            *stream_epoch,
                        )
                        .map_err(RuntimeError::ProtocolViolation)?;
                        if policies
                            .frame_security_epochs
                            .get(&(page, frame.clone(), *document_generation, *stream_epoch))
                            .copied()
                            != Some(security_epoch)
                        {
                            return Err(RuntimeError::NavigationCancelled);
                        }
                    }
                }
                security_epoch
            }
        } else {
            policies
                .root_lifetimes
                .get(&page)
                .map_or(0, |lifetime| lifetime.epoch)
        };
        policies.prepared_isolation.insert(
            token.clone(),
            crate::agent_isolation::DocumentIsolationPolicy::from_headers(headers),
        );
        policies
            .prepared_security_epochs
            .insert(token.clone(), epoch);
        policies
            .prepared
            .insert(token.clone(), (page, url.into(), snapshot));
        Ok(token)
    }
    #[cfg(test)]
    pub(crate) fn prepare_local_frame_worker_policy(
        &self,
        page: PageId,
        url: &str,
        source_policy: &oryn_js::worker_wire::AgentPolicy,
        html: &str,
        origin: oryn_js::worker_wire::AgentOrigin,
    ) -> Result<String, RuntimeError> {
        self.prepare_local_frame_worker_policy_for_creator(
            page,
            url,
            source_policy,
            html,
            origin,
            None,
        )
    }
    pub(crate) fn prepare_local_frame_worker_policy_for_creator(
        &self,
        page: PageId,
        url: &str,
        source_policy: &oryn_js::worker_wire::AgentPolicy,
        _html: &str,
        origin: oryn_js::worker_wire::AgentOrigin,
        creator: Option<&Creator>,
    ) -> Result<String, RuntimeError> {
        // CSP self remains the policy container creator's origin for local URLs.
        let mut csp = crate::csp::CspPolicySet::from_agent_policy(source_policy);
        csp.rebase_document(url);
        csp.set_document_self_origin(&origin.serialized);
        let mut snapshot = csp.worker_snapshot();
        snapshot.headers.extend(
            source_policy
                .headers
                .iter()
                .filter(|(name, _)| {
                    !name.eq_ignore_ascii_case("content-security-policy")
                        && !name.eq_ignore_ascii_case("content-security-policy-report-only")
                })
                .cloned(),
        );
        let token = self.prepare_frame_worker_policy_for_creator(
            page,
            url,
            &source_policy.headers,
            "",
            creator,
        )?;
        let mut policies = self.inner.worker_policies.lock().map_err(|_| {
            RuntimeError::ContextService("local frame policy registry poisoned".into())
        })?;
        if let Some((_, response_url, policy)) = policies.prepared.get_mut(&token) {
            *response_url = url.into();
            *policy = snapshot;
        }
        policies.prepared_origins.insert(token.clone(), origin);
        Ok(token)
    }
    pub(crate) fn navigation_resource_grants_drained(
        &self,
        page: PageId,
    ) -> Result<bool, RuntimeError> {
        let policies = self.inner.worker_policies.lock().map_err(|_| {
            RuntimeError::ContextService("navigation cleanup registry poisoned".into())
        })?;
        #[cfg(unix)]
        if policies
            .blank_activations
            .values()
            .any(|record| record.source == page && !record.acknowledged)
        {
            return Ok(false);
        }
        Ok(!policies.prerenders.iter().any(|((owner,_,_),scope)|*owner==page||scope.blank_source.as_ref().is_some_and(|(source,_)|*source==page))
            && !policies.parser_encoding.values().any(|entry|matches!(&entry.owner,AgentOwner::Window{page:owner,..}if *owner==page))
            && !policies.prepared.values().any(|(owner,_,_)|*owner==page)
            && !policies.local_documents.values().any(|entry|matches!(&entry.owner,crate::agent_registry::AgentOwner::Window{page:owner,..} if *owner==page)))
    }

    /// Only the reaped-process cleanup path may reclaim grants whose child can
    /// no longer send release messages. Broker admission and in-flight requests
    /// must already be drained. Do not clear unrelated page policy or live peers.
    pub(crate) fn retire_reaped_navigation_resources(
        &self,
        page: PageId,
        reaped_pages: &[PageId],
    ) -> Result<(), RuntimeError> {
        #[cfg(unix)]
        {
            let pending = self
                .inner
                .worker_policies
                .lock()
                .map_err(|_| {
                    RuntimeError::ContextService("navigation cleanup registry poisoned".into())
                })?
                .prerenders
                .iter()
                .filter(|((owner, _, _), scope)| {
                    *owner == page
                        || scope
                            .blank_source
                            .as_ref()
                            .is_some_and(|(source, _)| *source == page)
                })
                .map(|((owner, _, _), scope)| (*owner, scope.token.clone(), scope.lifetime))
                .collect::<Vec<_>>();
            for (owner, token, lifetime) in pending {
                // Use the normal, retryable cancellation to settle agents,
                // sockets and transactions before discarding the scope.
                if owner == page {
                    self.cancel_native_prerender(owner, &token, lifetime)?;
                } else {
                    self.cancel_blank_prerender(page, owner, &token, lifetime.0, lifetime.1)?;
                }
            }
        }
        let mut policies = self.inner.worker_policies.lock().map_err(|_| {
            RuntimeError::ContextService("navigation cleanup registry poisoned".into())
        })?;
        #[cfg(unix)]
        for ((owner, _, _), record) in policies.blank_activations.iter_mut() {
            if reaped_pages.contains(owner) {
                record.destroy();
            } else if record.source == page {
                record.retire_reaped_source()?;
            }
        }
        policies.local_documents.retain(
            |_, entry| !matches!(&entry.owner, AgentOwner::Window { page: owner, .. } if *owner == page));
        policies.parser_encoding.retain(
            |_, entry| !matches!(&entry.owner, AgentOwner::Window { page: owner, .. } if *owner == page));
        let tokens = policies
            .prepared
            .iter()
            .filter(|(_, (owner, _, _))| *owner == page)
            .map(|(token, _)| token.clone())
            .collect::<Vec<_>>();
        for token in tokens {
            policies.prepared.remove(&token);
            policies.prepared_origins.remove(&token);
            policies.prepared_isolation.remove(&token);
            policies.prepared_security_epochs.remove(&token);
        }
        Ok(())
    }
    pub(crate) fn discard_prepared_frame_policy(
        &self,
        page: PageId,
        token: &str,
    ) -> Result<(), RuntimeError> {
        let mut policies = self.inner.worker_policies.lock().map_err(|_| {
            RuntimeError::ContextService("frame policy cleanup registry poisoned".into())
        })?;
        if policies
            .prepared
            .get(token)
            .is_some_and(|(owner, _, _)| *owner != page)
        {
            return Err(RuntimeError::ProtocolViolation(
                "foreign frame policy cleanup".into(),
            ));
        }
        policies.prepared.remove(token);
        policies.prepared_origins.remove(token);
        policies.prepared_isolation.remove(token);
        policies.prepared_security_epochs.remove(token);
        Ok(())
    }
    pub(crate) fn prepared_frame_resource_policy(
        &self,
        page: PageId,
        token: &str,
    ) -> Result<oryn_js::worker_wire::AgentPolicy, RuntimeError> {
        let policies = self.inner.worker_policies.lock().map_err(|_| {
            RuntimeError::ContextService("prepared resource policy registry poisoned".into())
        })?;
        let (owner, _, policy) = policies.prepared.get(token).ok_or_else(|| {
            RuntimeError::ProtocolViolation("unknown prepared resource policy".into())
        })?;
        if *owner != page {
            return Err(RuntimeError::ProtocolViolation(
                "foreign prepared resource policy".into(),
            ));
        }
        let epoch = policies
            .prepared_security_epochs
            .get(token)
            .copied()
            .ok_or(RuntimeError::NavigationCancelled)?;
        if epoch
            != policies
                .root_lifetimes
                .get(&page)
                .map_or(0, |lifetime| lifetime.epoch)
        {
            let hub = self.inner.group.messages.lock().map_err(|_| {
                RuntimeError::ContextService("prepared private frame documents poisoned".into())
            })?;
            if !policies.prerenders.iter().any(|((owner, _, _), scope)| {
                *owner == page
                    && scope.lifetime.1 == epoch
                    && !scope.canceling
                    && hub
                        .prerender_document_authority(page.0, &scope.token, scope.lifetime)
                        .is_ok()
            }) {
                return Err(RuntimeError::ProtocolViolation(
                    "prepared resource policy belongs to a retired document".into(),
                ));
            }
        }
        Ok(policy.clone())
    }
    #[cfg(test)]
    pub(crate) fn bind_frame_worker_policy(
        &self,
        page: PageId,
        frame: String,
        generation: u64,
        epoch: u64,
        url: &str,
        token: &str,
    ) -> Result<(), RuntimeError> {
        self.bind_frame_worker_policy_with_sandbox(FramePolicyInputs {
            page,
            frame,
            generation,
            epoch,
            url,
            token,
            opaque_sandbox: false,
        })
    }
    pub(crate) fn bind_frame_worker_policy_with_sandbox(
        &self,
        params: FramePolicyInputs<'_>,
    ) -> Result<(), RuntimeError> {
        let FramePolicyInputs {
            page,
            frame,
            generation,
            epoch,
            url,
            token,
            opaque_sandbox,
        } = params;

        self.page_storage_partition(page)?;
        self.prepared_frame_resource_policy(page, token)?;
        let mut policies =
            self.inner.worker_policies.lock().map_err(|_| {
                RuntimeError::ContextService("worker policy registry poisoned".into())
            })?;
        let mut hub = self.inner.group.messages.lock().map_err(|_| {
            RuntimeError::ContextService("frame authority registry poisoned".into())
        })?;
        let private = hub.prerender_scope_for_document(
            page.0,
            &oryn_js::message_wire::WindowTarget::Frame(frame.clone()),
            generation,
            epoch,
        );
        let claimed = if let Some((candidate, lifetime)) = &private {
            hub.prerender_target_authority(
                page.0,
                candidate,
                *lifetime,
                &oryn_js::message_wire::WindowTarget::Frame(frame.clone()),
                generation,
                epoch,
            )
            .map_err(RuntimeError::ContextService)?
            .ok_or(RuntimeError::NavigationCancelled)?
        } else {
            hub.frame_document_authority(page.0, &frame, generation, epoch)
                .map_err(RuntimeError::ContextService)?
        };
        let security_epoch = private.as_ref().map_or_else(
            || {
                policies
                    .root_lifetimes
                    .get(&page)
                    .map_or(0, |life| life.epoch)
            },
            |(_, lifetime)| lifetime.1,
        );
        if policies.prepared_security_epochs.get(token).copied() != Some(security_epoch) {
            return Err(RuntimeError::ProtocolViolation(
                "frame response policy publication retired".into(),
            ));
        }
        let (owner, response_url, policy) = policies
            .prepared
            .get(token)
            .ok_or_else(|| RuntimeError::ContextService("unknown frame policy ticket".into()))?;
        if *owner != page || response_url != url {
            return Err(RuntimeError::ProtocolViolation(
                "frame policy ticket owner or response URL mismatch".into(),
            ));
        }
        let policy = policy.clone();
        let key = (page, frame.clone(), generation, epoch);
        let native_origin = SecurityOrigin::from_url(&policy.url);
        let expected = native_origin.serialize();
        if !policies.prepared_origins.contains_key(token)
            && claimed.0 != "null"
            && claimed.0 != expected
        {
            return Err(RuntimeError::ProtocolViolation(
                "frame origin differs from native response origin".into(),
            ));
        }
        let claimed = if let Some(origin) = policies.prepared_origins.get(token) {
            let sandboxed = opaque_sandbox
                || crate::csp::CspPolicySet::from_agent_policy(&policy)
                    .sandbox()
                    .is_some_and(|sandbox| !sandbox.allow_same_origin);
            if sandboxed {
                if claimed.0 != "null" {
                    return Err(RuntimeError::ProtocolViolation(
                        "sandboxed local frame requires opaque origin".into(),
                    ));
                }
                (
                    "null".into(),
                    format!("opaque-frame:{frame}:{generation}:{epoch}"),
                )
            } else {
                if claimed.0 != origin.serialized {
                    return Err(RuntimeError::ProtocolViolation(
                        "local frame differs from granted origin".into(),
                    ));
                }
                (origin.serialized.clone(), origin.authority.clone())
            }
        } else if claimed.0 == "null" {
            (
                "null".into(),
                format!("opaque-frame:{frame}:{generation}:{epoch}"),
            )
        } else {
            (expected, native_origin.security_token_key())
        };
        if policies
            .frame_origins
            .get(&key)
            .is_some_and(|origin| origin != &claimed)
        {
            return Err(RuntimeError::ProtocolViolation(
                "committed frame origin changed".into(),
            ));
        }
        if let Some(existing) = policies.frames.get(&key) {
            if existing.url != policy.url || !existing.headers.starts_with(&policy.headers) {
                return Err(RuntimeError::ProtocolViolation(
                    "committed frame worker policy cannot be replaced".into(),
                ));
            }
            return Ok(());
        }
        if policies.frames.len() >= 4096 {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "frame worker policies".into(),
            });
        }
        let isolation = policies
            .prepared_isolation
            .get(token)
            .copied()
            .ok_or_else(|| {
                RuntimeError::ContextService("missing native frame isolation policy".into())
            })?;
        if let Some((candidate, lifetime)) = &private {
            hub.commit_prerender_frame_origin(oryn_js::message_wire::PrerenderFrameOrigin {
                page: page.0,
                token: candidate,
                lifetime: *lifetime,
                frame: &frame,
                generation,
                epoch,
                origin: claimed.clone(),
            })
            .map_err(RuntimeError::ContextService)?;
        } else {
            hub.commit_frame_origin(page.0, &frame, generation, epoch, claimed.clone())
                .map_err(RuntimeError::ContextService)?;
        }
        policies
            .frame_security_epochs
            .insert(key.clone(), security_epoch);
        policies.frame_origins.insert(key.clone(), claimed);
        policies.frame_isolation.insert(key.clone(), isolation);
        policies.frames.insert(key, policy);
        drop(policies);
        drop(hub);
        #[cfg(unix)]
        if let Some(services) = self.existing_service_registry() {
            services
                .control_client(
                    AgentOwner::Window {
                        page,
                        target: oryn_js::message_wire::WindowTarget::Frame(frame),
                        generation,
                        epoch,
                    },
                    None,
                )
                .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        }
        Ok(())
    }
    pub(crate) fn window_ancestor_urls(
        &self,
        owner: &AgentOwner,
    ) -> Result<Vec<String>, RuntimeError> {
        let parents = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| RuntimeError::ContextService("form ancestry registry poisoned".into()))?
            .activation_parents
            .clone();
        let mut cursor = owner;
        let mut seen = std::collections::BTreeSet::new();
        let mut urls = Vec::new();
        while let Some(parent) = parents.get(cursor) {
            if !seen.insert(parent.clone()) {
                return Err(RuntimeError::ProtocolViolation(
                    "form ancestry cycle".into(),
                ));
            }
            let AgentOwner::Window {
                page,
                target,
                generation,
                epoch,
            } = parent
            else {
                return Err(RuntimeError::ProtocolViolation(
                    "form ancestor is not a Window".into(),
                ));
            };
            let (_, _, origin) = self.window_worker_creator(*page, target, *generation, *epoch)?;
            let policy = self.window_worker_policy(*page, target, *generation, *epoch)?;
            urls.push(if origin.serialized == "null" {
                "about:blank".into()
            } else {
                policy.url
            });
            cursor = parent;
        }
        Ok(urls)
    }

    pub(crate) fn window_worker_policy(
        &self,
        page: PageId,
        target: &oryn_js::message_wire::WindowTarget,
        generation: u64,
        epoch: u64,
    ) -> Result<oryn_js::worker_wire::AgentPolicy, RuntimeError> {
        self.window_worker_creator(page, target, generation, epoch)?;
        if matches!(target, oryn_js::message_wire::WindowTarget::Root) {
            let candidate = self
                .inner
                .worker_policies
                .lock()
                .map_err(|_| RuntimeError::ContextService("prerender policies poisoned".into()))?
                .prerenders
                .get(&(page, generation, epoch))
                .and_then(|scope| scope.policy.clone());
            if let Some(policy) = candidate {
                self.window_worker_creator(page, target, generation, epoch)?;
                return Ok(policy);
            }
        }
        let snapshot = match target {
            oryn_js::message_wire::WindowTarget::Root => {
                let mut snapshot = self
                    .inner
                    .document_security
                    .lock()
                    .map_err(|_| {
                        RuntimeError::ContextService("document policy registry poisoned".into())
                    })?
                    .get(&page)
                    .map(crate::csp::CspPolicySet::worker_snapshot)
                    .ok_or_else(|| {
                        RuntimeError::ContextService("root worker policy unavailable".into())
                    })?;
                self.page_isolation_policy(page)?
                    .append_native_headers(&mut snapshot.headers);
                // Exact native owner validation above/below permits the hidden
                // initial predecessor used by blank-prerender activation.
                let referrer = self
                    .inner
                    .document_referrer_policy
                    .lock()
                    .map_err(|_| {
                        RuntimeError::ContextService("referrer policy lock poisoned".into())
                    })?
                    .get(&page)
                    .copied()
                    .unwrap_or_default();
                snapshot
                    .headers
                    .push(("referrer-policy".into(), referrer.as_str().into()));
                let policies = self.inner.worker_policies.lock().map_err(|_| {
                    RuntimeError::ContextService("permissions policy registry poisoned".into())
                })?;
                // Preserve the declaration rather than reducing explicit origin
                // lists to a boolean that would accidentally authorize siblings.
                let retained = policies
                    .root_isolation_permissions
                    .get(&page)
                    .cloned()
                    .unwrap_or_default();
                snapshot.headers.retain(|(name, _)| {
                    !name.eq_ignore_ascii_case("permissions-policy")
                        && !retained
                            .iter()
                            .any(|(field, _)| name.eq_ignore_ascii_case(field))
                });
                snapshot.headers.extend(retained);
                Ok(snapshot)
            }
            oryn_js::message_wire::WindowTarget::Frame(frame) => self
                .inner
                .worker_policies
                .lock()
                .map_err(|_| {
                    RuntimeError::ContextService("worker policy registry poisoned".into())
                })?
                .frames
                .get(&(page, frame.clone(), generation, epoch))
                .cloned()
                .ok_or_else(|| {
                    RuntimeError::ContextService("frame worker policy not bound".into())
                }),
        }?;
        self.window_worker_creator(page, target, generation, epoch)?;
        Ok(snapshot)
    }
}

impl BrowserContext {
    pub(crate) fn clear_page_worker_policies(&self, page: PageId) {
        let mut policies = self
            .inner
            .worker_policies
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        policies
            .prerenders
            .retain(|(owner, _, _), _| *owner != page);
        policies
            .prerender_completions
            .retain(|(owner, _, _), _| *owner != page);
        #[cfg(unix)]
        for ((owner, _, _), record) in policies.blank_activations.iter_mut() {
            if *owner == page {
                record.destroy();
            }
        }
        policies.blobs.retain(
            |_, blob| !matches!(&blob.owner,AgentOwner::Window{page:owner,..} if *owner==page),
        );
        policies.local_documents.retain(
            |_, grant| !matches!(&grant.owner,AgentOwner::Window{page:owner,..}if *owner==page),
        );
        let removed = policies
            .prepared
            .iter()
            .filter(|(_, (owner, _, _))| *owner == page)
            .map(|(token, _)| token.clone())
            .collect::<Vec<_>>();
        for token in removed {
            policies.prepared_isolation.remove(&token);
            policies.prepared_security_epochs.remove(&token);
            policies.prepared_origins.remove(&token);
        }
        policies
            .isolation_containers
            .retain(|owner, _| !matches!(owner,AgentOwner::Window{page:owned,..}if *owned==page));
        policies.root_isolation.remove(&page);
        policies.root_isolation_permissions.remove(&page);
        policies
            .frame_isolation
            .retain(|(owner, _, _, _), _| *owner != page);
        policies.prepared.retain(|_, (owner, _, _)| *owner != page);
        policies.parser_encoding.retain(
            |_, entry| !matches!(&entry.owner,AgentOwner::Window{page:owner,..}if *owner==page),
        );
        policies.frames.retain(|(owner, _, _, _), _| *owner != page);
        policies
            .frame_openers
            .retain(|(owner, _), (source, _)| *owner != page && *source != page);
        policies
            .document_domains
            .documents
            .retain(|owner, _| !matches!(owner,AgentOwner::Window{page:owned,..}if *owned==page));
        policies.document_domains.initial.remove(&page);
        policies
            .frame_origins
            .retain(|(owner, _, _, _), _| *owner != page);
        policies
            .frame_security_epochs
            .retain(|(owner, _, _, _), _| *owner != page);
        policies.root_lifetimes.remove(&page);
        policies.window_inventories.remove(&page);
        policies
            .activations
            .retain(|owner, _| !matches!(owner,AgentOwner::Window{page:owned,..} if *owned==page));
        policies
            .activation_parents
            .retain(|owner, _| !matches!(owner,AgentOwner::Window{page:owned,..} if *owned==page));
        policies
            .sticky_activations
            .retain(|owner| !matches!(owner,AgentOwner::Window{page:owned,..} if *owned==page));
        policies
            .secure_contexts
            .retain(|owner, _| !matches!(owner,AgentOwner::Window{page:owned,..} if *owned==page));
        drop(policies);
        self.inner
            .group
            .shared_memory
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retire_page(page);
    }
}

#[cfg(all(test, unix))]
mod policy_tests {
    use super::*;
    use oryn_js::message_wire::{MessageRequest, WindowTarget};
    #[test]
    fn frame_creator_requires_native_response_commit_and_rejects_forged_origin() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://top.test/".into())
            .unwrap();
        let frame = "e".repeat(64);
        let target = WindowTarget::Frame(frame.clone());
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                    origin: "https://forged.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        assert!(
            context
                .window_worker_creator(page.page_id(), &target, 2, 0)
                .is_err()
        );
        assert!(
            context
                .handle_message_wire(
                    page.page_id(),
                    MessageRequest::BroadcastOpen {
                        target: target.clone(),
                        name: vec![]
                    }
                )
                .is_err()
        );
        let token = context
            .prepare_frame_worker_policy(page.page_id(), "https://actual.test/", &[], "")
            .unwrap();
        assert!(
            context
                .bind_frame_worker_policy(
                    page.page_id(),
                    frame.clone(),
                    2,
                    0,
                    "https://actual.test/",
                    &token
                )
                .is_err()
        );
        assert!(
            context
                .window_worker_creator(page.page_id(), &target, 2, 0)
                .is_err()
        );
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 3,
                    stream_epoch: 0,
                    origin: "https://actual.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        context
            .bind_frame_worker_policy(
                page.page_id(),
                frame.clone(),
                3,
                0,
                "https://actual.test/",
                &token,
            )
            .unwrap();
        assert_eq!(
            context
                .window_worker_creator(page.page_id(), &target, 3, 0)
                .unwrap()
                .2
                .serialized,
            "https://actual.test"
        );
        assert!(
            context
                .handle_message_wire(
                    page.page_id(),
                    MessageRequest::BroadcastOpen {
                        target: target.clone(),
                        name: vec![]
                    }
                )
                .is_ok()
        );
        assert!(
            context
                .handle_message_wire(
                    page.page_id(),
                    MessageRequest::RegisterWindow {
                        frame,
                        document_generation: 3,
                        stream_epoch: 0,
                        origin: "https://forged.test".into(),
                        inherit_origin: None
                    }
                )
                .is_err()
        );
        assert!(
            context
                .window_worker_creator(page.page_id(), &target, 2, 0)
                .is_err()
        );
    }
    #[test]
    fn opaque_frame_response_commit_does_not_inherit_forged_top_origin_key() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://top.test/".into())
            .unwrap();
        let frame = "f".repeat(64);
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 4,
                    stream_epoch: 7,
                    origin: "null".into(),
                    inherit_origin: Some(WindowTarget::Root),
                },
            )
            .unwrap();
        let token = context
            .prepare_frame_worker_policy(page.page_id(), "https://child.test/", &[], "")
            .unwrap();
        context
            .bind_frame_worker_policy(
                page.page_id(),
                frame.clone(),
                4,
                7,
                "https://child.test/",
                &token,
            )
            .unwrap();
        let (_, _, origin) = context
            .window_worker_creator(page.page_id(), &WindowTarget::Frame(frame.clone()), 4, 7)
            .unwrap();
        assert!(origin.opaque);
        assert_eq!(origin.authority, format!("opaque-frame:{frame}:4:7"));
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 4,
                    stream_epoch: 7,
                    origin: "null".into(),
                    inherit_origin: Some(WindowTarget::Root),
                },
            )
            .unwrap();
        assert_eq!(
            context
                .window_worker_creator(page.page_id(), &WindowTarget::Frame(frame), 4, 7)
                .unwrap()
                .2
                .authority,
            origin.authority
        );
    }
    #[test]
    fn native_frame_policy_ticket_cannot_be_rebound_to_foreign_page_or_url() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        let other = context.new_page();
        let frame = "a".repeat(64);
        let url = "https://child.test/frame";
        for page in [page.page_id(), other.page_id()] {
            context
                .commit_page_url(page, "https://top.test/".into())
                .unwrap();
            context
                .handle_message_wire(
                    page,
                    MessageRequest::RegisterWindow {
                        frame: frame.clone(),
                        document_generation: 2,
                        stream_epoch: 0,
                        origin: "https://child.test".into(),
                        inherit_origin: None,
                    },
                )
                .unwrap();
        }
        let token=context.prepare_frame_worker_policy(page.page_id(),url,
            &[("content-security-policy".into(),"worker-src 'none'".into())],
            "<meta http-equiv='content-security-policy' content=\"script-src 'self'; sandbox\">").unwrap();
        assert!(
            context
                .bind_frame_worker_policy(other.page_id(), frame.clone(), 2, 0, url, &token)
                .is_err()
        );
        assert!(
            context
                .bind_frame_worker_policy(
                    page.page_id(),
                    frame.clone(),
                    2,
                    0,
                    "https://child.test/other",
                    &token
                )
                .is_err()
        );
        context
            .bind_frame_worker_policy(page.page_id(), frame.clone(), 2, 0, url, &token)
            .unwrap();
        let before_meta = context
            .window_worker_policy(page.page_id(), &WindowTarget::Frame(frame.clone()), 2, 0)
            .unwrap();
        assert!(
            crate::csp::CspPolicySet::from_agent_policy(&before_meta)
                .allows_eval(false)
                .allowed,
            "future parser meta must not become response-header policy"
        );
        let owner = context
            .window_worker_creator(page.page_id(), &WindowTarget::Frame(frame.clone()), 2, 0)
            .unwrap()
            .0;
        context
            .install_realm_metadata(&owner, Some("script-src 'self'; sandbox"), None)
            .unwrap();
        let snapshot = context
            .window_worker_policy(page.page_id(), &WindowTarget::Frame(frame.clone()), 2, 0)
            .unwrap();
        let policy = crate::csp::CspPolicySet::from_agent_policy(&snapshot);
        assert!(!policy.allows_eval(false).allowed);
        assert!(
            crate::csp::CspPolicySet::from_agent_policy(&before_meta)
                .allows_eval(false)
                .allowed,
            "previously captured immutable policy retains its parser-time meaning"
        );
        assert!(
            !policy
                .authorize(&crate::csp::CspRequest {
                    destination: crate::csp::CspDestination::Worker,
                    url: "https://child.test/worker.js".into(),
                    nonce: None,
                    inline_source: None,
                    parser_inserted: false,
                    initiated_by_trusted_script: false,
                })
                .allowed,
            "response-header worker-src remains in force after parser metadata"
        );
        assert!(
            policy.sandbox().is_none(),
            "ignored meta sandbox cannot reactivate when snapshot is loaded"
        );
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RetireWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .install_realm_metadata(&owner, Some("script-src *"), None)
                .is_err(),
            "retired document cannot publish parser metadata"
        );
        assert!(
            context
                .window_worker_policy(page.page_id(), &WindowTarget::Frame(frame), 2, 0)
                .is_err()
        );
        context.clear_page_worker_policies(page.page_id());
        assert!(
            !context
                .inner
                .worker_policies
                .lock()
                .unwrap()
                .prepared
                .contains_key(&token)
        );
    }
}

impl BrowserContext {
    pub(crate) fn inherit_frame_worker_policy_at_url(
        &self,
        params: InheritedFramePolicyInputs<'_>,
    ) -> Result<(), RuntimeError> {
        let InheritedFramePolicyInputs {
            page,
            frame,
            generation,
            epoch,
            parent,
            parent_generation,
            parent_epoch,
            document_url,
        } = params;

        if !matches!(
            document_url,
            "about:blank" | "about:srcdoc" | "about:neterror"
        ) {
            return Err(RuntimeError::ProtocolViolation(
                "inherited frame URL is not a native local document".into(),
            ));
        }
        let policy = self
            .window_worker_policy(page, parent, parent_generation, parent_epoch)?
            .with_document_url(document_url.into());
        let (_, _, parent_origin) =
            self.window_worker_creator(page, parent, parent_generation, parent_epoch)?;
        let mut policies =
            self.inner.worker_policies.lock().map_err(|_| {
                RuntimeError::ContextService("worker policy registry poisoned".into())
            })?;
        let mut hub = self.inner.group.messages.lock().map_err(|_| {
            RuntimeError::ContextService("frame authority registry poisoned".into())
        })?;
        let private = hub.prerender_scope_for_document(
            page.0,
            &oryn_js::message_wire::WindowTarget::Frame(frame.clone()),
            generation,
            epoch,
        );
        let claimed = if let Some((token, lifetime)) = &private {
            hub.prerender_target_authority(
                page.0,
                token,
                *lifetime,
                &oryn_js::message_wire::WindowTarget::Frame(frame.clone()),
                generation,
                epoch,
            )
            .map_err(RuntimeError::ContextService)?
            .ok_or(RuntimeError::NavigationCancelled)?
        } else {
            hub.frame_document_authority(page.0, &frame, generation, epoch)
                .map_err(RuntimeError::ContextService)?
        };
        if private
            != hub.prerender_scope_for_document(page.0, parent, parent_generation, parent_epoch)
        {
            return Err(RuntimeError::ProtocolViolation(
                "frame registration and parent belong to different native trees".into(),
            ));
        }

        if claimed.0 != "null"
            && (claimed.0 != parent_origin.serialized || claimed.1 != parent_origin.authority)
        {
            return Err(RuntimeError::ProtocolViolation(
                "inherited frame origin differs from native parent".into(),
            ));
        }
        let key = (page, frame.clone(), generation, epoch);
        let claimed = if claimed.0 == "null"
            && (!parent_origin.opaque || claimed.1 != parent_origin.authority)
        {
            (
                "null".into(),
                format!("opaque-frame:{frame}:{generation}:{epoch}"),
            )
        } else {
            (parent_origin.serialized, parent_origin.authority)
        };
        let newly_bound = !policies.frames.contains_key(&key);
        if policies
            .frame_origins
            .get(&key)
            .is_some_and(|origin| origin != &claimed)
        {
            return Err(RuntimeError::ProtocolViolation(
                "committed inherited frame origin changed".into(),
            ));
        }
        if let Some(existing) = policies.frames.get(&key) {
            if existing.url != policy.url || !existing.headers.starts_with(&policy.headers) {
                return Err(RuntimeError::ProtocolViolation(
                    "inherited frame policy cannot replace committed policy".into(),
                ));
            }
        } else {
            if policies.frames.len() >= 4096 {
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "frame worker policies".into(),
                });
            }
            let private_isolation = private
                .as_ref()
                .and_then(|(_, lifetime)| policies.prerenders.get(&(page, lifetime.0, lifetime.1)))
                .and_then(|scope| scope.policy.as_ref())
                .map(|policy| {
                    crate::agent_isolation::DocumentIsolationPolicy::from_headers(&policy.headers)
                });
            let isolation = if matches!(parent, oryn_js::message_wire::WindowTarget::Root)
                && let Some(isolation) = private_isolation
            {
                isolation
            } else {
                match parent {
                    oryn_js::message_wire::WindowTarget::Root => policies.root_isolation.get(&page),
                    oryn_js::message_wire::WindowTarget::Frame(frame) => policies
                        .frame_isolation
                        .get(&(page, frame.clone(), parent_generation, parent_epoch)),
                }
                .copied()
                .unwrap_or_default()
            };
            if let Some((token, lifetime)) = &private {
                hub.commit_prerender_frame_origin(oryn_js::message_wire::PrerenderFrameOrigin {
                    page: page.0,
                    token,
                    lifetime: *lifetime,
                    frame: &frame,
                    generation,
                    epoch,
                    origin: claimed.clone(),
                })
                .map_err(RuntimeError::ContextService)?;
            } else {
                hub.commit_frame_origin(page.0, &frame, generation, epoch, claimed.clone())
                    .map_err(RuntimeError::ContextService)?;
            }
            let security_epoch = private.as_ref().map_or_else(
                || {
                    policies
                        .root_lifetimes
                        .get(&page)
                        .map_or(0, |lifetime| lifetime.epoch)
                },
                |(_, lifetime)| lifetime.1,
            );
            policies
                .frame_security_epochs
                .insert(key.clone(), security_epoch);
            policies.frame_origins.insert(key.clone(), claimed);
            policies.frame_isolation.insert(key.clone(), isolation);
            policies.frames.insert(key, policy);
        }
        drop(policies);
        drop(hub);
        self.register_activation_parent(ActivationParentInputs {
            page,
            frame: frame.clone(),
            generation,
            epoch,
            parent: parent.clone(),
            parent_generation,
            parent_epoch,
        })?;
        if newly_bound && document_url != "about:neterror" {
            self.inherit_document_domain(
                &AgentOwner::Window {
                    page,
                    target: parent.clone(),
                    generation: parent_generation,
                    epoch: parent_epoch,
                },
                &AgentOwner::Window {
                    page,
                    target: oryn_js::message_wire::WindowTarget::Frame(frame.clone()),
                    generation,
                    epoch,
                },
            )
            .map_err(RuntimeError::ContextService)?;
        }
        #[cfg(unix)]
        if newly_bound && let Some(services) = self.existing_service_registry() {
            services
                .control_client(
                    AgentOwner::Window {
                        page,
                        target: oryn_js::message_wire::WindowTarget::Frame(frame),
                        generation,
                        epoch,
                    },
                    None,
                )
                .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        }
        Ok(())
    }
}

impl BrowserContext {
    pub(crate) fn agent_indexed_db(
        &self,
        id: AgentId,
        command: oryn_js::platform::IdbHostCommand,
    ) -> Result<oryn_js::platform::IdbHostResponse, WorkerError> {
        let bootstrap = self.worker_bootstrap(id)?;
        if bootstrap.origin.opaque {
            return Err(WorkerError::new(
                "SecurityError",
                "opaque worker origins cannot access IndexedDB",
            ));
        }
        let origin = crate::storage::partitioned_idb_origin(
            &bootstrap.partition,
            &bootstrap.origin.authority,
        );
        let mut command: crate::storage::IdbCommand = serde_json::from_value(
            serde_json::to_value(command)
                .map_err(|error| WorkerError::new("DataError", error.to_string()))?,
        )
        .map_err(|error| WorkerError::new("DataError", error.to_string()))?;
        crate::storage::validate_script_idb_command(&command)
            .map_err(|error| WorkerError::new("SecurityError", error.to_string()))?;
        crate::storage::scope_idb_transaction(&mut command, &format!("agent:{}", id.0));
        match self
            .context_store()
            .execute(crate::storage::StorageRequest::IndexedDb { origin, command })
            .map_err(|error| WorkerError::new("UnknownError", error.to_string()))?
        {
            crate::storage::StorageResponse::IndexedDb { response } => serde_json::from_value(
                serde_json::to_value(response)
                    .map_err(|error| WorkerError::new("DataError", error.to_string()))?,
            )
            .map_err(|error| WorkerError::new("DataError", error.to_string())),
            _ => Err(WorkerError::new(
                "UnknownError",
                "IndexedDB storage response mismatch",
            )),
        }
    }
}

#[cfg(unix)]
impl BrowserContext {
    pub(crate) fn worker_cookie_ancestor_urls(
        &self,
        owner: &AgentOwner,
    ) -> Result<Vec<String>, WorkerError> {
        let (_, origin, policy) = self.native_worker_authority(owner)?;
        let urls = match owner {
            AgentOwner::Window { .. } => {
                let mut urls = self
                    .window_ancestor_urls(owner)
                    .map_err(|error| WorkerError::new("SecurityError", error.to_string()))?;
                urls.push(if origin.opaque {
                    "about:blank".into()
                } else {
                    policy.url
                });
                urls
            }
            AgentOwner::Agent(id) => self.worker_bootstrap(*id)?.cookie_ancestor_urls,
        };
        if urls.len() > 64
            || urls
                .iter()
                .try_fold(0usize, |size, url| size.checked_add(url.len()))
                .is_none_or(|size| size > 65536)
        {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "worker cookie ancestry capacity",
            ));
        }
        self.native_worker_authority(owner)?;
        Ok(urls)
    }
    pub(crate) fn agent_cookie_header(
        &self,
        bootstrap: &AgentBootstrap,
        url: &url::Url,
    ) -> Option<String> {
        self.inner.parent_cookies.as_ref().and_then(|cookies| {
            cookies
                .header_for_method(
                    url,
                    "GET",
                    &crate::cookies::CookieRequestContext {
                        dns_timing: None,
                        kind: crate::cookies::CookieRequestKind::WebSocket,
                        initiator_url: Some(bootstrap.origin.serialized.clone()),
                        top_level_url: bootstrap.top_level_url.clone(),
                        ancestor_urls: bootstrap.cookie_ancestor_urls.clone(),
                        ..Default::default()
                    },
                )
                .0
        })
    }
}

impl BrowserContext {
    pub(crate) fn agent_limits(&self) -> oryn_js::worker_wire::AgentLimits {
        oryn_js::worker_wire::AgentLimits {
            heap_bytes: self.inner.config.max_v8_heap_bytes,
            queue_bytes: self.inner.config.max_ipc_frame_bytes,
            // Page-command admission is a small RPC queue. Independent JS
            // agents need the worker-wire mailbox budget: one synchronous
            // task can publish many events before its owner gets a turn.
            queue_messages: oryn_js::worker_wire::AgentLimits::default().queue_messages,
            task_wall_ms: self
                .inner
                .config
                .command_timeout
                .as_millis()
                .min(u64::MAX as u128) as u64,
        }
    }
    pub(crate) fn agent_child_mode(&self) -> bool {
        self.inner.worker_process || !cfg!(feature = "v8-host")
    }
    pub(crate) fn agent_message_request(
        &self,
        id: AgentId,
        request: oryn_js::message_wire::MessageRequest,
    ) -> Result<oryn_js::message_wire::MessageResponse, WorkerError> {
        let wake = request.wakes_recipients();
        let (result, pages) = {
            let agents = self
                .inner
                .agents
                .lock()
                .map_err(|_| worker_lock_error("agent registry"))?;
            let (partition, origin) = agents.principal(id)?;
            let mut hub = self
                .inner
                .group
                .messages
                .lock()
                .map_err(|_| worker_lock_error("message hub"))?;
            hub.set_partition(
                oryn_js::message_wire::Principal::Agent(id.0),
                partition.into(),
            );
            let result = hub
                .handle_agent(id.0, request, &origin.serialized, &origin.authority)
                .map_err(|message| WorkerError::new("DataCloneError", message));
            let pages = if wake && result.is_ok() {
                hub.pending_pages()
            } else {
                Vec::new()
            };
            (result, pages)
        };
        #[cfg(unix)]
        for page in pages {
            self.wake_context_events(PageId(page));
        }
        #[cfg(not(unix))]
        let _ = pages;
        result
    }
}

#[derive(Clone)]
pub(crate) struct NativeBlobResource {
    pub(super) owner: AgentOwner,
    pub(crate) partition: String,
    pub(crate) origin: oryn_js::worker_wire::AgentOrigin,
    pub(crate) policy: oryn_js::worker_wire::AgentPolicy,
    pub(crate) bytes: Vec<u8>,
    pub(crate) media_type: String,
}
impl BrowserContext {
    pub(crate) fn native_worker_authority(
        &self,
        owner: &AgentOwner,
    ) -> Result<
        (
            String,
            oryn_js::worker_wire::AgentOrigin,
            oryn_js::worker_wire::AgentPolicy,
        ),
        WorkerError,
    > {
        match owner {
            AgentOwner::Window {
                page,
                target,
                generation,
                epoch,
            } => {
                let (_, partition, origin) = self
                    .window_worker_creator(*page, target, *generation, *epoch)
                    .map_err(|error| WorkerError::new("InvalidStateError", error.to_string()))?;
                let policy = self
                    .window_worker_policy(*page, target, *generation, *epoch)
                    .map_err(|error| WorkerError::new("InvalidStateError", error.to_string()))?;
                Ok((partition, origin, policy))
            }
            AgentOwner::Agent(id) => {
                let bootstrap = self.worker_bootstrap(*id)?;
                Ok((bootstrap.partition, bootstrap.origin, bootstrap.policy))
            }
        }
    }
    /// Existing resources/jobs may retain a frozen document. This accessor is
    /// never used for author Create/Import/Fetch admission, which stays active-
    /// document-only through native_worker_authority.
    /// Persistence is isolated by the profile-owned ContextStore. Only the
    /// ephemeral context component is removed from a parent-generated key;
    /// schemeful top-level site and native origin identity remain distinct.
    pub(crate) fn service_cache_operation(
        &self,
        owner: &AgentOwner,
        lease: crate::storage::cache::CacheLeaseOwner,
        operation: crate::storage::cache::CacheOperation,
    ) -> Result<crate::storage::cache::CacheResult, WorkerError> {
        let (partition, origin) = self.service_storage_authority(owner)?;
        let result = self
            .context_store()
            .cache_operation(lease, &partition, &origin, operation)
            .map_err(|error| {
                WorkerError::new(
                    match &error {
                        crate::storage::StorageError::QuotaExceeded { .. } => "QuotaExceededError",
                        crate::storage::StorageError::DataClone(_) => "TypeError",
                        crate::storage::StorageError::Constraint => "InvalidStateError",
                        _ => "UnknownError",
                    },
                    error.to_string(),
                )
            })?;
        self.native_worker_authority(owner)?;
        Ok(result)
    }
    pub(crate) fn service_storage_authority(
        &self,
        owner: &AgentOwner,
    ) -> Result<(String, String), WorkerError> {
        let (partition, origin, _) = self.native_worker_authority(owner)?;
        if origin.opaque {
            return Err(WorkerError::new(
                "SecurityError",
                "opaque service storage origin",
            ));
        }
        let parts: serde_json::Value = serde_json::from_str(&partition)
            .map_err(|_| worker_lock_error("invalid native storage partition"))?;
        let parts = parts
            .as_array()
            .ok_or_else(|| worker_lock_error("invalid native storage partition"))?;
        if parts.len() < 3
            || !parts[0].is_u64()
            || !matches!(parts[1].as_str(), Some("site" | "origin"))
        {
            return Err(worker_lock_error("invalid native storage partition"));
        }
        Ok((
            serde_json::to_string(&parts[1..])
                .map_err(|_| worker_lock_error("native storage partition encoding"))?,
            origin.authority,
        ))
    }
    pub(crate) fn retained_worker_authority(
        &self,
        owner: &AgentOwner,
    ) -> Result<
        (
            String,
            oryn_js::worker_wire::AgentOrigin,
            oryn_js::worker_wire::AgentPolicy,
        ),
        WorkerError,
    > {
        match self.native_worker_authority(owner) {
            Ok(authority) => return Ok(authority),
            Err(error) if matches!(owner, AgentOwner::Agent(_)) => return Err(error),
            Err(_) => {}
        }
        let AgentOwner::Window { page, .. } = owner else {
            unreachable!()
        };
        let pages = self
            .inner
            .group
            .pages
            .lock()
            .map_err(|_| worker_lock_error("page registry"))?;
        if !pages.get(page).is_some_and(|record| record.is_live()) {
            return Err(WorkerError::new(
                "InvalidStateError",
                "resource owner page retired",
            ));
        }
        let snapshots = self
            .inner
            .cached_document_security
            .lock()
            .map_err(|_| worker_lock_error("cached security"))?;
        snapshots
            .values()
            .filter(|snapshot| snapshot.page == *page && !snapshot.evicted)
            .find_map(|snapshot| snapshot.worker_authorities.get(owner).cloned())
            .ok_or_else(|| WorkerError::new("InvalidStateError", "resource owner document retired"))
    }
    pub(crate) fn clear_owner_blob_urls(&self, owner: &AgentOwner) {
        self.inner
            .worker_policies
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .blobs
            .retain(|_, blob| &blob.owner != owner);
    }
    pub(crate) fn register_native_blob(
        &self,
        owner: AgentOwner,
        bytes: Vec<u8>,
        media_type: String,
    ) -> Result<String, WorkerError> {
        let (partition, origin, policy) = self.native_worker_authority(&owner)?;
        if bytes.len() > 4 * 1024 * 1024 || media_type.len() > 1024 {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "blob URL resource limit",
            ));
        }
        let token = uuid::Uuid::new_v4();
        let url = format!("blob:{}/{}", origin.serialized, token);
        let mut policies = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| worker_lock_error("blob resources"))?;
        let retained = policies
            .blobs
            .values()
            .fold(0usize, |total, blob| total.saturating_add(blob.bytes.len()));
        // Bound both metadata and payload; tiny independent URLs should not
        // exhaust the registry long before the byte budget is reached.
        if policies.blobs.len() >= 64 * 1024
            || retained.saturating_add(bytes.len()) > 64 * 1024 * 1024
        {
            return Err(WorkerError::new(
                "QuotaExceededError",
                "blob URL registry limit",
            ));
        }
        policies.blobs.insert(
            url.clone(),
            NativeBlobResource {
                owner: owner.clone(),
                partition,
                origin,
                policy,
                bytes,
                media_type,
            },
        );
        drop(policies);
        if let Err(error) = self.native_worker_authority(&owner) {
            self.inner
                .worker_policies
                .lock()
                .map_err(|_| worker_lock_error("blob resources"))?
                .blobs
                .remove(&url);
            return Err(error);
        }
        Ok(url)
    }
    pub(crate) fn resolve_native_blob(
        &self,
        caller: &AgentOwner,
        url: &str,
    ) -> Result<NativeBlobResource, WorkerError> {
        let (partition, origin, _) = self.native_worker_authority(caller)?;
        // The fragment belongs to the consumer document, not the Blob store
        // key. A query remains part of that key; revocation still uses the
        // exact registered URL, as required by File API.
        let url = url.split_once('#').map_or(url, |(base, _)| base);
        let resource = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| worker_lock_error("blob resources"))?
            .blobs
            .get(url)
            .cloned()
            .ok_or_else(|| {
                WorkerError::new("NetworkError", "revoked or unknown blob URL")
                    .with_script_failure(oryn_js::worker_wire::ScriptFailureKind::Fetch)
            })?;
        if resource.partition != partition || resource.origin.authority != origin.authority {
            return Err(
                WorkerError::new("SecurityError", "foreign blob URL partition or origin")
                    .with_script_failure(oryn_js::worker_wire::ScriptFailureKind::Fetch),
            );
        }
        // Closing its creator revokes future lookup; an already returned copy
        // remains owned by its consumer and is unaffected by URL revocation.
        // A registry/authority lookup failure is not proof of revocation.
        // Explicit owner retirement removes its URLs through clear_owner_blob_urls.
        // Preserve local failure provenance and leave a retained URL retryable.
        self.retained_worker_authority(&resource.owner)?;
        self.native_worker_authority(caller)?;
        Ok(resource)
    }
    pub(crate) fn revoke_native_blob(
        &self,
        caller: &AgentOwner,
        url: &str,
    ) -> Result<(), WorkerError> {
        let (partition, origin, _) = self.native_worker_authority(caller)?;
        let mut policies = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| worker_lock_error("blob resources"))?;
        if policies.blobs.get(url).is_some_and(|resource| {
            resource.partition == partition && resource.origin.authority == origin.authority
        }) {
            policies.blobs.remove(url);
        }
        Ok(())
    }
}

#[cfg(test)]
mod blob_authority_tests {
    use super::*;
    use oryn_js::message_wire::{MessageRequest, WindowTarget};
    fn page(context: &BrowserContext, url: &str) -> (PageHandle, AgentOwner) {
        let page = context.new_page();
        context.commit_page_url(page.page_id(), url.into()).unwrap();
        context
            .install_document_security(page.page_id(), url.into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let owner = context
            .window_worker_creator(page.page_id(), &WindowTarget::Root, 1, 0)
            .unwrap()
            .0;
        (page, owner)
    }
    #[test]
    fn local_frame_ticket_preserves_blob_opaque_identity_and_sandbox_creates_distinct_identity() {
        let context = Browser::new().new_context();
        let (handle, owner) = page(&context, "data:text/html,parent");
        let page = handle.page_id();
        let (_, origin, policy) = context.native_worker_authority(&owner).unwrap();
        for (letter, sandboxed) in [('a', false), ('b', true)] {
            let frame = letter.to_string().repeat(64);
            let token = context
                .prepare_local_frame_worker_policy(
                    page,
                    "blob:null/native",
                    &policy,
                    "",
                    origin.clone(),
                )
                .unwrap();
            context
                .handle_message_wire(
                    page,
                    MessageRequest::RegisterWindow {
                        frame: frame.clone(),
                        document_generation: 2,
                        stream_epoch: 0,
                        origin: "null".into(),
                        inherit_origin: None,
                    },
                )
                .unwrap();
            context
                .bind_frame_worker_policy_with_sandbox(FramePolicyInputs {
                    page,
                    frame: frame.clone(),
                    generation: 2,
                    epoch: 0,
                    url: "blob:null/native",
                    token: &token,
                    opaque_sandbox: sandboxed,
                })
                .unwrap();
            let (_, _, bound) = context
                .window_worker_creator(page, &WindowTarget::Frame(frame), 2, 0)
                .unwrap();
            assert_eq!(bound.authority == origin.authority, !sandboxed);
            assert_eq!(bound.serialized, "null");
        }
        let frame = "c".repeat(64);
        let token = context
            .prepare_local_frame_worker_policy(page, "blob:null/native", &policy, "", origin)
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                    origin: "https://forged.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        assert!(
            context
                .bind_frame_worker_policy(page, frame, 2, 0, "blob:null/native", &token)
                .is_err()
        );
    }

    #[test]
    fn blob_urls_use_uuids_and_resolve_fragments_without_relaxing_authority() {
        let context = Browser::new().new_context();
        let (_handle, owner) = page(&context, "https://a.test/owner");
        let (_other, foreign) = page(&context, "https://b.test/foreign");
        let url = context
            .register_native_blob(owner.clone(), b"payload".to_vec(), "text/plain".into())
            .unwrap();
        let id = uuid::Uuid::parse_str(url.rsplit('/').next().unwrap()).unwrap();
        assert_eq!(id.get_version_num(), 4);
        assert_eq!(id.to_string(), url.rsplit('/').next().unwrap());
        let fragment = format!("{url}#part");
        assert_eq!(
            context
                .resolve_native_blob(&owner, &fragment)
                .unwrap()
                .bytes,
            b"payload"
        );
        assert!(context.resolve_native_blob(&foreign, &fragment).is_err());
        assert!(
            context
                .resolve_native_blob(&owner, &format!("{url}?changed#part"))
                .is_err()
        );
        // Revocation with a fragment does not identify a registered URL.
        context.revoke_native_blob(&owner, &fragment).unwrap();
        assert!(context.resolve_native_blob(&owner, &url).is_ok());
        context.revoke_native_blob(&owner, &url).unwrap();
        assert!(context.resolve_native_blob(&owner, &fragment).is_err());
    }

    #[test]
    fn thousands_of_small_blob_urls_fit_within_the_resource_budget() {
        let context = Browser::new().new_context();
        let (_handle, owner) = page(&context, "https://a.test/owner");
        let mut urls = std::collections::BTreeSet::new();
        for _ in 0..5000 {
            assert!(
                urls.insert(
                    context
                        .register_native_blob(owner.clone(), b"test".to_vec(), "text/plain".into())
                        .unwrap()
                )
            );
        }
        for url in urls {
            context.revoke_native_blob(&owner, &url).unwrap();
        }
        assert!(
            context
                .inner
                .worker_policies
                .lock()
                .unwrap()
                .blobs
                .is_empty()
        );
        assert!(
            context
                .register_native_blob(owner, vec![0; 4 * 1024 * 1024 + 1], String::new())
                .is_err()
        );
    }

    #[test]
    fn blob_url_resolution_shares_native_origin_and_revocation_preserves_retained_copy() {
        let context = Browser::new().new_context();
        let (first, owner) = page(&context, "https://a.test/first");
        let (_second, peer) = page(&context, "https://a.test/second");
        let (_foreign, foreign) = page(&context, "https://b.test/");
        let url = context
            .register_native_blob(
                owner.clone(),
                b"postMessage(1)".to_vec(),
                "text/javascript".into(),
            )
            .unwrap();
        let copy = context.resolve_native_blob(&peer, &url).unwrap();
        assert!(context.resolve_native_blob(&foreign, &url).is_err());
        context.revoke_native_blob(&foreign, &url).unwrap();
        assert!(context.resolve_native_blob(&owner, &url).is_ok());
        context.revoke_native_blob(&peer, &url).unwrap();
        assert!(context.resolve_native_blob(&owner, &url).is_err());
        assert_eq!(copy.bytes, b"postMessage(1)");
        let second = context
            .register_native_blob(owner, b"retained".to_vec(), "text/plain".into())
            .unwrap();
        context
            .handle_message_wire(
                first.page_id(),
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(context.resolve_native_blob(&peer, &second).is_err());
        assert!(
            !context
                .inner
                .worker_policies
                .lock()
                .unwrap()
                .blobs
                .contains_key(&second)
        );
    }
    #[test]
    fn blob_fetch_failure_provenance_does_not_classify_stale_caller_as_fetch() {
        use oryn_js::worker_wire::ScriptFailureKind;
        let context = Browser::new().new_context();
        let (owner_page, owner) = page(&context, "https://a.test/owner");
        let (_peer_page, peer) = page(&context, "https://b.test/peer");
        let url = context
            .register_native_blob(
                owner.clone(),
                b"export default 1".to_vec(),
                "text/javascript".into(),
            )
            .unwrap();
        assert_eq!(
            context
                .resolve_native_blob(&peer, &url)
                .err()
                .unwrap()
                .script_failure,
            Some(ScriptFailureKind::Fetch)
        );
        context.revoke_native_blob(&owner, &url).unwrap();
        assert_eq!(
            context
                .resolve_native_blob(&owner, &url)
                .err()
                .unwrap()
                .script_failure,
            Some(ScriptFailureKind::Fetch)
        );
        context
            .handle_message_wire(
                owner_page.page_id(),
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert_eq!(
            context
                .resolve_native_blob(&owner, &url)
                .err()
                .unwrap()
                .script_failure,
            None
        );
    }
    #[test]
    fn blob_url_is_unguessable_and_cannot_cross_context_registry() {
        let first = Browser::new().new_context();
        let (_page, owner) = page(&first, "https://a.test/");
        let second = Browser::new().new_context();
        let (_peer, peer) = page(&second, "https://a.test/");
        let a = first
            .register_native_blob(owner.clone(), vec![1], "text/plain".into())
            .unwrap();
        let b = first
            .register_native_blob(owner, vec![1], "text/plain".into())
            .unwrap();
        assert_ne!(a, b);
        assert_eq!(
            uuid::Uuid::parse_str(a.rsplit('/').next().unwrap())
                .unwrap()
                .get_version_num(),
            4
        );
        assert!(second.resolve_native_blob(&peer, &a).is_err());
    }
}

#[cfg(unix)]
impl BrowserContext {
    pub(crate) fn live_service_clients(&self) -> Vec<AgentOwner> {
        let documents = self
            .inner
            .group
            .messages
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .registered_documents();
        let mut clients = documents
            .into_iter()
            .filter_map(|(page, target, generation, epoch)| {
                self.window_worker_creator(PageId(page), &target, generation, epoch)
                    .ok()
                    .map(|(owner, _, _)| owner)
            })
            .collect::<Vec<_>>();
        if let Ok(agents) = self.inner.agents.lock() {
            clients.extend(
                agents
                    .live_service_client_agents()
                    .into_iter()
                    .map(AgentOwner::Agent),
            );
        }
        clients
    }
    pub(crate) fn service_navigation_partition(
        &self,
        page: PageId,
        url: &url::Url,
    ) -> Result<String, WorkerError> {
        self.page_storage_partition(page)
            .map_err(|error| WorkerError::new("InvalidStateError", error.to_string()))?;
        Ok(native_storage_partition(
            self.inner.id,
            &SecurityOrigin::from_url(url.as_str()),
        ))
    }
    pub(crate) fn cancel_service_navigation(&self, page: PageId, request: u64) {
        if let Some(registry) = self.existing_service_registry() {
            registry.cancel_navigation(page, request);
        }
    }
    pub(crate) fn intercept_service_navigation(
        &self,
        page: PageId,
        request: &crate::network::NetworkRequest,
        network: SharedNetworkBroker,
        timing: &crate::network::ServiceWorkerTiming,
    ) -> Result<Option<crate::network::NetworkResponse>, crate::network::NetworkError> {
        if !matches!(
            request.security.kind,
            crate::cookies::CookieRequestKind::Document | crate::cookies::CookieRequestKind::Frame
        ) {
            return Ok(None);
        }
        let registry = self
            .service_registry()
            .map_err(|error| crate::network::NetworkError::Transport(error.to_string()))?;
        registry
            .intercept_navigation_timed(page, request, network, timing)
            .map_err(|error| crate::network::NetworkError::Transport(error.to_string()))
    }
    pub(crate) fn intercept_service_navigation_operation(
        &self,
        page: PageId,
        request: &crate::network::NetworkRequest,
        network: SharedNetworkBroker,
        timing: &crate::network::ServiceWorkerTiming,
    ) -> Result<Option<crate::network::NetworkOperation>, crate::network::NetworkError> {
        if !matches!(
            request.security.kind,
            crate::cookies::CookieRequestKind::Document | crate::cookies::CookieRequestKind::Frame
        ) {
            return Ok(None);
        }
        let registry = self
            .service_registry()
            .map_err(|error| crate::network::NetworkError::Transport(error.to_string()))?;
        registry
            .intercept_navigation_operation_timed(page, request, network, timing)
            .map_err(|error| crate::network::NetworkError::Transport(error.to_string()))
    }
    pub(crate) fn retire_page_services(&self, page: PageId) -> Result<(), WorkerError> {
        if let Some(services) = self.existing_service_registry() {
            services.retire_page(page)
        } else {
            Ok(())
        }
    }
    pub(crate) fn existing_service_registry(
        &self,
    ) -> Option<std::sync::Arc<crate::service_registry::ServiceRegistry>> {
        self.inner
            .worker_policies
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .service
            .clone()
    }
    pub(crate) fn service_registry(
        &self,
    ) -> Result<std::sync::Arc<crate::service_registry::ServiceRegistry>, WorkerError> {
        let mut policies = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| worker_lock_error("service registry slot"))?;
        Ok(policies
            .service
            .get_or_insert_with(|| crate::service_registry::ServiceRegistry::new(self))
            .clone())
    }
}

#[cfg(all(test, unix))]
mod secure_context_authority_tests {
    #[test]
    fn opaque_local_root_publication_matches_its_native_admission_and_keeps_storage_denied() {
        for precursor in ["https://example.test/source", "http://example.test/source"] {
            for url in ["data:text/html,opaque", "about:blank"] {
                let context = Browser::in_process_with_config(Default::default()).new_context();
                let page = context.new_page();
                let id = page.page_id();
                context.commit_page_url(id, url.into()).unwrap();
                let mut policy = crate::csp::CspPolicySet::from_headers(precursor, &[]);
                policy.rebase_document(url);
                context
                    .install_document_security_identity(
                        id,
                        url.into(),
                        &[],
                        Some(SecurityOrigin::opaque()),
                        Some(policy),
                        None,
                    )
                    .unwrap();
                let (serialized, opaque_key, secure, generation, _) =
                    context.native_document_authority(id, url).unwrap();
                assert_eq!(serialized, "null");
                assert!(opaque_key.is_some());
                assert_eq!(secure, precursor.starts_with("https:"));
                context
                    .handle_message_wire(
                        id,
                        MessageRequest::RegisterRootDocument {
                            document_generation: generation,
                            stream_epoch: 0,
                        },
                    )
                    .unwrap();
                let owner = context
                    .window_worker_creator(id, &WindowTarget::Root, generation, 0)
                    .unwrap()
                    .0;
                if !secure {
                    assert!(
                        context
                            .publish_worker_secure_context(owner.clone(), true)
                            .is_err()
                    );
                }
                context
                    .publish_worker_secure_context(owner.clone(), secure)
                    .unwrap();
                assert_eq!(context.worker_secure_context(&owner).unwrap(), secure);
                let (_, origin, retained) = context.native_worker_authority(&owner).unwrap();
                assert!(origin.opaque);
                assert_eq!(origin.serialized, "null");
                assert_eq!(retained.url, url);
                assert_eq!(retained.policy_self_url.as_deref(), Some(precursor));
                assert_eq!(
                    context.service_storage_authority(&owner).unwrap_err().name,
                    "SecurityError"
                );
                context
                    .handle_message_wire(
                        id,
                        MessageRequest::RetireRootDocument {
                            document_generation: generation,
                            stream_epoch: 0,
                        },
                    )
                    .unwrap();
                assert!(
                    context
                        .publish_worker_secure_context(owner, secure)
                        .is_err()
                );
            }
        }
    }

    #[test]
    fn opaque_srcdoc_publication_uses_the_bound_parent_policy_and_exact_child_lifetime() {
        for precursor in ["https://example.test/source", "http://example.test/source"] {
            let context = Browser::in_process_with_config(Default::default()).new_context();
            let page = context.new_page();
            let id = page.page_id();
            context.commit_page_url(id, precursor.into()).unwrap();
            context
                .install_document_security(id, precursor.into(), &[])
                .unwrap();
            context
                .handle_message_wire(
                    id,
                    MessageRequest::RegisterRootDocument {
                        document_generation: 1,
                        stream_epoch: 0,
                    },
                )
                .unwrap();
            let parent = context
                .window_worker_creator(id, &WindowTarget::Root, 1, 0)
                .unwrap()
                .0;
            let secure = precursor.starts_with("https:");
            context
                .publish_worker_secure_context(parent, secure)
                .unwrap();
            let frame = "a".repeat(64);
            context
                .handle_message_wire(
                    id,
                    MessageRequest::RegisterWindow {
                        frame: frame.clone(),
                        document_generation: 2,
                        stream_epoch: 0,
                        origin: "null".into(),
                        inherit_origin: None,
                    },
                )
                .unwrap();
            context
                .inherit_frame_worker_policy_at_url(InheritedFramePolicyInputs {
                    page: id,
                    frame: frame.clone(),
                    generation: 2,
                    epoch: 0,
                    parent: &WindowTarget::Root,
                    parent_generation: 1,
                    parent_epoch: 0,
                    document_url: "about:srcdoc",
                })
                .unwrap();
            let target = WindowTarget::Frame(frame);
            let owner = context.window_worker_creator(id, &target, 2, 0).unwrap().0;
            if !secure {
                assert!(
                    context
                        .publish_worker_secure_context(owner.clone(), true)
                        .is_err()
                );
            }
            context
                .publish_worker_secure_context(owner.clone(), secure)
                .unwrap();
            let (_, origin, policy) = context.native_worker_authority(&owner).unwrap();
            assert!(origin.opaque);
            assert_eq!(origin.serialized, "null");
            assert_eq!(policy.url, "about:srcdoc");
            assert_eq!(policy.policy_self_url.as_deref(), Some(precursor));
            assert_eq!(context.worker_secure_context(&owner).unwrap(), secure);
            assert_eq!(
                context.service_storage_authority(&owner).unwrap_err().name,
                "SecurityError"
            );
            let wrong = AgentOwner::Window {
                page: id,
                target,
                generation: 3,
                epoch: 0,
            };
            assert!(
                context
                    .publish_worker_secure_context(wrong, secure)
                    .is_err()
            );
        }
    }

    use super::*;
    use oryn_js::message_wire::{MessageRequest, WindowTarget};
    #[test]
    fn secure_context_publication_is_immutable_and_bound_to_document_lifetime() {
        let context = Browser::in_process_with_config(Default::default()).new_context();
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "https://example.test/".into())
            .unwrap();
        context
            .install_document_security(page.page_id(), "https://example.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let owner = context
            .window_worker_creator(page.page_id(), &WindowTarget::Root, 1, 0)
            .unwrap()
            .0;
        assert!(
            !context.worker_secure_context(&owner).unwrap(),
            "missing native publication fails closed"
        );
        context
            .publish_worker_secure_context(owner.clone(), false)
            .unwrap();
        assert!(
            context
                .publish_worker_secure_context(owner.clone(), true)
                .is_err()
        );
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            !context
                .inner
                .worker_policies
                .lock()
                .unwrap()
                .secure_contexts
                .contains_key(&owner),
            "retirement clears authority even without a service registry"
        );
        context
            .install_document_security(page.page_id(), "https://example.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(context.publish_worker_secure_context(owner, true).is_err());
        let owner = context
            .window_worker_creator(page.page_id(), &WindowTarget::Root, 2, 0)
            .unwrap()
            .0;
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        assert!(context.worker_secure_context(&owner).unwrap());
        context.clear_page_worker_policies(page.page_id());
        let retired = context.worker_secure_context(&owner).unwrap_err();
        assert_eq!(retired.name, "InvalidStateError");
        assert!(
            retired
                .message
                .contains("root document security publication retired")
        );
        assert!(
            context
                .publish_worker_secure_context(owner.clone(), true)
                .is_err()
        );
        assert!(
            !context
                .inner
                .worker_policies
                .lock()
                .unwrap()
                .secure_contexts
                .contains_key(&owner)
        );
    }
    #[test]
    fn plain_http_document_cannot_publish_secure_authority() {
        let context = Browser::in_process_with_config(Default::default()).new_context();
        let page = context.new_page();
        context
            .commit_page_url(page.page_id(), "http://example.test/".into())
            .unwrap();
        context
            .install_document_security(page.page_id(), "http://example.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let owner = context
            .window_worker_creator(page.page_id(), &WindowTarget::Root, 1, 0)
            .unwrap()
            .0;
        assert!(context.publish_worker_secure_context(owner, true).is_err());
    }
}

impl WorkerPolicyRegistry {
    pub(super) fn frame_document_url(
        &self,
        page: PageId,
        frame: &str,
        generation: u64,
        epoch: u64,
    ) -> Option<&str> {
        self.frames
            .get(&(page, frame.to_owned(), generation, epoch))
            .map(|policy| policy.url.as_str())
    }

    fn transient_activation(&mut self, owner: &AgentOwner, consume: bool, now: Instant) -> bool {
        self.activations.retain(|_, (created, _, _)| {
            now.saturating_duration_since(*created) < Duration::from_secs(5)
        });
        let AgentOwner::Window { page, .. } = owner else {
            return false;
        };
        if !self.activations.contains_key(owner) {
            return false;
        }
        if consume {
            // Activation consumption visits the entire top-level traversable,
            // including cross-origin descendants, under one coordinator lock.
            self.activations.retain(|candidate,_|!matches!(candidate,AgentOwner::Window{page:other,..} if other==page));
        }
        true
    }
}

#[cfg(test)]
mod transient_activation_tests {
    use super::*;
    use oryn_js::message_wire::WindowTarget;
    fn owner(page: u64, target: WindowTarget, generation: u64) -> AgentOwner {
        AgentOwner::Window {
            page: PageId(page),
            target,
            generation,
            epoch: 0,
        }
    }
    #[test]
    fn transient_grant_is_exact_expiring_and_consumed_across_page() {
        let root = owner(1, WindowTarget::Root, 7);
        let frame = owner(1, WindowTarget::Frame("child".into()), 3);
        let other = owner(2, WindowTarget::Root, 7);
        let successor = owner(1, WindowTarget::Root, 8);
        let now = Instant::now();
        let mut registry = WorkerPolicyRegistry::default();
        for key in [&root, &frame, &other] {
            registry.activations.insert(
                key.clone(),
                (now, "partition".into(), "https://example.test".into()),
            );
        }
        assert!(!registry.transient_activation(&successor, true, now));
        assert!(registry.transient_activation(&root, false, now));
        assert!(registry.transient_activation(&root, false, now));
        assert!(registry.transient_activation(&frame, true, now));
        assert!(!registry.transient_activation(&root, false, now));
        assert!(!registry.transient_activation(&frame, true, now));
        assert!(registry.transient_activation(&other, false, now));
        assert!(!registry.transient_activation(&other, true, now + Duration::from_secs(5)));
    }
    #[test]
    fn retired_generation_cannot_query_or_consume_successor_activation() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .commit_page_url(id, "https://example.test/".into())
            .unwrap();
        context
            .handle_message_wire(
                id,
                oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let old = AgentOwner::Window {
            page: id,
            target: WindowTarget::Root,
            generation: 1,
            epoch: 0,
        };
        context.record_native_activation(old.clone()).unwrap();
        assert!(context.transient_native_activation(&old, false).unwrap());
        context
            .handle_message_wire(
                id,
                oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let new = AgentOwner::Window {
            page: id,
            target: WindowTarget::Root,
            generation: 2,
            epoch: 0,
        };
        context.record_native_activation(new.clone()).unwrap();
        assert!(context.transient_native_activation(&old, true).is_err());
        assert!(context.transient_native_activation(&new, true).unwrap());
        assert!(!context.transient_native_activation(&new, true).unwrap());
    }
}

fn activation_descendant(
    parents: &BTreeMap<AgentOwner, AgentOwner>,
    child: &AgentOwner,
    ancestor: &AgentOwner,
) -> bool {
    let mut current = child;
    for _ in 0..=parents.len() {
        let Some(parent) = parents.get(current) else {
            return false;
        };
        if parent == ancestor {
            return true;
        }
        current = parent;
    }
    false
}

#[cfg(all(test, unix))]
mod activation_tree_tests {
    use super::*;
    use oryn_js::message_wire::{MessageRequest, WindowTarget};
    fn frame(
        context: &BrowserContext,
        page: PageId,
        name: &str,
        origin: &str,
        parent: &AgentOwner,
    ) -> AgentOwner {
        let token = name.repeat(64);
        let url = format!("{origin}/frame");
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterWindow {
                    frame: token.clone(),
                    document_generation: 1,
                    stream_epoch: 0,
                    origin: origin.into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        let policy = context
            .prepare_frame_worker_policy(page, &url, &[], "<body></body>")
            .unwrap();
        context
            .bind_frame_worker_policy(page, token.clone(), 1, 0, &url, &policy)
            .unwrap();
        let AgentOwner::Window {
            target,
            generation,
            epoch,
            ..
        } = parent
        else {
            unreachable!()
        };
        context
            .register_activation_parent(ActivationParentInputs {
                page,
                frame: token.clone(),
                generation: 1,
                epoch: 0,
                parent: target.clone(),
                parent_generation: *generation,
                parent_epoch: *epoch,
            })
            .unwrap();
        AgentOwner::Window {
            page,
            target: WindowTarget::Frame(token),
            generation: 1,
            epoch: 0,
        }
    }
    #[test]
    fn notification_reaches_cross_origin_ancestors_and_same_origin_descendants_only() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .commit_page_url(page, "https://top.test/".into())
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let root = AgentOwner::Window {
            page,
            target: WindowTarget::Root,
            generation: 1,
            epoch: 0,
        };
        let source = frame(&context, page, "a", "https://source.test", &root);
        let intermediate = frame(&context, page, "b", "https://foreign.test", &source);
        let descendant = frame(&context, page, "c", "https://source.test", &intermediate);
        let sibling = frame(&context, page, "d", "https://source.test", &root);
        context.record_native_activation(source.clone()).unwrap();
        for owner in [&root, &source, &descendant] {
            assert!(context.transient_native_activation(owner, false).unwrap());
            assert!(context.sticky_native_activation(owner).unwrap());
        }
        for owner in [&intermediate, &sibling] {
            assert!(!context.transient_native_activation(owner, false).unwrap());
            assert!(!context.sticky_native_activation(owner).unwrap());
        }
        assert!(
            context
                .transient_native_activation(&descendant, true)
                .unwrap()
        );
        for owner in [&root, &source, &descendant] {
            assert!(!context.transient_native_activation(owner, false).unwrap());
            assert!(context.sticky_native_activation(owner).unwrap());
        }
        let AgentOwner::Window {
            target: WindowTarget::Frame(source_frame),
            ..
        } = source
        else {
            unreachable!()
        };
        let AgentOwner::Window {
            target: descendant_target,
            ..
        } = descendant
        else {
            unreachable!()
        };
        assert!(
            context
                .register_activation_parent(ActivationParentInputs {
                    page,
                    frame: source_frame,
                    generation: 1,
                    epoch: 0,
                    parent: descendant_target,
                    parent_generation: 1,
                    parent_epoch: 0
                })
                .is_err()
        );
    }
}

#[cfg(all(test, unix))]
mod isolated_worker_admission_tests {
    use super::*;
    use oryn_js::worker_wire::*;
    fn bootstrap(id: u64, kind: AgentKind) -> AgentBootstrap {
        AgentBootstrap {
            cookie_ancestor_urls: Vec::new(),
            import_metadata: Default::default(),
            service_skip_waiting: false,
            resource_digests: Default::default(),
            secure_context: true,
            service_registration: None,
            service_controller: None,
            id: AgentId(id),
            kind,
            partition: "native-partition".into(),
            origin: AgentOrigin {
                serialized: "https://a.test".into(),
                authority: "native-origin-a".into(),
                opaque: false,
            },
            script_url: "https://a.test/worker.js".into(),
            source: Vec::new(),
            modules: Default::default(),
            options: Default::default(),
            referrer_policy: "no-referrer".into(),
            limits: Default::default(),
            top_level_url: Some("https://a.test/".into()),
            policy: AgentPolicy {
                hide_nonces: false,
                policy_sources: Vec::new(),
                policy_self_urls: Vec::new(),
                document_self_url: None,
                policy_self_url: None,
                url: "https://a.test/worker.js".into(),
                headers: vec![("cross-origin-embedder-policy".into(), "require-corp".into())],
            },
        }
    }
    #[test]
    fn dedicated_inherits_but_shared_and_service_have_independent_native_clusters() {
        let context = Browser::new().new_context();
        let parent = AgentOwner::Agent(AgentId(800));
        context
            .inner
            .group
            .shared_memory
            .lock()
            .unwrap()
            .admit_root(parent.clone(), "native-origin-a".into())
            .unwrap();
        for (id, kind) in [
            (801, AgentKind::Dedicated),
            (802, AgentKind::Shared),
            (803, AgentKind::Service),
        ] {
            context
                .admit_worker_shared_memory(&bootstrap(id, kind), Some(&parent))
                .unwrap();
        }
        let mut memory = context.inner.group.shared_memory.lock().unwrap();
        let (descriptor, _) = memory
            .allocate(crate::shared_memory::SharedAllocationInputs {
                owner: &parent,
                byte_length: 4,
                maximum_length: None,
                granularity: 4096,
                guard_length: 0,
                wasm: false,
                memory64: false,
            })
            .unwrap();
        let dedicated = AgentOwner::Agent(AgentId(801));
        let _alias = memory.open(&dedicated, descriptor).unwrap();
        for id in [802, 803] {
            let owner = AgentOwner::Agent(AgentId(id));
            assert!(memory.isolated(&owner));
            assert!(memory.open(&owner, descriptor).is_err());
        }
        memory.release(&parent, descriptor.token).unwrap();
        memory.release(&dedicated, descriptor.token).unwrap();
    }
    #[test]
    fn insecure_denied_or_retired_creator_cannot_supply_dedicated_cluster() {
        let context = Browser::new().new_context();
        let parent = AgentOwner::Agent(AgentId(810));
        context
            .inner
            .group
            .shared_memory
            .lock()
            .unwrap()
            .admit_root(parent.clone(), "native-origin-a".into())
            .unwrap();
        let mut insecure = bootstrap(811, AgentKind::Dedicated);
        insecure.secure_context = false;
        context
            .admit_worker_shared_memory(&insecure, Some(&parent))
            .unwrap();
        let mut denied = bootstrap(812, AgentKind::Dedicated);
        denied.policy.headers.push((
            "permissions-policy".into(),
            "cross-origin-isolated=()".into(),
        ));
        context
            .admit_worker_shared_memory(&denied, Some(&parent))
            .unwrap();
        context.retire_shared_memory(&parent);
        context
            .admit_worker_shared_memory(&bootstrap(813, AgentKind::Dedicated), Some(&parent))
            .unwrap();
        let memory = context.inner.group.shared_memory.lock().unwrap();
        for id in [811, 812, 813] {
            assert!(!memory.isolated(&AgentOwner::Agent(AgentId(id))));
        }
    }
}

#[cfg(all(test, unix))]
mod popup_creator_tests {
    use super::*;
    use oryn_js::message_wire::{MessageRequest, WindowTarget};
    #[test]
    fn cross_origin_frame_popup_inherits_frame_policy_origin_and_opener_target() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        context
            .install_document_security(page.page_id(), "https://top.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let frame = "b".repeat(64);
        let target = WindowTarget::Frame(frame.clone());
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                    origin: "https://child.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        let ticket = context
            .prepare_frame_worker_policy(
                page.page_id(),
                "https://child.test/",
                &[("content-security-policy".into(), "script-src 'none'".into())],
                "",
            )
            .unwrap();
        context
            .bind_frame_worker_policy(
                page.page_id(),
                frame.clone(),
                2,
                0,
                "https://child.test/",
                &ticket,
            )
            .unwrap();
        context
            .register_activation_parent(ActivationParentInputs {
                page: page.page_id(),
                frame: frame.clone(),
                generation: 2,
                epoch: 0,
                parent: WindowTarget::Root,
                parent_generation: 1,
                parent_epoch: 0,
            })
            .unwrap();
        context
            .register_isolation_container(IsolationContainerInputs {
                page: page.page_id(),
                frame: frame.clone(),
                generation: 2,
                epoch: 0,
                allow: String::new(),
                source: "https://child.test/".into(),
                sandbox: None,
            })
            .unwrap();
        let creator = oryn_js::worker_wire::Creator::Window {
            target: target.clone(),
            document_generation: 2,
            stream_epoch: 0,
        };
        let popup = context
            .new_popup_from_creator(page.page_id(), &creator)
            .unwrap();
        assert_eq!(
            context.page_origin(popup.page_id()).unwrap(),
            "https://child.test"
        );
        assert_eq!(
            context.native_opener_target(popup.page_id()).unwrap(),
            target
        );
        let policy = context
            .inner
            .document_security
            .lock()
            .unwrap()
            .get(&popup.page_id())
            .unwrap()
            .clone();
        assert!(!policy.allows_eval(false).allowed);
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RetireWindow {
                    frame,
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .new_popup_from_creator(page.page_id(), &creator)
                .is_err()
        );
    }
    #[test]
    fn immutable_iframe_sandbox_is_part_of_popup_and_worker_policy() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        context
            .install_document_security(page.page_id(), "https://top.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let frame = "c".repeat(64);
        context
            .handle_message_wire(
                page.page_id(),
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                    origin: "null".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        let ticket = context
            .prepare_frame_worker_policy(page.page_id(), "https://child.test/", &[], "")
            .unwrap();
        context
            .bind_frame_worker_policy(
                page.page_id(),
                frame.clone(),
                2,
                0,
                "https://child.test/",
                &ticket,
            )
            .unwrap();
        context
            .register_activation_parent(ActivationParentInputs {
                page: page.page_id(),
                frame: frame.clone(),
                generation: 2,
                epoch: 0,
                parent: WindowTarget::Root,
                parent_generation: 1,
                parent_epoch: 0,
            })
            .unwrap();
        context
            .register_isolation_container(IsolationContainerInputs {
                page: page.page_id(),
                frame: frame.clone(),
                generation: 2,
                epoch: 0,
                allow: String::new(),
                source: "https://child.test/".into(),
                sandbox: Some("allow-scripts".into()),
            })
            .unwrap();
        let policy = context
            .window_worker_policy(page.page_id(), &WindowTarget::Frame(frame.clone()), 2, 0)
            .unwrap();
        let policy = crate::csp::CspPolicySet::from_agent_policy(&policy);
        let flags = policy.sandbox().unwrap();
        assert!(flags.allow_scripts);
        assert!(!flags.allow_popups);
        assert!(!flags.allow_same_origin);
        assert!(
            context
                .register_isolation_container(IsolationContainerInputs {
                    page: page.page_id(),
                    frame,
                    generation: 2,
                    epoch: 0,
                    allow: String::new(),
                    source: "https://child.test/".into(),
                    sandbox: Some("allow-scripts allow-popups".into())
                })
                .is_err()
        );
    }
}

#[cfg(unix)]
impl BrowserContext {
    pub(crate) fn reclaim_context_response(
        &self,
        source: crate::context_service::ResponseSource,
        action: crate::context_service::ResponseCleanup,
    ) {
        self.inner
            .response_reclaims
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .enqueue(source, action);
        let _ = self.retry_context_response_reclaims();
    }
    pub(crate) fn retry_context_response_reclaims(&self) -> Result<(), RuntimeError> {
        self.retry_unparked_resumes()?;
        self.retry_prerender_cancellations()?;
        if let Some(mut batch) =
            crate::context_service::ResponseReclaims::begin(&self.inner.response_reclaims)
        {
            batch.run(|source, action| self.dispose_context_response(source, action).is_ok());
            drop(batch);
            if !self
                .inner
                .response_reclaims
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .ready()
            {
                return Err(RuntimeError::ContextService(
                    "response capability cleanup has not drained".into(),
                ));
            }
        }
        // A teardown callback may re-enter this method. The outer batch keeps
        // custody; only admission, not cleanup recursion, rejects that state.
        Ok(())
    }
    pub(crate) fn admit_context_response(&self) -> Result<(), RuntimeError> {
        self.retry_context_response_reclaims()?;
        if !self
            .inner
            .response_reclaims
            .lock()
            .map_err(|_| RuntimeError::ContextService("response cleanup lock poisoned".into()))?
            .ready()
        {
            return Err(RuntimeError::ContextService(
                "response capability cleanup is pending".into(),
            ));
        }
        Ok(())
    }
    fn dispose_context_response(
        &self,
        source: crate::context_service::ResponseSource,
        action: &crate::context_service::ResponseCleanup,
    ) -> Result<(), WorkerError> {
        use crate::context_service::ResponseCleanup;
        use crate::context_service::ResponseSource;
        match action {
            ResponseCleanup::Message(value) => self
                .inner
                .group
                .messages
                .lock()
                .map_err(|_| worker_lock_error("response message cleanup"))?
                .dispose_queued_value(source.principal(), value.cleanup_clone())
                .map_err(|error| worker_lock_error(&error)),
            ResponseCleanup::FramePolicy(token) => match source {
                ResponseSource::Window(page) => self
                    .discard_prepared_frame_policy(page, token)
                    .map_err(|error| worker_lock_error(&error.to_string())),
                ResponseSource::Agent(_) => {
                    Err(worker_lock_error("agent cannot own frame policy grant"))
                }
            },
            ResponseCleanup::LocalNavigation(token) => match source {
                ResponseSource::Window(page) => self
                    .discard_prepared_local_navigation(page, token)
                    .map_err(|error| worker_lock_error(&error.to_string())),
                ResponseSource::Agent(_) => {
                    Err(worker_lock_error("agent cannot own local navigation grant"))
                }
            },
            ResponseCleanup::Download(id) => match source {
                ResponseSource::Window(_) => {
                    self.discard_download(*id);
                    Ok(())
                }
                ResponseSource::Agent(_) => {
                    Err(worker_lock_error("agent cannot own page download response"))
                }
            },
            ResponseCleanup::BlankPrerender {
                page,
                token,
                generation,
                epoch,
            } => match source {
                ResponseSource::Window(source) => self
                    .cancel_blank_prerender(source, *page, token, *generation, *epoch)
                    .map_err(|error| worker_lock_error(&error.to_string())),
                ResponseSource::Agent(_) => Err(worker_lock_error(
                    "agent cannot own blank prerender response",
                )),
            },
            ResponseCleanup::Prerender {
                token,
                generation,
                epoch,
            } => match source {
                ResponseSource::Window(page) => self
                    .cancel_native_prerender(page, token, (*generation, *epoch))
                    .map_err(|error| worker_lock_error(&error.to_string())),
                ResponseSource::Agent(_) => {
                    Err(worker_lock_error("agent cannot own prerender response"))
                }
            },
            ResponseCleanup::ParkedDocument(token) => match source {
                ResponseSource::Window(page) => self
                    .rollback_unreceived_document(page, token)
                    .map_err(|error| worker_lock_error(&error)),
                ResponseSource::Agent(_) => Err(worker_lock_error(
                    "agent cannot own a cached document response",
                )),
            },
            ResponseCleanup::Broadcast(token) => self
                .inner
                .group
                .messages
                .lock()
                .map_err(|_| worker_lock_error("response broadcast cleanup"))?
                .discard_broadcast(source.principal(), token)
                .map_err(|error| worker_lock_error(&error)),
            ResponseCleanup::Memory { creator, .. } | ResponseCleanup::Wait { creator, .. } => {
                let owner = match (source, creator) {
                    (
                        ResponseSource::Window(page),
                        oryn_js::worker_wire::Creator::Window {
                            target,
                            document_generation,
                            stream_epoch,
                        },
                    ) => AgentOwner::Window {
                        page,
                        target: target.clone(),
                        generation: *document_generation,
                        epoch: *stream_epoch,
                    },
                    (
                        ResponseSource::Agent(source),
                        oryn_js::worker_wire::Creator::Agent(agent),
                    ) if source == *agent => AgentOwner::Agent(source),
                    _ => return Err(worker_lock_error("response cleanup principal mismatch")),
                };
                let mut registry = self
                    .inner
                    .group
                    .shared_memory
                    .lock()
                    .map_err(|_| worker_lock_error("response mapping cleanup"))?;
                match action {
                    ResponseCleanup::Memory { token, .. } => registry
                        .release_response_lease(&owner, *token)
                        .map_err(|error| worker_lock_error(&error)),
                    ResponseCleanup::Wait { id, .. } => registry
                        .cancel_wait_admission(&owner, *id)
                        .map_err(|error| worker_lock_error(&error)),
                    _ => unreachable!(),
                }
            }
            ResponseCleanup::Connection(connection) => {
                self.dispatch_worker_registry(|registry, hub| {
                    let Ok(owner) = registry.connection_owner(connection) else {
                        return Ok(());
                    };
                    let matches = match (&owner, source) {
                        (AgentOwner::Window { page, .. }, ResponseSource::Window(source)) => {
                            *page == source
                        }
                        (AgentOwner::Agent(agent), ResponseSource::Agent(source)) => {
                            *agent == source
                        }
                        _ => false,
                    };
                    if !matches {
                        return Err(worker_lock_error("foreign response worker connection"));
                    }
                    registry.release(hub, &owner, connection)
                })
            }
        }
    }
}

impl BrowserContext {
    pub(crate) fn read_native_window_origin(
        &self,
        source: PageId,
        creator: &oryn_js::worker_wire::Creator,
        target_page: PageId,
        target: &oryn_js::message_wire::WindowTarget,
        expected: Option<crate::browsing_groups::GroupToken>,
    ) -> Result<(String, bool), RuntimeError> {
        use oryn_js::{message_wire::WindowTarget, worker_wire::Creator};
        let denied = || {
            RuntimeError::ContextService("SecurityError: Window origin is not accessible".into())
        };
        let Creator::Window {
            target: source_target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(denied());
        };
        let pages = self.inner.group.pages.lock().map_err(|_| denied())?;
        if ![source, target_page]
            .iter()
            .all(|id| pages.get(id).is_some_and(|record| record.is_live()))
        {
            return Err(denied());
        }
        let groups = self
            .inner
            .group
            .browsing_groups
            .lock()
            .map_err(|_| denied())?;
        if !groups.related(source, target_page, expected) {
            return Err(denied());
        }
        let origins = self.inner.security_origins.lock().map_err(|_| denied())?;
        let live = pages
            .iter()
            .filter(|(_, record)| record.is_live())
            .filter_map(|(id, _)| origins.get(id).map(|origin| (id.0, origin.serialize())))
            .collect();
        let keys = pages
            .iter()
            .filter(|(_, record)| record.is_live())
            .filter_map(|(id, _)| {
                origins
                    .get(id)
                    .map(|origin| (id.0, origin.security_token_key()))
            })
            .collect();
        let policies = self.inner.worker_policies.lock().map_err(|_| denied())?;
        let hub = self.inner.group.messages.lock().map_err(|_| denied())?;
        match source_target {
            WindowTarget::Root => hub
                .root_document_authority(source.0, *document_generation, *stream_epoch)
                .map_err(|_| denied())?,
            WindowTarget::Frame(frame) => {
                hub.frame_document_authority(source.0, frame, *document_generation, *stream_epoch)
                    .map_err(|_| denied())?;
            }
        }
        for (page, target) in [(source, source_target), (target_page, target)] {
            if let WindowTarget::Frame(frame) = target {
                if !policies.frame_origins.iter().any(
                    |((owner, token, generation, epoch), origin)| {
                        *owner == page
                            && token == frame
                            && policies
                                .frame_security_epochs
                                .get(&(page, frame.clone(), *generation, *epoch))
                                .copied()
                                == Some(
                                    policies
                                        .root_lifetimes
                                        .get(&page)
                                        .map_or(0, |lifetime| lifetime.epoch),
                                )
                            && hub
                                .frame_document_authority(page.0, frame, *generation, *epoch)
                                .as_ref()
                                == Ok(origin)
                    },
                ) {
                    return Err(denied());
                }
            } else {
                let (generation, _) = hub.root_document_identity(page.0).ok_or_else(denied)?;
                if !policies
                    .root_lifetimes
                    .get(&page)
                    .is_some_and(|lifetime| lifetime.current(generation))
                {
                    return Err(denied());
                }
            }
        }
        let (_, source_key) = hub
            .window_security_origin(source.0, source_target, &live, &keys)
            .ok_or_else(denied)?;
        let (serialized, target_key) = hub
            .window_security_origin(target_page.0, target, &live, &keys)
            .ok_or_else(denied)?;
        if source_key != target_key {
            return Err(denied());
        }
        let opaque = serialized == "null";
        Ok((serialized, opaque))
    }
}

#[cfg(all(test, unix))]
mod window_origin_authority_tests {
    use super::*;
    use oryn_js::{
        message_wire::{MessageRequest, WindowTarget},
        worker_wire::Creator,
    };
    #[test]
    fn frame_policy_admission_rejects_agent_creator_before_allocating_ticket() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        let creator = Creator::Agent(oryn_js::worker_wire::AgentId(1));
        assert!(matches!(
            context.prepare_frame_worker_policy_for_creator(
                page,
                "https://child.test/",
                &[],
                "",
                Some(&creator)
            ),
            Err(RuntimeError::ProtocolViolation(_))
        ));
        let policies = context.inner.worker_policies.lock().unwrap();
        assert!(policies.prepared.is_empty());
        assert!(policies.prepared_isolation.is_empty());
        assert!(policies.prepared_security_epochs.is_empty());
    }
    #[test]
    fn frame_origin_reads_use_current_realm_identity_and_reject_cross_origin_and_retirement() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(id, "https://top.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        for frame in [&a, &b] {
            context
                .handle_message_wire(
                    id,
                    MessageRequest::RegisterWindow {
                        frame: frame.clone(),
                        document_generation: 2,
                        stream_epoch: 0,
                        origin: "https://child.test".into(),
                        inherit_origin: None,
                    },
                )
                .unwrap();
            let ticket = context
                .prepare_frame_worker_policy(id, "https://child.test/", &[], "")
                .unwrap();
            context
                .bind_frame_worker_policy(id, frame.clone(), 2, 0, "https://child.test/", &ticket)
                .unwrap();
        }
        let source = Creator::Window {
            target: WindowTarget::Frame(a.clone()),
            document_generation: 2,
            stream_epoch: 0,
        };
        assert_eq!(
            context
                .read_native_window_origin(id, &source, id, &WindowTarget::Frame(b.clone()), None)
                .unwrap(),
            ("https://child.test".into(), false)
        );
        assert!(
            context
                .read_native_window_origin(id, &source, id, &WindowTarget::Root, None)
                .is_err()
        );
        let stale = Creator::Window {
            target: WindowTarget::Frame(a.clone()),
            document_generation: 1,
            stream_epoch: 0,
        };
        assert!(
            context
                .read_native_window_origin(id, &stale, id, &WindowTarget::Frame(b.clone()), None)
                .is_err()
        );
        context
            .handle_message_wire(
                id,
                MessageRequest::RetireWindow {
                    frame: b.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .read_native_window_origin(id, &source, id, &WindowTarget::Frame(b), None)
                .is_err()
        );
    }
    #[test]
    fn unrelated_opaque_frames_do_not_share_authority_because_both_serialize_null() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let a = "c".repeat(64);
        let b = "d".repeat(64);
        for frame in [&a, &b] {
            context
                .handle_message_wire(
                    id,
                    MessageRequest::RegisterWindow {
                        frame: frame.clone(),
                        document_generation: 2,
                        stream_epoch: 0,
                        origin: "null".into(),
                        inherit_origin: None,
                    },
                )
                .unwrap();
            let ticket = context
                .prepare_frame_worker_policy(id, "https://child.test/", &[], "")
                .unwrap();
            context
                .bind_frame_worker_policy(id, frame.clone(), 2, 0, "https://child.test/", &ticket)
                .unwrap();
        }
        let source = Creator::Window {
            target: WindowTarget::Frame(a.clone()),
            document_generation: 2,
            stream_epoch: 0,
        };
        assert_eq!(
            context
                .read_native_window_origin(id, &source, id, &WindowTarget::Frame(a), None)
                .unwrap(),
            ("null".into(), true)
        );
        assert!(
            context
                .read_native_window_origin(id, &source, id, &WindowTarget::Frame(b), None)
                .is_err()
        );
    }
}

impl BrowserContext {
    pub(crate) fn disown_native_opener(
        &self,
        page: PageId,
        creator: &oryn_js::worker_wire::Creator,
    ) -> Result<(), RuntimeError> {
        if let oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Frame(token),
            document_generation,
            stream_epoch,
        } = creator
        {
            self.window_worker_creator(
                page,
                &oryn_js::message_wire::WindowTarget::Frame(token.clone()),
                *document_generation,
                *stream_epoch,
            )?;
            self.inner
                .worker_policies
                .lock()
                .map_err(|_| RuntimeError::ContextService("frame opener disown poisoned".into()))?
                .frame_openers
                .remove(&(page, token.clone()));
            return Ok(());
        }

        let oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ContextService(
                "opener disown requires root document authority".into(),
            ));
        };
        let mut pages =
            self.inner.group.pages.lock().map_err(|_| {
                RuntimeError::ContextService("opener page registry poisoned".into())
            })?;
        let record = pages
            .get_mut(&page)
            .filter(|record| record.is_live())
            .ok_or(RuntimeError::PageNotFound(page))?;
        let hub = self.inner.group.messages.lock().map_err(|_| {
            RuntimeError::ContextService("opener document registry poisoned".into())
        })?;
        hub.root_document_authority(page.0, *document_generation, *stream_epoch)
            .map_err(RuntimeError::ContextService)?;
        let mut targets =
            self.inner.group.opener_targets.lock().map_err(|_| {
                RuntimeError::ContextService("opener target registry poisoned".into())
            })?;
        record.descriptor.page.opener_page_id = None;
        targets.remove(&page);
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod opener_disown_tests {
    use super::*;
    use oryn_js::{
        message_wire::{MessageRequest, WindowTarget},
        worker_wire::Creator,
    };
    #[test]
    fn disowning_requires_current_root_generation_and_does_not_change_group() {
        let context = Browser::new().new_context();
        let parent = context.new_page();
        let child = context.new_popup(parent.page_id(), None);
        let id = child.page_id();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 1,
                },
            )
            .unwrap();
        let group = context.native_window_group(id, id, None).unwrap().0;
        for creator in [
            Creator::Agent(oryn_js::worker_wire::AgentId(1)),
            Creator::Window {
                target: WindowTarget::Frame("a".repeat(64)),
                document_generation: 2,
                stream_epoch: 1,
            },
            Creator::Window {
                target: WindowTarget::Root,
                document_generation: 2,
                stream_epoch: 0,
            },
        ] {
            assert!(context.disown_native_opener(id, &creator).is_err());
            assert_eq!(
                context.native_window_group(id, id, None).unwrap().1,
                Some(parent.page_id())
            );
        }
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 2,
            stream_epoch: 1,
        };
        context.disown_native_opener(id, &creator).unwrap();
        context.disown_native_opener(id, &creator).unwrap();
        let current = context.native_window_group(id, id, None).unwrap();
        assert_eq!(current.0, group);
        assert!(current.1.is_none());
    }
}

impl BrowserContext {
    pub(crate) fn native_root_target_name(
        &self,
        page: PageId,
        creator: &oryn_js::worker_wire::Creator,
        value: Option<String>,
    ) -> Result<String, RuntimeError> {
        let oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "root target name requires native root authority".into(),
            ));
        };
        if value.as_ref().is_some_and(|value| value.len() > 65_536) {
            return Err(RuntimeError::ProtocolViolation(
                "target name exceeds native bound".into(),
            ));
        }
        let mut pages =
            self.inner.group.pages.lock().map_err(|_| {
                RuntimeError::ContextService("target name registry poisoned".into())
            })?;
        let record = pages
            .get_mut(&page)
            .filter(|record| record.is_live())
            .ok_or(RuntimeError::PageNotFound(page))?;
        // Match history publication lock order; cached names and the current
        // name must change under the same authenticated document authority.
        let origins = self
            .inner
            .security_origins
            .lock()
            .map_err(|_| RuntimeError::ContextService("target name origins poisoned".into()))?;
        let mut snapshots =
            self.inner.cached_document_security.lock().map_err(|_| {
                RuntimeError::ContextService("target name snapshots poisoned".into())
            })?;
        let hub =
            self.inner.group.messages.lock().map_err(|_| {
                RuntimeError::ContextService("target name authority poisoned".into())
            })?;
        hub.root_document_authority(page.0, *document_generation, *stream_epoch)
            .map_err(RuntimeError::ContextService)?;
        if let Some(value) = value {
            let saved: std::sync::Arc<str> = std::sync::Arc::from(value.as_str());
            if let Some(origin) = origins.get(&page) {
                for snapshot in snapshots.values_mut().filter(|snapshot| {
                    snapshot.page == page
                        && snapshot.frame_target.is_none()
                        && snapshot.origin.security_token_key() == origin.security_token_key()
                }) {
                    snapshot.target_name = Some(saved.clone());
                }
            }
            record.target_name = value;
        }
        Ok(record.target_name.clone())
    }
    #[cfg(test)]
    pub(crate) fn named_related_page(
        &self,
        source: PageId,
        name: &str,
    ) -> Result<Option<PageId>, RuntimeError> {
        let pages = self
            .inner
            .group
            .pages
            .lock()
            .map_err(|_| RuntimeError::ContextService("target registry poisoned".into()))?;
        if !pages.get(&source).is_some_and(|record| record.is_live()) {
            return Err(RuntimeError::PageNotFound(source));
        }
        let groups = self
            .inner
            .group
            .browsing_groups
            .lock()
            .map_err(|_| RuntimeError::ContextService("target groups poisoned".into()))?;
        Ok(pages.iter().find_map(|(id, record)| {
            (record.is_live() && record.target_name == name && groups.related(source, *id, None))
                .then_some(*id)
        }))
    }
}

#[cfg(all(test, unix))]
mod native_target_name_tests {
    use super::*;
    #[test]
    fn equal_serialized_opaque_roots_do_not_share_cached_names() {
        use oryn_js::message_wire::MessageResponse;
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let id = page.page_id();
        let creator = |generation| Creator::Window {
            target: WindowTarget::Root,
            document_generation: generation,
            stream_epoch: 0,
        };
        context
            .install_document_security(id, "data:text/html,same".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        context
            .native_root_target_name(id, &creator(1), Some("first opaque".into()))
            .unwrap();
        let first_origin = context.inner.security_origins.lock().unwrap()[&id].security_token_key();
        let MessageResponse::DocumentParked(first) = context
            .handle_message_wire(
                id,
                MessageRequest::ParkDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap()
        else {
            panic!("first opaque cache token")
        };
        context
            .install_document_security(id, "data:text/html,same".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert_ne!(
            context.inner.security_origins.lock().unwrap()[&id].security_token_key(),
            first_origin
        );
        assert_eq!(
            context
                .native_root_target_name(id, &creator(2), None)
                .unwrap(),
            ""
        );
        context
            .native_root_target_name(id, &creator(2), Some("second opaque".into()))
            .unwrap();
        let MessageResponse::DocumentParked(second) = context
            .handle_message_wire(
                id,
                MessageRequest::ParkDocument {
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap()
        else {
            panic!("second opaque cache token")
        };
        context
            .handle_message_wire(id, MessageRequest::RestoreDocument { token: first })
            .unwrap();
        assert_eq!(
            context
                .native_root_target_name(id, &creator(1), None)
                .unwrap(),
            "first opaque"
        );
        context
            .handle_message_wire(
                id,
                MessageRequest::ParkDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        context
            .handle_message_wire(id, MessageRequest::RestoreDocument { token: second })
            .unwrap();
        assert_eq!(
            context
                .native_root_target_name(id, &creator(2), None)
                .unwrap(),
            "second opaque"
        );
    }
    use oryn_js::{
        message_wire::{MessageRequest, WindowTarget},
        worker_wire::Creator,
    };
    #[test]
    fn root_name_clear_uses_strict_origin_and_preserves_auxiliary_opener_exception() {
        let context = Browser::in_process_probe().new_context();
        let source = context.new_page();
        let id = source.page_id();
        context
            .install_document_security(id, "https://names.test/first".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        context
            .native_root_target_name(id, &creator, Some("main".into()))
            .unwrap();
        let popup = context
            .new_named_popup_from_creator(id, &creator, false, "auxiliary".into())
            .unwrap();
        context
            .install_document_security(id, "https://names.test/second".into(), &[])
            .unwrap();
        assert_eq!(
            context.inner.group.pages.lock().unwrap()[&id].target_name,
            "main"
        );
        context
            .install_document_security(id, "https://other.test/".into(), &[])
            .unwrap();
        assert!(
            context.inner.group.pages.lock().unwrap()[&id]
                .target_name
                .is_empty()
        );
        context
            .install_document_security(popup.page_id(), "https://other.test/".into(), &[])
            .unwrap();
        assert_eq!(
            context.inner.group.pages.lock().unwrap()[&popup.page_id()].target_name,
            "auxiliary"
        );
        context
            .install_document_security(
                popup.page_id(),
                "https://isolated.test/".into(),
                &[("cross-origin-opener-policy".into(), "same-origin".into())],
            )
            .unwrap();
        let pages = context.inner.group.pages.lock().unwrap();
        let record = &pages[&popup.page_id()];
        assert!(record.descriptor.page.opener_page_id.is_none());
        assert!(record.target_name.is_empty());
    }

    #[test]
    fn cached_root_names_follow_same_origin_updates_and_restore_foreign_groups() {
        use oryn_js::message_wire::MessageResponse;
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let id = page.page_id();
        let creator = |generation| Creator::Window {
            target: WindowTarget::Root,
            document_generation: generation,
            stream_epoch: 0,
        };
        let install = |generation, url: &str, name: &str| {
            context
                .install_document_security(id, url.into(), &[])
                .unwrap();
            context
                .handle_message_wire(
                    id,
                    MessageRequest::RegisterRootDocument {
                        document_generation: generation,
                        stream_epoch: 0,
                    },
                )
                .unwrap();
            context
                .native_root_target_name(id, &creator(generation), Some(name.into()))
                .unwrap();
        };
        let park = |generation| {
            let MessageResponse::DocumentParked(token) = context
                .handle_message_wire(
                    id,
                    MessageRequest::ParkDocument {
                        document_generation: generation,
                        stream_epoch: 0,
                    },
                )
                .unwrap()
            else {
                panic!("cached name capability")
            };
            token
        };
        install(1, "https://names.test/first", "original");
        let first = park(1);
        install(2, "https://names.test/second", "latest");
        let _second = park(2);
        install(3, "https://foreign.test/", "foreign");
        assert!(
            context
                .native_root_target_name(id, &creator(1), Some("stale".into()))
                .is_err()
        );
        let foreign = park(3);
        context
            .handle_message_wire(id, MessageRequest::RestoreDocument { token: first })
            .unwrap();
        assert_eq!(
            context
                .native_root_target_name(id, &creator(1), None)
                .unwrap(),
            "latest"
        );
        assert!(
            context
                .native_root_target_name(id, &creator(1), Some("x".repeat(65_537)))
                .is_err()
        );
        assert_eq!(
            context
                .native_root_target_name(id, &creator(1), None)
                .unwrap(),
            "latest"
        );
        let _first_again = park(1);
        context
            .handle_message_wire(id, MessageRequest::RestoreDocument { token: foreign })
            .unwrap();
        assert_eq!(
            context
                .native_root_target_name(id, &creator(3), None)
                .unwrap(),
            "foreign"
        );
    }

    #[test]
    fn native_names_are_group_scoped_and_stale_document_cannot_rename() {
        let context = Browser::new().new_context();
        let source = context.new_page();
        let other = context.new_page();
        let id = source.page_id();
        context
            .install_document_security(id, "https://forms.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let popup = context
            .new_named_popup_from_creator(id, &creator, false, "receipt".into())
            .unwrap();
        assert_eq!(
            context.named_related_page(id, "receipt").unwrap(),
            Some(popup.page_id())
        );
        assert_eq!(
            context
                .named_related_page(other.page_id(), "receipt")
                .unwrap(),
            None
        );
        assert_eq!(
            context
                .native_root_target_name(id, &creator, Some("source".into()))
                .unwrap(),
            "source"
        );
        assert_eq!(context.named_related_page(id, "source").unwrap(), Some(id));
        context
            .handle_message_wire(
                id,
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .native_root_target_name(id, &creator, Some("forged".into()))
                .is_err()
        );
        assert_eq!(context.named_related_page(id, "source").unwrap(), Some(id));
    }
}

impl BrowserContext {
    pub(crate) fn root_navigation_creator(
        &self,
        page: PageId,
    ) -> Result<oryn_js::worker_wire::Creator, RuntimeError> {
        let hub = self.inner.group.messages.lock().map_err(|_| {
            RuntimeError::ContextService("navigation source registry poisoned".into())
        })?;
        let (document_generation, stream_epoch) = hub
            .root_document_identity(page.0)
            .ok_or(RuntimeError::PageNotFound(page))?;
        Ok(oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation,
            stream_epoch,
        })
    }
    /// HTML allowed-by-sandboxing-to-navigate. Native document identities and
    /// committed ancestry supply the inputs; authored target strings do not.
    pub(crate) fn native_target_navigation_allowed(
        &self,
        source_page: PageId,
        source: &oryn_js::worker_wire::Creator,
        target_page: PageId,
        target: &oryn_js::worker_wire::Creator,
    ) -> Result<bool, RuntimeError> {
        use oryn_js::{message_wire::WindowTarget, worker_wire::Creator};
        let Creator::Window {
            target: source_target,
            document_generation: sg,
            stream_epoch: se,
        } = source
        else {
            return Err(RuntimeError::ProtocolViolation(
                "navigation source is not a Window".into(),
            ));
        };
        let Creator::Window {
            target: target_target,
            document_generation: tg,
            stream_epoch: te,
        } = target
        else {
            return Err(RuntimeError::ProtocolViolation(
                "navigation target is not a Window".into(),
            ));
        };
        let (source_owner, _, _source_origin) =
            self.window_worker_creator(source_page, source_target, *sg, *se)?;
        let (target_owner, _, _target_origin) =
            self.window_worker_creator(target_page, target_target, *tg, *te)?;
        {
            let groups = self.inner.group.browsing_groups.lock().map_err(|_| {
                RuntimeError::ContextService("navigation group registry poisoned".into())
            })?;
            if !groups.related(source_page, target_page, None) {
                return Ok(false);
            }
        }
        let policy = self.window_worker_policy(source_page, source_target, *sg, *se)?;
        let policy = crate::csp::CspPolicySet::from_agent_policy(&policy);
        let (source_ancestor, target_ancestor) = {
            let policies =
                self.inner.worker_policies.lock().map_err(|_| {
                    RuntimeError::ContextService("navigation ancestry poisoned".into())
                })?;
            (
                activation_descendant(&policies.activation_parents, &target_owner, &source_owner),
                activation_descendant(&policies.activation_parents, &source_owner, &target_owner),
            )
        };
        let allowed = if source_owner == target_owner || source_ancestor {
            true
        } else if target_ancestor {
            if !matches!(target_target, WindowTarget::Root) {
                true
            } else if let Some(flags) = policy.sandbox() {
                let active = self
                    .transient_native_activation(&source_owner, false)
                    .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
                flags.allow_top_navigation
                    || (active && flags.allow_top_navigation_by_user_activation)
            } else {
                self.native_unsandboxed_top_navigation_allowed(&source_owner, &target_owner)?
            }
        } else {
            let permitted = matches!(target_target, WindowTarget::Root)
                && self
                    .inner
                    .group
                    .pages
                    .lock()
                    .map_err(|_| {
                        RuntimeError::ContextService("navigation target registry poisoned".into())
                    })?
                    .get(&target_page)
                    .is_some_and(|record| {
                        record.sandbox_navigator.as_ref()
                            == Some(&(source_page, source_target.clone()))
                    });
            permitted || policy.sandbox().is_none()
        };
        self.window_worker_creator(source_page, source_target, *sg, *se)?;
        self.window_worker_creator(target_page, target_target, *tg, *te)?;
        Ok(allowed)
    }
    fn native_unsandboxed_top_navigation_allowed(
        &self,
        source: &AgentOwner,
        target: &AgentOwner,
    ) -> Result<bool, RuntimeError> {
        // A cross-origin descendant must have its own sticky activation.
        // Top-page activation alone does not authorize frame-busting.
        if self
            .native_document_domain_identity(source)
            .map_err(RuntimeError::ContextService)?
            .same_origin_domain(
                &self
                    .native_document_domain_identity(target)
                    .map_err(RuntimeError::ContextService)?,
            )
        {
            return Ok(true);
        }
        self.sticky_native_activation(source)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))
    }
    /// Named familiarity uses same-origin-domain access plus the live opener
    /// chain; it never changes the strict principal used to load the target.
    fn named_familiar_page_from_domain(
        &self,
        page: PageId,
        identity: &oryn_js::origin::DomainAccessIdentity,
        name: &str,
    ) -> Result<Option<PageId>, RuntimeError> {
        let candidates = {
            let pages = self.inner.group.pages.lock().map_err(|_| {
                RuntimeError::ContextService("named navigation registry poisoned".into())
            })?;
            let groups = self.inner.group.browsing_groups.lock().map_err(|_| {
                RuntimeError::ContextService("named navigation groups poisoned".into())
            })?;
            let mut result = Vec::new();
            for (id, _) in pages
                .iter()
                .filter(|(_, record)| record.is_live() && record.target_name == name)
            {
                if !groups.related(page, *id, None) {
                    continue;
                }
                let mut current = Some(*id);
                let mut seen = BTreeSet::new();
                let mut chain = Vec::new();
                while let Some(candidate) = current.filter(|candidate| seen.insert(*candidate)) {
                    chain.push(candidate);
                    current = pages
                        .get(&candidate)
                        .filter(|record| record.is_live())
                        .and_then(|record| record.descriptor.page.opener_page_id);
                }
                result.push((*id, chain));
            }
            result
        };
        for (id, chain) in candidates {
            for candidate in chain {
                if candidate == page {
                    return Ok(Some(id));
                }
                let target_identity = if let Ok(oryn_js::worker_wire::Creator::Window {
                    target,
                    document_generation,
                    stream_epoch,
                }) = self.root_navigation_creator(candidate)
                {
                    let (owner, _, _) = self.window_worker_creator(
                        candidate,
                        &target,
                        document_generation,
                        stream_epoch,
                    )?;
                    self.native_document_domain_identity(&owner)
                        .map_err(RuntimeError::ContextService)?
                } else {
                    oryn_js::origin::DomainAccessIdentity::new(self.security_origin(candidate)?)
                };
                if identity.same_origin_domain(&target_identity) {
                    return Ok(Some(id));
                }
            }
        }
        Ok(None)
    }
    fn named_frame_familiar_from_domain(
        &self,
        identity: &oryn_js::origin::DomainAccessIdentity,
        owner: &AgentOwner,
    ) -> Result<bool, RuntimeError> {
        let mut current = owner.clone();
        let mut seen = BTreeSet::new();
        while seen.insert(current.clone()) {
            if identity.same_origin_domain(
                &self
                    .native_document_domain_identity(&current)
                    .map_err(RuntimeError::ContextService)?,
            ) {
                return Ok(true);
            }
            let parent = self
                .inner
                .worker_policies
                .lock()
                .map_err(|_| RuntimeError::ContextService("named frame ancestry poisoned".into()))?
                .activation_parents
                .get(&current)
                .cloned();
            let Some(parent) = parent else { break };
            current = parent;
        }
        Ok(false)
    }
}

impl BrowserContext {
    #[cfg(test)]
    pub(crate) fn native_top_target_navigation_allowed(
        &self,
        page: PageId,
        creator: &oryn_js::worker_wire::Creator,
        target_page: PageId,
    ) -> Result<bool, RuntimeError> {
        if page == target_page {
            let target = self.root_navigation_creator(target_page)?;
            return self.native_target_navigation_allowed(page, creator, target_page, &target);
        }
        use oryn_js::worker_wire::Creator;
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "navigation requires a Window".into(),
            ));
        };
        self.window_worker_creator(page, target, *document_generation, *stream_epoch)?;
        let policy =
            self.window_worker_policy(page, target, *document_generation, *stream_epoch)?;
        let policy = crate::csp::CspPolicySet::from_agent_policy(&policy);
        let permitted =
            {
                let pages = self.inner.group.pages.lock().map_err(|_| {
                    RuntimeError::ContextService("navigation pages poisoned".into())
                })?;
                let record = pages
                    .get(&target_page)
                    .filter(|record| record.is_live())
                    .ok_or(RuntimeError::PageNotFound(target_page))?;
                let groups = self.inner.group.browsing_groups.lock().map_err(|_| {
                    RuntimeError::ContextService("navigation groups poisoned".into())
                })?;
                if !groups.related(page, target_page, None) {
                    return Ok(false);
                }
                record.sandbox_navigator.as_ref() == Some(&(page, target.clone()))
            };
        let allowed = permitted || policy.sandbox().is_none();
        self.window_worker_creator(page, target, *document_generation, *stream_epoch)?;
        Ok(allowed)
    }
}

#[cfg(all(test, unix))]
mod native_navigation_permission_tests {
    use super::*;
    use oryn_js::{
        message_wire::{MessageRequest, WindowTarget},
        worker_wire::Creator,
    };
    fn frame(
        context: &BrowserContext,
        page: PageId,
        label: &str,
        parent: &Creator,
        policy: &str,
    ) -> Creator {
        frame_at(context, page, label, parent, policy, "https://child.test/")
    }
    fn frame_at(
        context: &BrowserContext,
        page: PageId,
        label: &str,
        parent: &Creator,
        policy: &str,
        url: &str,
    ) -> Creator {
        let frame = label.repeat(64);
        let origin = url::Url::parse(url).unwrap().origin().ascii_serialization();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 1,
                    stream_epoch: 0,
                    origin,
                    inherit_origin: None,
                },
            )
            .unwrap();
        let ticket = context
            .prepare_frame_worker_policy(
                page,
                url,
                &[("content-security-policy".into(), policy.into())],
                "",
            )
            .unwrap();
        context
            .bind_frame_worker_policy(page, frame.clone(), 1, 0, url, &ticket)
            .unwrap();
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = parent
        else {
            unreachable!()
        };
        context
            .register_activation_parent(ActivationParentInputs {
                page,
                frame: frame.clone(),
                generation: 1,
                epoch: 0,
                parent: target.clone(),
                parent_generation: *document_generation,
                parent_epoch: *stream_epoch,
            })
            .unwrap();
        context
            .register_isolation_container(IsolationContainerInputs {
                page,
                frame: frame.clone(),
                generation: 1,
                epoch: 0,
                allow: String::new(),
                source: url.into(),
                sandbox: None,
            })
            .unwrap();
        Creator::Window {
            target: WindowTarget::Frame(frame),
            document_generation: 1,
            stream_epoch: 0,
        }
    }
    #[test]
    fn cross_origin_top_navigation_requires_source_sticky_activation() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://top.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let root = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let child = frame(&context, page, "d", &root, "");
        let nested = frame(&context, page, "e", &child, "");
        let same_origin = frame_at(&context, page, "f", &root, "", "https://top.test/child");
        assert!(
            context
                .handle_message_wire(
                    page,
                    MessageRequest::RegisterWindow {
                        frame: "g".repeat(64),
                        document_generation: 1,
                        stream_epoch: 0,
                        origin: "https://child.test".into(),
                        inherit_origin: None,
                    }
                )
                .is_err(),
            "malformed capability cannot obtain a navigation policy"
        );
        let explicit = frame(
            &context,
            page,
            "c",
            &root,
            "sandbox allow-scripts allow-same-origin allow-top-navigation",
        );
        let allowed = |source: &Creator| {
            let direct = context
                .native_target_navigation_allowed(page, source, page, &root)
                .unwrap();
            assert_eq!(
                direct,
                context
                    .native_top_target_navigation_allowed(page, source, page)
                    .unwrap()
            );
            direct
        };
        assert!(allowed(&root));
        assert!(allowed(&same_origin));
        assert!(allowed(&explicit));
        assert!(!allowed(&child));
        assert!(!allowed(&nested));
        let root_owner = context
            .window_worker_creator(page, &WindowTarget::Root, 1, 0)
            .unwrap()
            .0;
        context.record_native_activation(root_owner).unwrap();
        assert!(
            !allowed(&child),
            "activation in the top document is insufficient"
        );
        assert!(
            !allowed(&nested),
            "same-origin immediate parent does not make the top same-origin"
        );
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = &nested
        else {
            unreachable!()
        };
        let owner = context
            .window_worker_creator(page, target, *document_generation, *stream_epoch)
            .unwrap()
            .0;
        context.record_native_activation(owner.clone()).unwrap();
        assert!(allowed(&nested));
        assert!(context.transient_native_activation(&owner, true).unwrap());
        assert!(!context.transient_native_activation(&owner, false).unwrap());
        assert!(
            allowed(&nested),
            "unsandboxed top navigation uses sticky activation"
        );
    }
    #[test]
    fn popup_relationship_policy_uses_native_coop_and_blob_origin() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://top.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let root = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let child = frame(&context, page, "f", &root, "");
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = &child
        else {
            unreachable!()
        };
        let owner = context
            .window_worker_creator(page, target, *document_generation, *stream_epoch)
            .unwrap()
            .0;
        let blob = context
            .register_native_blob(owner, b"<body>receipt".to_vec(), "text/html".into())
            .unwrap();
        assert_eq!(
            context
                .native_popup_relationship_policy(page, &root, Some(&blob), false)
                .unwrap(),
            (true, false)
        );
        assert_eq!(
            context
                .native_popup_relationship_policy(
                    page,
                    &root,
                    Some("blob:https://top.test/not-registered"),
                    false
                )
                .unwrap(),
            (false, false)
        );
        context
            .install_document_security(
                page,
                "https://top.test/".into(),
                &[("cross-origin-opener-policy".into(), "same-origin".into())],
            )
            .unwrap();
        assert!(
            context
                .native_popup_relationship_policy(page, &child, None, false)
                .is_err(),
            "republishing root security retires the old child's policy authority"
        );
        assert!(
            context
                .native_popup_relationship_policy(page, &root, None, false)
                .is_err(),
            "republishing security also retires the original root authority"
        );
        let (_, _, _, generation, _) = context
            .native_document_authority(page, "https://top.test/")
            .unwrap();
        assert!(generation > 1);
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: generation,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let root = Creator::Window {
            target: WindowTarget::Root,
            document_generation: generation,
            stream_epoch: 0,
        };
        let child = frame(&context, page, "e", &root, "");
        assert_eq!(
            context
                .native_popup_relationship_policy(page, &child, None, false)
                .unwrap(),
            (true, true)
        );
        assert_eq!(
            context
                .native_popup_relationship_policy(page, &root, None, false)
                .unwrap(),
            (false, false)
        );
        let popup = context
            .new_named_popup_from_creator(page, &child, false, "named".into())
            .unwrap();
        let pages = context.inner.group.pages.lock().unwrap();
        let record = pages.get(&popup.page_id()).unwrap();
        assert_eq!(record.descriptor.page.opener_page_id, None);
        assert!(record.target_name.is_empty());
    }

    #[test]
    fn sandbox_navigation_uses_native_ancestry_activation_and_permitted_auxiliary() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://top.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let root = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let middle = frame(
            &context,
            page,
            "a",
            &root,
            "sandbox allow-scripts allow-same-origin allow-forms allow-popups allow-top-navigation-by-user-activation",
        );
        let source = frame(
            &context,
            page,
            "b",
            &middle,
            "sandbox allow-scripts allow-same-origin allow-forms allow-popups allow-top-navigation-by-user-activation",
        );
        let sibling = frame(&context, page, "c", &root, "");
        assert!(
            context
                .native_target_navigation_allowed(page, &source, page, &source)
                .unwrap()
        );
        assert!(
            context
                .native_target_navigation_allowed(page, &middle, page, &source)
                .unwrap()
        );
        assert!(
            context
                .native_target_navigation_allowed(page, &source, page, &middle)
                .unwrap(),
            "non-top ancestor is permitted"
        );
        assert!(
            !context
                .native_target_navigation_allowed(page, &source, page, &root)
                .unwrap()
        );
        assert!(
            !context
                .native_target_navigation_allowed(page, &source, page, &sibling)
                .unwrap()
        );
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = &source
        else {
            unreachable!()
        };
        let owner = context
            .window_worker_creator(page, target, *document_generation, *stream_epoch)
            .unwrap()
            .0;
        context.record_native_activation(owner).unwrap();
        assert!(
            context
                .native_target_navigation_allowed(page, &source, page, &root)
                .unwrap()
        );
        let popup = context.new_popup_from_creator(page, &source).unwrap();
        assert!(
            context
                .native_top_target_navigation_allowed(page, &source, popup.page_id())
                .unwrap()
        );
        assert!(
            !context
                .native_top_target_navigation_allowed(page, &middle, popup.page_id())
                .unwrap(),
            "another sandboxed navigable is not the permitted navigator"
        );
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireWindow {
                    frame: if let WindowTarget::Frame(frame) = target {
                        frame.clone()
                    } else {
                        unreachable!()
                    },
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .native_target_navigation_allowed(page, &source, page, &root)
                .is_err()
        );
    }
}

#[cfg(unix)]
impl BrowserContext {
    #[cfg(all(test, feature = "v8-host", feature = "in-process-probe"))]
    pub(crate) fn named_familiar_navigation_target(
        &self,
        page: PageId,
        creator: &oryn_js::worker_wire::Creator,
        name: &str,
    ) -> Result<
        Option<(
            PageId,
            Option<crate::host::webdriver_script::AutomationRealmSnapshot>,
        )>,
        RuntimeError,
    > {
        self.named_familiar_navigation_target_with_local(page, creator, name, &BTreeMap::new())
    }
    pub(crate) fn named_familiar_navigation_target_with_local(
        &self,
        page: PageId,
        creator: &oryn_js::worker_wire::Creator,
        name: &str,
        local: &BTreeMap<
            PageId,
            Vec<(
                String,
                crate::host::webdriver_script::AutomationRealmSnapshot,
            )>,
        >,
    ) -> Result<
        Option<(
            PageId,
            Option<crate::host::webdriver_script::AutomationRealmSnapshot>,
        )>,
        RuntimeError,
    > {
        self.named_familiar_navigation_target_from_source(page, creator, None, name, local)
    }
    pub(crate) fn named_familiar_document_navigation_target(
        &self,
        page: PageId,
        source: &oryn_dom::DocumentProvenance,
        name: &str,
        local: &BTreeMap<
            PageId,
            Vec<(
                String,
                crate::host::webdriver_script::AutomationRealmSnapshot,
            )>,
        >,
    ) -> Result<
        Option<(
            PageId,
            Option<crate::host::webdriver_script::AutomationRealmSnapshot>,
        )>,
        RuntimeError,
    > {
        let (creator, _, _) = self.document_popup_source(page, source)?;
        self.named_familiar_navigation_target_from_source(page, &creator, Some(source), name, local)
    }
    fn named_familiar_navigation_target_from_source(
        &self,
        page: PageId,
        creator: &oryn_js::worker_wire::Creator,
        source: Option<&oryn_dom::DocumentProvenance>,
        name: &str,
        local: &BTreeMap<
            PageId,
            Vec<(
                String,
                crate::host::webdriver_script::AutomationRealmSnapshot,
            )>,
        >,
    ) -> Result<
        Option<(
            PageId,
            Option<crate::host::webdriver_script::AutomationRealmSnapshot>,
        )>,
        RuntimeError,
    > {
        if local.len() > 256 || local.values().map(Vec::len).sum::<usize>() > 4096 {
            return Err(RuntimeError::ProtocolViolation(
                "local navigation inventory capacity".into(),
            ));
        }
        use crate::host::webdriver_script::{FrameCommand, FrameResponse};
        use oryn_js::{message_wire::WindowTarget, worker_wire::Creator};
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "named navigation requires native Window".into(),
            ));
        };
        if source.is_none()
            && ["_self", "_parent", "_top"]
                .iter()
                .any(|keyword| name.eq_ignore_ascii_case(keyword))
        {
            return self.keyword_navigation_target(page, creator, name, local);
        }
        let source_identity = if let Some(source) = source {
            // Authenticated capture retains the domain of the source Document,
            // including inert/retired sources, without borrowing a successor.
            self.document_source_domain_identity(page, source)?
        } else {
            let (owner, _, _) =
                self.window_worker_creator(page, target, *document_generation, *stream_epoch)?;
            self.native_document_domain_identity(&owner)
                .map_err(RuntimeError::ContextService)?
        };
        if let Some(selected) =
            self.preferred_local_named_target(page, creator, source, &source_identity, name, local)?
        {
            return Ok(Some(selected));
        }
        let top_match = self.named_familiar_page_from_domain(page, &source_identity, name)?;
        let candidates =
            {
                let pages = self.inner.group.pages.lock().map_err(|_| {
                    RuntimeError::ContextService("named target pages poisoned".into())
                })?;
                let groups = self.inner.group.browsing_groups.lock().map_err(|_| {
                    RuntimeError::ContextService("named target groups poisoned".into())
                })?;
                pages
                    .iter()
                    .filter(|(id, record)| record.is_live() && groups.related(page, **id, None))
                    .map(|(id, record)| (*id, record.handle.clone()))
                    .collect::<Vec<_>>()
            };
        let executor = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        for (id, handle) in candidates {
            if id == page && local.contains_key(&page) {
                continue;
            }
            if top_match == Some(id) {
                return Ok(Some((id, None)));
            }
            // The native source actor supplies its local forest before entering
            // this RPC. Never synchronously request those actors from here.
            let inventory = if let Some(inventory) = local.get(&id) {
                inventory.clone()
            } else {
                if id == page {
                    continue;
                }
                if self
                    .inner
                    .group
                    .messages
                    .lock()
                    .map_err(|_| {
                        RuntimeError::ContextService(
                            "named target document registry poisoned".into(),
                        )
                    })?
                    .root_document_identity(id.0)
                    .is_none()
                {
                    continue;
                }
                let remaining =
                    deadline
                        .checked_duration_since(Instant::now())
                        .ok_or_else(|| {
                            RuntimeError::ContextService(
                                "native named target inventory deadline exceeded".into(),
                            )
                        })?;
                let response = executor
                    .block_on(async {
                        tokio::time::timeout(
                            remaining,
                            handle.webdriver_frames(FrameCommand::NamedInventory),
                        )
                        .await
                    })
                    .map_err(|_| {
                        RuntimeError::ContextService(
                            "native named target inventory deadline exceeded".into(),
                        )
                    })?
                    .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
                let FrameResponse::NamedInventory(inventory) = response else {
                    return Err(RuntimeError::ProtocolViolation(
                        "invalid native named target inventory".into(),
                    ));
                };
                inventory
            };
            for (target_name, snapshot) in inventory
                .into_iter()
                .filter(|(_, snapshot)| snapshot.live && snapshot.parent_realm.is_some())
            {
                if target_name != name {
                    continue;
                }
                let Some(frame) = snapshot.frame_token.clone() else {
                    continue;
                };
                let target = WindowTarget::Frame(frame);
                let (owner, _, _) = self.window_worker_creator(
                    id,
                    &target,
                    snapshot.document_generation,
                    snapshot.document_authority_epoch,
                )?;
                let familiar = self.named_frame_familiar_from_domain(&source_identity, &owner)?;
                let target_creator = Creator::Window {
                    target,
                    document_generation: snapshot.document_generation,
                    stream_epoch: snapshot.document_authority_epoch,
                };
                let allowed = if let Some(source) = source {
                    self.document_target_navigation_allowed(id, source, &target_creator)?
                } else {
                    self.native_target_navigation_allowed(page, creator, id, &target_creator)?
                };
                if familiar && allowed {
                    if source.is_none() {
                        self.window_worker_creator(
                            page,
                            match creator {
                                Creator::Window { target, .. } => target,
                                _ => unreachable!(),
                            },
                            *document_generation,
                            *stream_epoch,
                        )?;
                    }
                    return Ok(Some((id, Some(snapshot))));
                }
            }
        }
        Ok(None)
    }
}

impl BrowserContext {
    pub(crate) fn native_popup_relationship_policy(
        &self,
        page: PageId,
        creator: &oryn_js::worker_wire::Creator,
        url: Option<&str>,
        requested_noopener: bool,
    ) -> Result<(bool, bool), RuntimeError> {
        let oryn_js::worker_wire::Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "popup requires native Window".into(),
            ));
        };
        let (_, _, origin) =
            self.window_worker_creator(page, target, *document_generation, *stream_epoch)?;
        let top = self.security_origin(page)?;
        let force_blank = !matches!(target, oryn_js::message_wire::WindowTarget::Root)
            && self.page_isolation_policy(page)?.same_origin_opener
            && origin.authority != top.security_token_key();
        let mut noopener = requested_noopener || force_blank;
        if let Some(url) = url.filter(|url| url.starts_with("blob:")) {
            let blob = self
                .inner
                .worker_policies
                .lock()
                .map_err(|_| {
                    RuntimeError::ContextService("blob relationship registry poisoned".into())
                })?
                .blobs
                .get(url)
                .cloned();
            if let Some(blob) = blob {
                self.retained_worker_authority(&blob.owner)
                    .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
                let same_site = if blob.origin.authority == top.security_token_key() {
                    true
                } else if top.is_opaque() || blob.origin.serialized == "null" {
                    false
                } else {
                    url::Url::parse(&blob.origin.serialized)
                        .ok()
                        .and_then(|url| crate::cookies::SchemefulSite::from_url(&url))
                        .zip(
                            url::Url::parse(&top.serialize())
                                .ok()
                                .and_then(|url| crate::cookies::SchemefulSite::from_url(&url)),
                        )
                        .is_some_and(|(blob, top)| blob == top)
                };
                noopener |= !same_site;
            }
        }
        self.window_worker_creator(page, target, *document_generation, *stream_epoch)?;
        Ok((noopener, force_blank))
    }
}

#[cfg(all(test, unix))]
mod native_geolocation_policy_tests {
    use super::*;
    use oryn_js::{
        message_wire::{MessageRequest, WindowTarget},
        platform_control::{PermissionDescriptor, PermissionState, Request, Response},
        worker_wire::Creator,
    };
    fn root(header: &str) -> (BrowserContext, PageId, AgentOwner) {
        let context = Browser::in_process_with_config(Default::default()).new_context_with_options(
            ContextOptions {
                media_devices: oryn_js::media_capture::DeviceConfiguration::None,
                ..Default::default()
            },
        );
        let page = context.new_page();
        let id = page.page_id();
        context
            .commit_page_url(id, "https://top.test/".into())
            .unwrap();
        let headers = if header.is_empty() {
            vec![]
        } else {
            vec![("Permissions-Policy".into(), header.into())]
        };
        context
            .install_document_security(id, "https://top.test/".into(), &headers)
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let owner = context
            .window_worker_creator(id, &WindowTarget::Root, 1, 0)
            .unwrap()
            .0;
        (context, id, owner)
    }
    fn frame(
        context: &BrowserContext,
        page: PageId,
        parent: &AgentOwner,
        name: &str,
        origin: &str,
        allow: &str,
    ) -> AgentOwner {
        let encoded = name
            .bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert!(encoded.len() <= 64);
        let token = format!("{encoded:0<64}");
        let url = format!("{origin}/frame");
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterWindow {
                    frame: token.clone(),
                    document_generation: 1,
                    stream_epoch: 0,
                    origin: origin.into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        let policy = context
            .prepare_frame_worker_policy(page, &url, &[], "<body></body>")
            .unwrap();
        context
            .bind_frame_worker_policy(page, token.clone(), 1, 0, &url, &policy)
            .unwrap();
        let AgentOwner::Window {
            target,
            generation,
            epoch,
            ..
        } = parent
        else {
            unreachable!()
        };
        context
            .register_activation_parent(ActivationParentInputs {
                page,
                frame: token.clone(),
                generation: 1,
                epoch: 0,
                parent: target.clone(),
                parent_generation: *generation,
                parent_epoch: *epoch,
            })
            .unwrap();
        context
            .register_isolation_container(IsolationContainerInputs {
                page,
                frame: token.clone(),
                generation: 1,
                epoch: 0,
                allow: allow.into(),
                source: url,
                sandbox: None,
            })
            .unwrap();
        let owner = AgentOwner::Window {
            page,
            target: WindowTarget::Frame(token),
            generation: 1,
            epoch: 0,
        };
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        owner
    }
    #[test]
    fn live_container_updates_authenticate_parent_and_preserve_sandbox() {
        let (context, page, root) = root("");
        context
            .publish_worker_secure_context(root.clone(), true)
            .unwrap();
        let child = frame(
            &context,
            page,
            &root,
            "live-policy",
            "https://child.test",
            "",
        );
        let AgentOwner::Window {
            target,
            generation,
            epoch,
            ..
        } = &child
        else {
            unreachable!()
        };
        let creator = Creator::Window {
            target: target.clone(),
            document_generation: *generation,
            stream_epoch: *epoch,
        };
        let parent = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        assert!(
            !context
                .native_platform_allowed(&child, "geolocation")
                .unwrap()
        );
        context
            .update_live_frame_container_policy(
                page,
                &creator,
                &parent,
                "geolocation *".into(),
                "https://child.test/frame".into(),
            )
            .unwrap();
        assert!(
            context
                .native_platform_allowed(&child, "geolocation")
                .unwrap()
        );
        let sandbox = context
            .inner
            .worker_policies
            .lock()
            .unwrap()
            .isolation_containers[&child]
            .2
            .clone();
        context
            .update_live_frame_container_policy(
                page,
                &creator,
                &parent,
                "geolocation 'none'".into(),
                "https://child.test/next".into(),
            )
            .unwrap();
        assert!(
            !context
                .native_platform_allowed(&child, "geolocation")
                .unwrap()
        );
        assert_eq!(
            context
                .inner
                .worker_policies
                .lock()
                .unwrap()
                .isolation_containers[&child]
                .2,
            sandbox
        );
        assert!(
            context
                .update_live_frame_container_policy(
                    page,
                    &creator,
                    &creator,
                    "geolocation *".into(),
                    "https://child.test/frame".into()
                )
                .is_err()
        );
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireWindow {
                    frame: match target {
                        WindowTarget::Frame(frame) => frame.clone(),
                        _ => unreachable!(),
                    },
                    document_generation: *generation,
                    stream_epoch: *epoch,
                },
            )
            .unwrap();
        assert!(
            context
                .update_live_frame_container_policy(
                    page,
                    &creator,
                    &parent,
                    "geolocation *".into(),
                    "https://child.test/frame".into()
                )
                .is_err()
        );
    }
    #[test]
    fn worker_cookie_ancestry_is_native_and_rejects_retired_frame_identity() {
        let (context, page, root) = root("");
        let cross = frame(&context, page, &root, "c", "https://cross.test", "");
        let nested = frame(&context, page, &cross, "n", "https://top.test", "");
        let urls = context.worker_cookie_ancestor_urls(&nested).unwrap();
        assert!(
            urls.iter()
                .any(|url| url.starts_with("https://cross.test/"))
        );
        assert!(urls.iter().any(|url| url.starts_with("https://top.test/")));
        let AgentOwner::Window {
            target: WindowTarget::Frame(token),
            generation,
            epoch,
            ..
        } = nested.clone()
        else {
            unreachable!()
        };
        let forged = AgentOwner::Window {
            page,
            target: WindowTarget::Frame(token.clone()),
            generation: generation + 1,
            epoch,
        };
        assert!(context.worker_cookie_ancestor_urls(&forged).is_err());
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireWindow {
                    frame: token,
                    document_generation: generation,
                    stream_epoch: epoch,
                },
            )
            .unwrap();
        assert!(context.worker_cookie_ancestor_urls(&nested).is_err());
    }
    #[test]
    fn production_query_cannot_turn_a_grant_into_insecure_or_policy_disabled_location() {
        let (context, page, owner) = root("geolocation=()");
        let descriptor = PermissionDescriptor {
            name: "geolocation".into(),
            sysex: None,
            pan_tilt_zoom: None,
            allow_without_gesture: None,
        };
        context
            .webdriver_set_permission(
                "https://top.test",
                descriptor.clone(),
                PermissionState::Granted,
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        assert!(matches!(
            context
                .native_platform_query(
                    page,
                    creator.clone(),
                    Request::Permission {
                        descriptor: descriptor.clone()
                    }
                )
                .unwrap(),
            Response::Permission {
                state: PermissionState::Denied,
                ..
            }
        ));
        context.publish_worker_secure_context(owner, true).unwrap();
        assert!(matches!(
            context
                .native_platform_query(page, creator, Request::Geolocation)
                .unwrap(),
            Response::Geolocation {
                position: Err(oryn_js::platform_control::PositionError { code: 1, .. }),
                ..
            }
        ));
    }
    #[test]
    fn native_policy_snapshot_queries_current_identity_and_live_declarations() {
        let (context, page, root) = root("");
        context
            .publish_worker_secure_context(root.clone(), true)
            .unwrap();
        let child = frame(
            &context,
            page,
            &root,
            "policy-api",
            "https://child.test",
            "",
        );
        let AgentOwner::Window {
            target,
            generation,
            epoch,
            ..
        } = &child
        else {
            unreachable!()
        };
        let creator = Creator::Window {
            target: target.clone(),
            document_generation: *generation,
            stream_epoch: *epoch,
        };
        let parent = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let Response::PermissionsPolicySnapshot(snapshot) = context
            .native_platform_query(page, creator.clone(), Request::PermissionsPolicySnapshot)
            .unwrap()
        else {
            panic!("policy snapshot")
        };
        assert!(snapshot.allows("picture-in-picture", None));
        assert!(!snapshot.allows("geolocation", None));
        context
            .update_live_frame_container_policy(
                page,
                &creator,
                &parent,
                "geolocation *; picture-in-picture 'none'".into(),
                "https://child.test/frame".into(),
            )
            .unwrap();
        let snapshot = context.native_permissions_policy_snapshot(&child).unwrap();
        assert!(snapshot.allows("geolocation", None));
        assert!(!snapshot.allows("picture-in-picture", None));
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireWindow {
                    frame: match target {
                        WindowTarget::Frame(frame) => frame.clone(),
                        _ => unreachable!(),
                    },
                    document_generation: *generation,
                    stream_epoch: *epoch,
                },
            )
            .unwrap();
        assert!(
            context
                .native_platform_query(page, creator, Request::PermissionsPolicySnapshot)
                .is_err()
        );
    }
    #[test]
    fn default_self_explicit_delegation_and_ancestor_denial_use_committed_native_chain() {
        let (context, page, owner) = root("");
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        assert!(
            context
                .native_platform_allowed(&owner, "geolocation")
                .unwrap()
        );
        let denied = frame(&context, page, &owner, "a", "https://child.test", "");
        assert!(
            !context
                .native_platform_allowed(&denied, "geolocation")
                .unwrap()
        );
        let permitted = frame(
            &context,
            page,
            &owner,
            "b",
            "https://child.test",
            "geolocation",
        );
        assert!(
            context
                .native_platform_allowed(&permitted, "geolocation")
                .unwrap()
        );
        let descendant = frame(
            &context,
            page,
            &denied,
            "c",
            "https://child.test",
            "geolocation *",
        );
        assert!(
            !context
                .native_platform_allowed(&descendant, "geolocation")
                .unwrap(),
            "descendant cannot restore an ancestor denied feature"
        );
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .native_platform_allowed(&permitted, "geolocation")
                .is_err()
        );
    }
    #[test]
    fn bluetooth_get_devices_enforces_native_ancestor_policy_and_reports_denial() {
        let (context, page, owner) = root("bluetooth=()");
        context.publish_worker_secure_context(owner, true).unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let response = context
            .native_platform_query(page, creator.clone(), Request::BluetoothDevices)
            .unwrap();
        assert!(
            matches!(response,Response::Bluetooth{allowed:false,reports,..}if reports==vec!["enforce"])
        );
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .native_platform_query(page, creator, Request::BluetoothDevices)
                .is_err()
        );
    }
    #[test]
    fn camera_prompt_and_install_eligibility_use_exact_native_features_and_committed_policy() {
        use oryn_js::platform_control::{PromptChoice, PromptKind};
        let (context, page, owner) = root("");
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        assert_eq!(context.native_install_invalid_reason(&owner).unwrap(), "");
        let child = frame(
            &context,
            page,
            &owner,
            "camera-only",
            "https://top.test",
            "camera",
        );
        assert!(
            context
                .native_capture_features_delegated(&child, &["camera"])
                .unwrap()
        );
        assert!(!context.native_media_element_delegated(&child).unwrap());
        assert_eq!(
            context.native_install_invalid_reason(&child).unwrap(),
            "illegal_subframe"
        );
        context.record_native_activation(owner.clone()).unwrap();
        let Response::Prompt(prompt) = context
            .native_platform_query(
                page,
                creator.clone(),
                Request::PromptBegin {
                    kind: PromptKind::Camera,
                },
            )
            .unwrap()
        else {
            panic!("camera prompt")
        };
        context.record_native_activation(owner.clone()).unwrap();
        context
            .native_platform_query(
                page,
                creator,
                Request::PromptAnswer {
                    id: prompt.id,
                    choice: PromptChoice::Allow,
                },
            )
            .unwrap();
        let response = context
            .inner
            .platform_control
            .lock()
            .unwrap()
            .query(
                page,
                "https://top.test",
                Request::Permission {
                    descriptor: PermissionDescriptor {
                        name: "microphone".into(),
                        sysex: None,
                        pan_tilt_zoom: None,
                        allow_without_gesture: None,
                    },
                },
            )
            .unwrap();
        assert!(
            matches!(
                response,
                Response::Permission {
                    state: PermissionState::Prompt,
                    ..
                }
            ),
            "camera choice cannot grant microphone"
        );
        let sandboxed = Browser::in_process_with_config(Default::default()).new_context();
        let sandbox_page = sandboxed.new_page().page_id();
        sandboxed
            .commit_page_url(sandbox_page, "https://top.test/".into())
            .unwrap();
        sandboxed
            .install_document_security(
                sandbox_page,
                "https://top.test/".into(),
                &[(
                    "Content-Security-Policy".into(),
                    "sandbox allow-scripts allow-same-origin".into(),
                )],
            )
            .unwrap();
        sandboxed
            .handle_message_wire(
                sandbox_page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let sandbox_owner = sandboxed
            .window_worker_creator(sandbox_page, &WindowTarget::Root, 1, 0)
            .unwrap()
            .0;
        assert_eq!(
            sandboxed
                .native_install_invalid_reason(&sandbox_owner)
                .unwrap(),
            "illegal_sandbox"
        );
    }
    #[test]
    fn production_permission_prompt_requires_distinct_native_open_and_answer_activation() {
        use oryn_js::platform_control::{PromptChoice, PromptKind};
        let (context, page, owner) = root("");
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        assert!(
            context
                .native_platform_query(
                    page,
                    creator.clone(),
                    Request::PromptBegin {
                        kind: PromptKind::UserMedia
                    }
                )
                .is_err()
        );
        context.record_native_activation(owner.clone()).unwrap();
        let Response::Prompt(prompt) = context
            .native_platform_query(
                page,
                creator.clone(),
                Request::PromptBegin {
                    kind: PromptKind::UserMedia,
                },
            )
            .unwrap()
        else {
            panic!("prompt")
        };
        assert!(
            context
                .native_platform_query(
                    page,
                    creator.clone(),
                    Request::PromptAnswer {
                        id: prompt.id.clone(),
                        choice: PromptChoice::Allow
                    }
                )
                .is_err(),
            "opening activation is consumed"
        );
        context.record_native_activation(owner).unwrap();
        let Response::Prompt(result) = context
            .native_platform_query(
                page,
                creator.clone(),
                Request::PromptAnswer {
                    id: prompt.id.clone(),
                    choice: PromptChoice::Allow,
                },
            )
            .unwrap()
        else {
            panic!("answer")
        };
        assert_eq!(result.state, PermissionState::Granted);
        assert!(
            context
                .native_platform_query(
                    page,
                    creator,
                    Request::PromptAnswer {
                        id: prompt.id,
                        choice: PromptChoice::Allow
                    }
                )
                .is_err()
        );
    }
    #[test]
    fn native_capture_requires_grants_secure_authority_activation_and_explicit_frame_delegation() {
        let (context, page, owner) = root("");
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let capture = || Request::Media {
            request: oryn_js::media_capture::Request::Capture {
                constraints: serde_json::json!({}),
            },
        };
        assert!(
            context
                .native_platform_query(page, creator.clone(), capture())
                .unwrap_err()
                .starts_with("NotAllowedError")
        );
        for name in ["camera", "microphone"] {
            context
                .webdriver_set_permission(
                    "https://top.test",
                    PermissionDescriptor {
                        name: name.into(),
                        sysex: None,
                        pan_tilt_zoom: None,
                        allow_without_gesture: None,
                    },
                    PermissionState::Granted,
                )
                .unwrap();
        }
        assert!(
            context
                .native_platform_query(page, creator.clone(), capture())
                .unwrap_err()
                .starts_with("InvalidStateError")
        );
        context.record_native_activation(owner.clone()).unwrap();
        assert!(
            context
                .native_platform_query(page, creator.clone(), capture())
                .unwrap_err()
                .starts_with("NotFoundError"),
            "grants and activation cannot invent fixture devices"
        );
        let child = frame(&context, page, &owner, "e", "https://top.test", "");
        assert!(
            !context.native_media_element_delegated(&child).unwrap(),
            "same-origin media element still needs explicit delegation"
        );
        let child = frame(
            &context,
            page,
            &owner,
            "f",
            "https://top.test",
            "camera; microphone",
        );
        assert!(context.native_media_element_delegated(&child).unwrap());
        assert!(
            context
                .publish_worker_secure_context(owner.clone(), false)
                .is_err(),
            "secure-context publication is immutable"
        );
        let (insecure, insecure_page, insecure_owner) = root("");
        insecure
            .publish_worker_secure_context(insecure_owner.clone(), false)
            .unwrap();
        for name in ["camera", "microphone"] {
            insecure
                .webdriver_set_permission(
                    "https://top.test",
                    PermissionDescriptor {
                        name: name.into(),
                        sysex: None,
                        pan_tilt_zoom: None,
                        allow_without_gesture: None,
                    },
                    PermissionState::Granted,
                )
                .unwrap();
        }
        insecure.record_native_activation(insecure_owner).unwrap();
        assert!(
            insecure
                .native_platform_query(insecure_page, creator.clone(), capture())
                .unwrap_err()
                .starts_with("NotAllowedError")
        );
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .native_platform_query(page, creator, capture())
                .is_err()
        );
    }
    #[test]
    fn insecure_committed_ancestor_disables_a_trustworthy_child() {
        let (context, page, owner) = root("");
        context
            .publish_worker_secure_context(owner.clone(), false)
            .unwrap();
        let child = frame(
            &context,
            page,
            &owner,
            "d",
            "https://child.test",
            "geolocation *",
        );
        assert!(
            !context
                .native_platform_allowed(&child, "geolocation")
                .unwrap()
        );
        assert!(
            !context
                .native_platform_allowed(&owner, "geolocation")
                .unwrap()
        );
    }
}

#[cfg(all(test, unix))]
mod native_root_publication_tests {
    use super::*;
    use oryn_js::message_wire::{MessageRequest, WindowTarget};
    #[test]
    fn root_generation_cannot_acquire_replacement_policy_even_at_same_origin() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        let url = "https://app.test/";
        context
            .install_document_security(page, url.into(), &[])
            .unwrap();
        let (_, _, _, first, _) = context.native_document_authority(page, url).unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: first,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .window_worker_creator(page, &WindowTarget::Root, first, 0)
                .is_ok()
        );
        let frame = "d".repeat(64);
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 1,
                    stream_epoch: 0,
                    origin: "https://app.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        let ticket = context
            .prepare_frame_worker_policy(page, url, &[], "")
            .unwrap();
        context
            .bind_frame_worker_policy(page, frame.clone(), 1, 0, url, &ticket)
            .unwrap();
        context
            .install_document_security(
                page,
                url.into(),
                &[(
                    "content-security-policy".into(),
                    "connect-src 'none'".into(),
                )],
            )
            .unwrap();
        assert!(
            context
                .window_worker_creator(page, &WindowTarget::Root, first, 0)
                .is_err()
        );
        assert!(
            context
                .window_worker_creator(page, &WindowTarget::Frame(frame), 1, 0)
                .is_err()
        );
        assert!(
            context
                .prepared_frame_resource_policy(page, &ticket)
                .is_err()
        );
        assert!(
            context
                .handle_message_wire(
                    page,
                    MessageRequest::RegisterRootDocument {
                        document_generation: first,
                        stream_epoch: 1
                    }
                )
                .is_err()
        );
        assert!(
            context
                .handle_message_wire(
                    page,
                    MessageRequest::InDocument {
                        target: WindowTarget::Root,
                        document_generation: first,
                        stream_epoch: 0,
                        request: Box::new(MessageRequest::CreateChannel)
                    }
                )
                .is_err()
        );
        let (_, _, _, second, _) = context.native_document_authority(page, url).unwrap();
        assert!(second > first);
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: second,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .window_worker_creator(page, &WindowTarget::Root, second, 0)
                .is_ok()
        );
    }
    #[test]
    fn failed_security_publication_does_not_partially_switch_coop_group() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://old.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let original = context
            .inner
            .group
            .browsing_groups
            .lock()
            .unwrap()
            .token(page)
            .unwrap();
        context
            .inner
            .worker_policies
            .lock()
            .unwrap()
            .root_lifetimes
            .get_mut(&page)
            .unwrap()
            .high_epoch = u64::MAX;
        assert!(
            context
                .install_document_security(
                    page,
                    "https://new.test/".into(),
                    &[("cross-origin-opener-policy".into(), "same-origin".into())]
                )
                .is_err()
        );
        assert_eq!(
            context
                .inner
                .group
                .browsing_groups
                .lock()
                .unwrap()
                .token(page),
            Some(original)
        );
        assert_eq!(
            context
                .window_worker_creator(page, &WindowTarget::Root, 1, 0)
                .unwrap()
                .2
                .serialized,
            "https://old.test"
        );
        assert_eq!(
            context
                .window_worker_policy(page, &WindowTarget::Root, 1, 0)
                .unwrap()
                .url,
            "https://old.test/"
        );
    }
    #[test]
    fn native_security_commitments_recycle_retired_epochs_without_reviving_generations() {
        let mut lifetime = RootDocumentLifetime {
            initial_materialized: true,
            ..Default::default()
        };
        lifetime.publish().unwrap();
        let cached = lifetime.reserve().unwrap();
        let cached_epoch = lifetime.epoch;
        for _ in 0..8192 {
            lifetime.publish().unwrap();
            lifetime.retain_cached_epochs(&std::collections::BTreeSet::from([cached_epoch]));
            let current = lifetime.reserve().unwrap();
            assert!(lifetime.current(current));
            assert!(lifetime.commitments.len() <= 2);
        }
        let latest = lifetime.epoch;
        lifetime.epoch = cached_epoch;
        assert!(lifetime.current(cached));
        lifetime.epoch = latest;
        lifetime.retain_cached_epochs(&Default::default());
        assert!(!lifetime.commitments.contains_key(&cached));
        assert!(lifetime.admit(cached).is_err());
    }

    #[test]
    fn security_publication_counter_survives_history_rollback_and_initial_precommit() {
        let mut lifetime = RootDocumentLifetime::default();
        lifetime.admit(1).unwrap();
        lifetime.publish().unwrap();
        assert!(lifetime.current(1));
        lifetime.admit(2).unwrap();
        assert!(lifetime.current(2)); // document.open
        let original = lifetime.epoch;
        lifetime.publish().unwrap();
        let replacement = lifetime.reserve().unwrap();
        assert!(lifetime.current(replacement));
        lifetime.epoch = original;
        assert!(lifetime.current(1));
        assert!(!lifetime.current(replacement));
        lifetime.publish().unwrap();
        assert!(!lifetime.current(replacement));
        assert!(lifetime.admit(replacement).is_err());
        let next = lifetime.reserve().unwrap();
        assert!(next > replacement);
        assert!(lifetime.current(next));
    }
}

#[cfg(test)]
mod realm_metadata_regressions {
    use super::*;
    use oryn_js::message_wire::{MessageRequest, WindowTarget};
    #[test]
    fn inherited_frame_meta_self_binds_native_origin_not_about_url() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(id, "https://top.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let frame = "a".repeat(64);
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                    origin: "https://top.test".into(),
                    inherit_origin: Some(WindowTarget::Root),
                },
            )
            .unwrap();
        context
            .inherit_frame_worker_policy_at_url(InheritedFramePolicyInputs {
                page: id,
                frame: frame.clone(),
                generation: 2,
                epoch: 0,
                parent: &WindowTarget::Root,
                parent_generation: 1,
                parent_epoch: 0,
                document_url: "about:srcdoc",
            })
            .unwrap();
        let owner = AgentOwner::Window {
            page: id,
            target: WindowTarget::Frame(frame.clone()),
            generation: 2,
            epoch: 0,
        };
        context
            .install_realm_metadata(&owner, Some("img-src 'self'"), None)
            .unwrap();
        let policy = context
            .window_worker_policy(id, &WindowTarget::Frame(frame), 2, 0)
            .unwrap();
        assert_eq!(policy.url, "about:srcdoc");
        let policy = crate::csp::CspPolicySet::from_agent_policy(&policy);
        let request = |url: &str| crate::csp::CspRequest {
            destination: crate::csp::CspDestination::Image,
            url: url.into(),
            nonce: None,
            inline_source: None,
            parser_inserted: true,
            initiated_by_trusted_script: false,
        };
        assert!(policy.authorize(&request("https://top.test/p.png")).allowed);
        assert!(
            !policy
                .authorize(&request("https://foreign.test/p.png"))
                .allowed
        );
    }
    #[test]
    fn frame_meta_restricts_only_exact_committed_frame_and_retirement_denies_mutation() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(id, "https://top.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let frame = "d".repeat(64);
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                    origin: "https://child.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        let ticket = context
            .prepare_frame_worker_policy(id, "https://child.test/", &[], "")
            .unwrap();
        context
            .bind_frame_worker_policy(id, frame.clone(), 2, 0, "https://child.test/", &ticket)
            .unwrap();
        context
            .register_activation_parent(ActivationParentInputs {
                page: id,
                frame: frame.clone(),
                generation: 2,
                epoch: 0,
                parent: WindowTarget::Root,
                parent_generation: 1,
                parent_epoch: 0,
            })
            .unwrap();
        let owner = AgentOwner::Window {
            page: id,
            target: WindowTarget::Frame(frame.clone()),
            generation: 2,
            epoch: 0,
        };
        let sibling = "e".repeat(64);
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterWindow {
                    frame: sibling.clone(),
                    document_generation: 3,
                    stream_epoch: 0,
                    origin: "https://sibling.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        let ticket = context
            .prepare_frame_worker_policy(id, "https://sibling.test/", &[], "")
            .unwrap();
        context
            .bind_frame_worker_policy(id, sibling.clone(), 3, 0, "https://sibling.test/", &ticket)
            .unwrap();
        context
            .install_realm_metadata(&owner, Some("script-src 'none'"), Some("no-referrer"))
            .unwrap();
        context
            .install_realm_metadata(&owner, Some("script-src *"), None)
            .unwrap();
        let policy = context
            .window_worker_policy(id, &WindowTarget::Frame(frame.clone()), 2, 0)
            .unwrap();
        assert!(
            policy
                .headers
                .iter()
                .any(|(name, value)| name == "referrer-policy" && value == "no-referrer")
        );
        assert!(
            !crate::csp::CspPolicySet::from_agent_policy(&policy)
                .allows_eval(false)
                .allowed
        );
        assert!(
            context
                .inner
                .document_security
                .lock()
                .unwrap()
                .get(&id)
                .unwrap()
                .allows_eval(false)
                .allowed
        );
        assert_ne!(
            context.page_referrer_policy(id).unwrap().as_str(),
            "no-referrer"
        );
        let sibling_policy = context
            .window_worker_policy(id, &WindowTarget::Frame(sibling), 3, 0)
            .unwrap();
        assert!(
            crate::csp::CspPolicySet::from_agent_policy(&sibling_policy)
                .allows_eval(false)
                .allowed
        );
        context
            .handle_message_wire(
                id,
                MessageRequest::RetireWindow {
                    frame,
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .install_realm_metadata(&owner, Some("default-src *"), Some("unsafe-url"))
                .is_err()
        );
        let old_root = AgentOwner::Window {
            page: id,
            target: WindowTarget::Root,
            generation: 1,
            epoch: 0,
        };
        context
            .install_document_security(id, "https://new.test/".into(), &[])
            .unwrap();
        assert!(
            context
                .install_realm_metadata(&old_root, Some("script-src 'none'"), Some("no-referrer"))
                .is_err()
        );
        assert!(
            context
                .inner
                .document_security
                .lock()
                .unwrap()
                .get(&id)
                .unwrap()
                .allows_eval(false)
                .allowed
        );
    }
}

impl BrowserContext {
    pub(crate) fn publish_native_window_inventory(
        &self,
        source: PageId,
        creator: &oryn_js::worker_wire::Creator,
        local: &BTreeMap<
            PageId,
            Vec<(
                String,
                crate::host::webdriver_script::AutomationRealmSnapshot,
            )>,
        >,
    ) -> Result<(), RuntimeError> {
        use oryn_js::{message_wire::WindowTarget, worker_wire::Creator};
        if local.len() > 256
            || local.values().map(Vec::len).sum::<usize>() > 4096
            || local
                .values()
                .flatten()
                .fold(0usize, |bytes, (name, entry)| {
                    bytes
                        .saturating_add(name.len())
                        .saturating_add(entry.url.len())
                        .saturating_add(entry.frame_token.as_ref().map_or(0, String::len))
                })
                > 1024 * 1024
        {
            return Err(RuntimeError::ProtocolViolation(
                "native Window tree publication capacity".into(),
            ));
        }
        self.read_related_window(
            source,
            creator,
            source,
            &match creator {
                Creator::Window { target, .. } => target.clone(),
                _ => {
                    return Err(RuntimeError::ProtocolViolation(
                        "Window tree publisher is not a document".into(),
                    ));
                }
            },
            None,
        )?;
        let mut prepared = Vec::new();
        for (page, inventory) in local {
            if inventory.is_empty() {
                continue;
            }
            // Logical close precedes renderer and message-endpoint retirement.
            // A retained forest must not be republished during that interval.
            if !self.native_window_group(source, *page, None)?.2
                || self.script_close_state(source, creator, *page, None)?
            {
                return Err(RuntimeError::ProtocolViolation(
                    "Window tree publication crosses group".into(),
                ));
            }
            let roots = inventory
                .iter()
                .filter(|(_, entry)| entry.live && entry.parent_realm.is_none())
                .collect::<Vec<_>>();
            if roots.len() != 1 {
                return Err(RuntimeError::ProtocolViolation(
                    "Window tree publication root identity".into(),
                ));
            }
            let root = roots[0].1.clone();
            let root_creator = Creator::Window {
                target: WindowTarget::Root,
                document_generation: root.document_generation,
                stream_epoch: root.document_authority_epoch,
            };
            self.window_worker_creator(
                *page,
                &WindowTarget::Root,
                root.document_generation,
                root.document_authority_epoch,
            )?;
            let mut realms = std::collections::BTreeSet::new();
            let mut tokens = std::collections::BTreeSet::new();
            for (_, entry) in inventory {
                if !entry.live || !realms.insert(entry.realm) {
                    return Err(RuntimeError::ProtocolViolation(
                        "Window tree publication duplicate/retired realm".into(),
                    ));
                }
                let target = match &entry.frame_token {
                    Some(token) if entry.parent_realm.is_some() && tokens.insert(token.clone()) => {
                        WindowTarget::Frame(token.clone())
                    }
                    None if entry.parent_realm.is_none() => WindowTarget::Root,
                    _ => {
                        return Err(RuntimeError::ProtocolViolation(
                            "Window tree publication incomplete frame".into(),
                        ));
                    }
                };
                let (owner, _, _) = self.window_worker_creator(
                    *page,
                    &target,
                    entry.document_generation,
                    entry.document_authority_epoch,
                )?;
                if let Some(parent_realm) = entry.parent_realm {
                    let parent = inventory
                        .iter()
                        .find(|(_, candidate)| candidate.live && candidate.realm == parent_realm)
                        .map(|(_, candidate)| candidate)
                        .ok_or_else(|| {
                            RuntimeError::ProtocolViolation(
                                "Window tree publication missing parent".into(),
                            )
                        })?;
                    let parent_target = parent
                        .frame_token
                        .as_ref()
                        .map(|token| WindowTarget::Frame(token.clone()))
                        .unwrap_or(WindowTarget::Root);
                    let parent_owner = AgentOwner::Window {
                        page: *page,
                        target: parent_target,
                        generation: parent.document_generation,
                        epoch: parent.document_authority_epoch,
                    };
                    if self
                        .inner
                        .worker_policies
                        .lock()
                        .map_err(|_| {
                            RuntimeError::ContextService(
                                "Window tree publication ancestry lock".into(),
                            )
                        })?
                        .activation_parents
                        .get(&owner)
                        != Some(&parent_owner)
                    {
                        return Err(RuntimeError::ProtocolViolation(
                            "Window tree publication wrong parent".into(),
                        ));
                    }
                }
            }
            prepared.push((*page, root_creator, inventory.clone()));
        }
        let mut policies = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| RuntimeError::ContextService("Window tree publication lock".into()))?;
        if policies.window_inventories.len().saturating_add(
            prepared
                .iter()
                .filter(|(page, _, _)| !policies.window_inventories.contains_key(page))
                .count(),
        ) > 4096
        {
            return Err(RuntimeError::ProtocolViolation(
                "Window tree publication page capacity".into(),
            ));
        }
        for (page, creator, inventory) in prepared {
            policies
                .window_inventories
                .insert(page, (creator, inventory));
        }
        Ok(())
    }

    pub(crate) fn read_related_window_with_inventory(
        &self,
        source: PageId,
        creator: &oryn_js::worker_wire::Creator,
        target_page: PageId,
        target: &oryn_js::message_wire::WindowTarget,
        expected: Option<crate::browsing_groups::GroupToken>,
        local: &BTreeMap<
            PageId,
            Vec<(
                String,
                crate::host::webdriver_script::AutomationRealmSnapshot,
            )>,
        >,
    ) -> Result<crate::context_service::RelatedWindowSnapshot, RuntimeError> {
        use oryn_js::{message_wire::WindowTarget, worker_wire::Creator};
        if local.len() > 256
            || local.values().map(Vec::len).sum::<usize>() > 4096
            || local
                .values()
                .flatten()
                .fold(0usize, |bytes, (name, entry)| {
                    bytes
                        .saturating_add(name.len())
                        .saturating_add(entry.url.len())
                        .saturating_add(entry.frame_token.as_ref().map_or(0, String::len))
                })
                > 1024 * 1024
        {
            return Err(RuntimeError::ProtocolViolation(
                "remote Window inventory capacity".into(),
            ));
        }
        let mut snapshot =
            self.read_related_window(source, creator, target_page, target, expected)?;
        // An empty local entry is not authority to erase another actor's saved
        // tree. Instead, check the native traversable before using either tree.
        if self.script_close_state(source, creator, target_page, expected)? {
            return Err(RuntimeError::ContextService(
                "remote Window tree target is closing".into(),
            ));
        }
        self.publish_native_window_inventory(source, creator, local)?;
        let inventory = if let Some(inventory) = local.get(&target_page) {
            inventory.clone()
        } else {
            let saved = self
                .inner
                .worker_policies
                .lock()
                .map_err(|_| RuntimeError::ContextService("remote Window inventory lock".into()))?
                .window_inventories
                .get(&target_page)
                .cloned();
            if let Some((root, inventory)) = saved {
                let current_root = self.read_related_window(
                    source,
                    creator,
                    target_page,
                    &WindowTarget::Root,
                    expected,
                )?;
                if root != current_root.creator {
                    return Err(RuntimeError::ContextService(
                        "remote Window tree publication belongs to previous document".into(),
                    ));
                }
                inventory
            } else if snapshot.length == 0 {
                return Ok(snapshot);
            } else {
                return Err(RuntimeError::ContextService(
                    "remote Window tree publication pending".into(),
                ));
            }
        };
        let parent = inventory
            .iter()
            .find(|(_, entry)| {
                entry.live
                    && match target {
                        WindowTarget::Root => entry.parent_realm.is_none(),
                        WindowTarget::Frame(token) => entry.frame_token.as_ref() == Some(token),
                    }
            })
            .map(|(_, entry)| entry)
            .ok_or_else(|| {
                RuntimeError::ContextService("remote Window target retired during inventory".into())
            })?;
        let parent_creator = Creator::Window {
            target: target.clone(),
            document_generation: parent.document_generation,
            stream_epoch: parent.document_authority_epoch,
        };
        if parent_creator != snapshot.creator {
            return Err(RuntimeError::ContextService(
                "remote Window target replaced during inventory".into(),
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut bytes = 0usize;
        for (name, entry) in inventory
            .iter()
            .filter(|(_, entry)| entry.live && entry.parent_realm == Some(parent.realm))
        {
            let token = entry.frame_token.clone().ok_or_else(|| {
                RuntimeError::ProtocolViolation("remote child has no native identity".into())
            })?;
            if !seen.insert(token.clone()) {
                return Err(RuntimeError::ProtocolViolation(
                    "duplicate remote child identity".into(),
                ));
            }
            bytes = bytes.saturating_add(name.len()).saturating_add(token.len());
            if bytes > 1024 * 1024 {
                return Err(RuntimeError::ProtocolViolation(
                    "remote child name capacity".into(),
                ));
            }
            let child = WindowTarget::Frame(token);
            let (child_owner, _, _) = self.window_worker_creator(
                target_page,
                &child,
                entry.document_generation,
                entry.document_authority_epoch,
            )?;
            let Creator::Window {
                document_generation,
                stream_epoch,
                ..
            } = &snapshot.creator
            else {
                unreachable!()
            };
            let parent_owner = AgentOwner::Window {
                page: target_page,
                target: target.clone(),
                generation: *document_generation,
                epoch: *stream_epoch,
            };
            if self
                .inner
                .worker_policies
                .lock()
                .map_err(|_| RuntimeError::ContextService("remote child ancestry lock".into()))?
                .activation_parents
                .get(&child_owner)
                != Some(&parent_owner)
            {
                return Err(RuntimeError::ContextService(
                    "remote child parent changed during inventory".into(),
                ));
            }
            snapshot.children.push((name.clone(), child));
        }
        let current = self.read_related_window(source, creator, target_page, target, expected)?;
        if current.creator != snapshot.creator
            || current.length != snapshot.children.len()
            || self.script_close_state(source, creator, target_page, expected)?
        {
            return Err(RuntimeError::ContextService(
                "remote child inventory changed during read".into(),
            ));
        }
        snapshot.length = snapshot.children.len();
        snapshot.same_origin_domain = current.same_origin_domain;
        snapshot.url = current.url;
        Ok(snapshot)
    }

    pub(crate) fn read_related_window(
        &self,
        source: PageId,
        creator: &oryn_js::worker_wire::Creator,
        target_page: PageId,
        target: &oryn_js::message_wire::WindowTarget,
        expected: Option<crate::browsing_groups::GroupToken>,
    ) -> Result<crate::context_service::RelatedWindowSnapshot, RuntimeError> {
        use oryn_js::{message_wire::WindowTarget, worker_wire::Creator};
        let denied = || {
            RuntimeError::ContextService("SecurityError: related Window authority is stale".into())
        };
        let Creator::Window {
            target: source_target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(denied());
        };
        let source_owner = AgentOwner::Window {
            page: source,
            target: source_target.clone(),
            generation: *document_generation,
            epoch: *stream_epoch,
        };
        let (_, _, _) = self
            .native_worker_authority(&source_owner)
            .map_err(|_| denied())?;
        if !self.native_window_group(source, target_page, expected)?.2 {
            return Err(denied());
        }
        if matches!(target, WindowTarget::Root)
            && self
                .inner
                .group
                .messages
                .lock()
                .map_err(|_| denied())?
                .root_document_identity(target_page.0)
                .is_none()
        {
            self.native_initial_document_creator(target_page)?;
        }
        let documents = self
            .inner
            .group
            .messages
            .lock()
            .map_err(|_| denied())?
            .registered_documents();
        let (_, _, generation, epoch) = documents
            .iter()
            .find(|(page, candidate, _, _)| *page == target_page.0 && candidate == target)
            .ok_or_else(denied)?;
        let owner = AgentOwner::Window {
            page: target_page,
            target: target.clone(),
            generation: *generation,
            epoch: *epoch,
        };
        let (_, _, policy) = self.native_worker_authority(&owner).map_err(|_| denied())?;
        let parents = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| denied())?
            .activation_parents
            .clone();
        let parent = match target {
            WindowTarget::Root => WindowTarget::Root,
            WindowTarget::Frame(_) => match parents.get(&owner) {
                Some(AgentOwner::Window { page, target, .. }) if *page == target_page => {
                    target.clone()
                }
                _ => return Err(denied()),
            },
        };
        let length = documents
            .iter()
            .filter(|(page, target, generation, epoch)| {
                let candidate = AgentOwner::Window {
                    page: PageId(*page),
                    target: target.clone(),
                    generation: *generation,
                    epoch: *epoch,
                };
                parents.get(&candidate) == Some(&owner)
                    && self.native_worker_authority(&candidate).is_ok()
            })
            .count();
        let opener = self.native_browsing_context_opener(
            target_page,
            &Creator::Window {
                target: target.clone(),
                document_generation: *generation,
                stream_epoch: *epoch,
            },
        )?;
        self.native_worker_authority(&source_owner)
            .map_err(|_| denied())?;
        self.native_worker_authority(&owner).map_err(|_| denied())?;
        if !self.native_window_group(source, target_page, expected)?.2 {
            return Err(denied());
        }
        let same_origin_domain = self
            .native_document_domain_identity(&source_owner)
            .map_err(RuntimeError::ContextService)?
            .same_origin_domain(
                &self
                    .native_document_domain_identity(&owner)
                    .map_err(RuntimeError::ContextService)?,
            );
        Ok(crate::context_service::RelatedWindowSnapshot {
            children: Vec::new(),
            same_origin_domain,
            creator: Creator::Window {
                target: target.clone(),
                document_generation: *generation,
                stream_epoch: *epoch,
            },
            parent,
            opener,
            length,
            url: same_origin_domain.then_some(policy.url),
        })
    }
}

#[cfg(all(test, unix))]
mod related_window_snapshot_tests {
    use oryn_js::{
        message_wire::{MessageRequest, WindowTarget},
        worker_wire::Creator,
    };
    #[tokio::test]
    async fn structural_snapshot_rejects_stale_source_and_hides_cross_origin_url() {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(id, "https://a.test/source".into(), &[])
            .unwrap();
        let (_, _, _, generation, _) = context
            .native_document_authority(id, "https://a.test/source")
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: generation,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: generation,
            stream_epoch: 0,
        };
        let child = context.new_popup(id, None);
        let child_id = child.page_id();
        context
            .install_document_security(child_id, "https://b.test/target".into(), &[])
            .unwrap();
        let (_, _, _, child_generation, _) = context
            .native_document_authority(child_id, "https://b.test/target")
            .unwrap();
        context
            .handle_message_wire(
                child_id,
                MessageRequest::RegisterRootDocument {
                    document_generation: child_generation,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let snapshot = context
            .read_related_window(id, &creator, child_id, &WindowTarget::Root, None)
            .unwrap();
        assert!(snapshot.url.is_none());
        assert_eq!(snapshot.parent, WindowTarget::Root);
        assert_eq!(snapshot.length, 0);
        assert_eq!(snapshot.opener, Some((id, WindowTarget::Root)));
        let stale = Creator::Window {
            target: WindowTarget::Root,
            document_generation: generation,
            stream_epoch: 1,
        };
        assert!(
            context
                .read_related_window(id, &stale, child_id, &WindowTarget::Root, None)
                .is_err()
        );
        let stranger = context.new_page();
        assert!(
            context
                .read_related_window(id, &creator, stranger.page_id(), &WindowTarget::Root, None)
                .is_err()
        );
        child.close().await.unwrap();
        assert!(
            context
                .read_related_window(id, &creator, child_id, &WindowTarget::Root, None)
                .is_err()
        );
        stranger.close().await.unwrap();
        page.close().await.unwrap();
    }
}

#[cfg(test)]
mod worker_script_report_owner_tests {
    use super::*;
    #[test]
    fn worker_csp_batch_admission_preserves_unacknowledged_events() {
        let policy = crate::csp::CspPolicySet::from_headers(
            "https://worker.test/main.js",
            &[(
                "Content-Security-Policy".into(),
                "require-trusted-types-for 'script'".into(),
            )],
        );
        let violation = policy
            .authorize_trusted_type_sink("Element innerHTML", "raw")
            .violations
            .remove(0);
        let mut registry = WorkerPolicyRegistry::default();
        let agent = AgentId(1);
        registry
            .admit_csp_events(agent, &vec![violation.clone(); 255])
            .unwrap();
        assert!(
            registry
                .admit_csp_events(agent, &[violation.clone(), violation.clone()])
                .is_err()
        );
        assert_eq!(registry.csp_event_id, 255);
        assert_eq!(registry.agent_csp_events[&agent].len(), 255);
        let first = registry.poll_csp_events(agent, 0);
        assert_eq!(first.len(), 64);
        assert_eq!(first[0].id, 1);
        assert_eq!(
            serde_json::to_value(registry.poll_csp_events(agent, 0)).unwrap(),
            serde_json::to_value(&first).unwrap(),
            "polling without acknowledgement must replay the same batch"
        );
        let next = registry.poll_csp_events(agent, 64);
        assert_eq!(next[0].id, 65);
        assert_eq!(registry.agent_csp_events[&agent].len(), 191);
        registry
            .admit_csp_events(agent, &[violation.clone(), violation])
            .unwrap();
        assert_eq!(registry.csp_event_id, 257);
        assert_eq!(registry.agent_csp_events[&agent].back().unwrap().id, 257);
    }
    #[test]
    fn worker_csp_identity_exhaustion_has_no_partial_batch() {
        let policy = crate::csp::CspPolicySet::from_headers(
            "https://worker.test/main.js",
            &[(
                "Content-Security-Policy".into(),
                "require-trusted-types-for 'script'".into(),
            )],
        );
        let violation = policy
            .authorize_trusted_type_sink("Element innerHTML", "raw")
            .violations
            .remove(0);
        let mut registry = WorkerPolicyRegistry {
            csp_event_id: u64::MAX - 1,
            ..Default::default()
        };
        assert!(
            registry
                .admit_csp_events(AgentId(1), &[violation.clone(), violation])
                .is_err()
        );
        assert_eq!(registry.csp_event_id, u64::MAX - 1);
        assert!(registry.agent_csp_events.is_empty());
    }

    use oryn_js::message_wire::{MessageRequest, WindowTarget};
    #[test]
    fn report_owner_rejects_retired_document_and_retained_job_never_targets_successor() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(id, "https://source.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let owner = AgentOwner::Window {
            page: id,
            target: WindowTarget::Root,
            generation: 1,
            epoch: 0,
        };
        let creator = oryn_js::worker_wire::Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let policy = crate::csp::CspPolicySet::from_headers(
            "https://source.test/",
            &[("Content-Security-Policy".into(), "worker-src 'none'".into())],
        );
        let decision = policy.authorize(&crate::csp::CspRequest {
            destination: crate::csp::CspDestination::Worker,
            url: "https://source.test/worker.js".into(),
            nonce: None,
            inline_source: None,
            parser_inserted: false,
            initiated_by_trusted_script: true,
        });
        context
            .report_worker_csp(
                &owner,
                &policy,
                decision.violations.clone(),
                "https://source.test/",
                false,
            )
            .unwrap();
        assert_eq!(
            context
                .poll_context_events(id, &creator, 0, 32)
                .unwrap()
                .iter()
                .filter(|entry| matches!(
                    &entry.event,
                    crate::context_service::ContextEvent::CspViolation { .. }
                ))
                .count(),
            1
        );
        context
            .handle_message_wire(
                id,
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .report_worker_csp(
                    &owner,
                    &policy,
                    decision.violations.clone(),
                    "https://source.test/",
                    false
                )
                .is_err()
        );
        // Admitted jobs retain their captured policy after the source dies, but
        // no stale event is addressed to a newer native root document.
        let count = context.inner.context_events.lock().unwrap().len();
        context
            .report_worker_csp(
                &owner,
                &policy,
                decision.violations,
                "https://source.test/",
                true,
            )
            .unwrap();
        assert_eq!(context.inner.context_events.lock().unwrap().len(), count);
        assert!(context.poll_agent_csp_events(AgentId(u64::MAX), 0).is_err());
    }
}

#[cfg(test)]
mod parser_meta_policy_timing_tests {
    use super::*;
    use oryn_js::message_wire::{MessageRequest, WindowTarget};
    #[test]
    fn response_preparation_does_not_apply_future_meta_before_native_parser_commit() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        let html = "<script>before()</script><meta http-equiv=Content-Security-Policy content=\"script-src 'none';style-src 'none'\"><script>after()</script>";
        context
            .prepare_document_security(id, "https://root.test/".into(), &[], html)
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let root = AgentOwner::Window {
            page: id,
            target: WindowTarget::Root,
            generation: 1,
            epoch: 0,
        };
        let root_policy = context
            .window_worker_policy(id, &WindowTarget::Root, 1, 0)
            .unwrap();
        assert!(!root_policy.hide_nonces);
        assert!(
            crate::csp::CspPolicySet::from_agent_policy(&root_policy)
                .allows_eval(false)
                .allowed
        );
        let frame = "f".repeat(64);
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                    origin: "https://child.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        let ticket = context
            .prepare_frame_worker_policy(id, "https://child.test/", &[], html)
            .unwrap();
        context
            .bind_frame_worker_policy(id, frame.clone(), 2, 0, "https://child.test/", &ticket)
            .unwrap();
        let child = AgentOwner::Window {
            page: id,
            target: WindowTarget::Frame(frame.clone()),
            generation: 2,
            epoch: 0,
        };
        assert!(
            crate::csp::CspPolicySet::from_agent_policy(
                &context
                    .window_worker_policy(id, &WindowTarget::Frame(frame.clone()), 2, 0)
                    .unwrap()
            )
            .allows_eval(false)
            .allowed
        );
        context
            .install_realm_metadata(&child, Some("script-src 'none';style-src 'none'"), None)
            .unwrap();
        let child_policy = context
            .window_worker_policy(id, &WindowTarget::Frame(frame), 2, 0)
            .unwrap();
        assert!(!child_policy.hide_nonces);
        assert!(
            !crate::csp::CspPolicySet::from_agent_policy(&child_policy)
                .allows_eval(false)
                .allowed
        );
        assert!(
            crate::csp::CspPolicySet::from_agent_policy(
                &context
                    .window_worker_policy(id, &WindowTarget::Root, 1, 0)
                    .unwrap()
            )
            .allows_eval(false)
            .allowed
        );
        context
            .install_realm_metadata(&root, Some("script-src 'none'"), None)
            .unwrap();
        assert!(
            !crate::csp::CspPolicySet::from_agent_policy(
                &context
                    .window_worker_policy(id, &WindowTarget::Root, 1, 0)
                    .unwrap()
            )
            .allows_eval(false)
            .allowed
        );
    }
}

#[cfg(test)]
mod nonactive_root_lifetime_tests {
    use super::*;
    #[test]
    fn preparing_nonactive_document_never_changes_current_security_publication() {
        let mut lifetime = RootDocumentLifetime::default();
        lifetime.publish().unwrap();
        let active = lifetime.reserve().unwrap();
        let original = lifetime.epoch;
        let candidate = lifetime.reserve_nonactive().unwrap();
        assert_eq!(lifetime.epoch, original);
        assert!(lifetime.current(active));
        assert!(!lifetime.current(candidate.0));
        assert!(
            lifetime
                .activate_nonactive(original + 1, candidate.0, candidate.1)
                .is_err()
        );
        assert_eq!(lifetime.epoch, original);
        lifetime
            .activate_nonactive(original, candidate.0, candidate.1)
            .unwrap();
        assert!(lifetime.current(candidate.0));
        assert!(!lifetime.current(active));
        lifetime.abandon_nonactive(candidate.0, candidate.1);
        assert!(lifetime.current(candidate.0));
    }
    #[test]
    fn canceled_nonactive_ids_and_epochs_are_never_reissued() {
        let mut lifetime = RootDocumentLifetime::default();
        lifetime.publish().unwrap();
        let first = lifetime.reserve_nonactive().unwrap();
        lifetime.abandon_nonactive(first.0, first.1);
        let second = lifetime.reserve_nonactive().unwrap();
        assert!(second.0 > first.0 && second.1 > first.1);
        assert!(
            lifetime
                .activate_nonactive(lifetime.epoch, first.0, first.1)
                .is_err()
        );
    }
}
