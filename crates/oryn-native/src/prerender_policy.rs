//! Parent-owned nonactive Document policy, kept separate from the active page.

/// Inputs for register native prerender frame.
pub(crate) struct NativePrerenderFrameInputs<'a> {
    pub(crate) page: PageId,
    pub(crate) parent: &'a Creator,
    pub(crate) frame: String,
    pub(crate) generation: u64,
    pub(crate) epoch: u64,
    pub(crate) origin: String,
    pub(crate) inherit_origin: bool,
}
use super::*;

impl BrowserContext {
    pub(crate) fn private_prerender_group(
        &self,
        owner: &AgentOwner,
    ) -> Result<Option<crate::browsing_groups::GroupToken>, RuntimeError> {
        let AgentOwner::Window {
            page,
            target,
            generation,
            epoch,
        } = owner
        else {
            return Ok(None);
        };
        let policies =
            self.inner.worker_policies.lock().map_err(|_| {
                RuntimeError::ContextService("private group policy poisoned".into())
            })?;
        let hub =
            self.inner.group.messages.lock().map_err(|_| {
                RuntimeError::ContextService("private group documents poisoned".into())
            })?;
        let Some((token, lifetime)) =
            hub.prerender_scope_for_document(page.0, target, *generation, *epoch)
        else {
            return Ok(None);
        };
        hub.prerender_target_authority(page.0, &token, lifetime, target, *generation, *epoch)
            .map_err(RuntimeError::ProtocolViolation)?;
        let scope = policies
            .prerenders
            .get(&(*page, lifetime.0, lifetime.1))
            .filter(|scope| scope.token == token && !scope.canceling)
            .ok_or(RuntimeError::NavigationCancelled)?;
        Ok(Some(scope.group.token))
    }
    pub(crate) fn register_native_prerender_frame(
        &self,
        params: NativePrerenderFrameInputs<'_>,
    ) -> Result<(), RuntimeError> {
        let NativePrerenderFrameInputs {
            page,
            parent,
            frame,
            generation,
            epoch,
            origin,
            inherit_origin,
        } = params;

        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = parent
        else {
            return Err(RuntimeError::ProtocolViolation(
                "prerender frame requires Window parent".into(),
            ));
        };
        self.window_worker_creator(page, target, *document_generation, *stream_epoch)?;
        let policies = self.inner.worker_policies.lock().map_err(|_| {
            RuntimeError::ContextService("prerender frame policies poisoned".into())
        })?;
        let mut hub = self.inner.group.messages.lock().map_err(|_| {
            RuntimeError::ContextService("prerender frame documents poisoned".into())
        })?;
        let (token, lifetime) = hub
            .prerender_scope_for_document(page.0, target, *document_generation, *stream_epoch)
            .ok_or(RuntimeError::NavigationCancelled)?;
        let scope = policies
            .prerenders
            .get(&(page, lifetime.0, lifetime.1))
            .filter(|scope| !scope.canceling && scope.token == token && scope.policy.is_some())
            .ok_or(RuntimeError::NavigationCancelled)?;
        let root_origin = SecurityOrigin::from_broker_authority(
            &scope.origin.serialized,
            &scope.origin.authority,
        )
        .map_err(|error| RuntimeError::ProtocolViolation(error.into()))?;
        hub.register_prerender_frame(oryn_js::message_wire::PrerenderFrameRegistration {
            page: page.0,
            token: &token,
            lifetime,
            parent: target,
            parent_generation: *document_generation,
            parent_epoch: *stream_epoch,
            frame,
            generation,
            epoch,
            frame_origin: origin,
            inherit_origin: inherit_origin.then(|| target.clone()),
            origin: &scope.origin.serialized,
            origin_key: &scope.origin.authority,
            partition: native_storage_partition(self.inner.id, &root_origin),
        })
        .map_err(RuntimeError::ProtocolViolation)
    }
}

#[derive(Clone)]
pub(crate) struct Scope {
    pub(crate) blank_source: Option<(PageId, Creator)>,
    pub(crate) blank_selected: bool,
    pub(crate) token: oryn_js::message_wire::PrerenderToken,
    pub(crate) initiator: (u64, u64),
    initiator_security_epoch: u64,
    pub(crate) lifetime: (u64, u64),
    pub(crate) url: String,
    pub(crate) policy: Option<oryn_js::worker_wire::AgentPolicy>,
    response_policy: Option<oryn_js::worker_wire::AgentPolicy>,
    pub(crate) origin: oryn_js::worker_wire::AgentOrigin,
    pub(crate) group: crate::browsing_groups::DocumentGroup,
    request_origin: oryn_js::worker_wire::AgentOrigin,
    pub(crate) canceling: bool,
    cleanup_stage: u8,
    cleanup_busy: bool,
    cleanup_owners: Vec<AgentOwner>,
    cleanup_owner: usize,
}

pub(super) struct Completion {
    token: oryn_js::message_wire::PrerenderToken,
    predecessor: oryn_js::message_wire::ParkedDocumentToken,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActivationAck {
    Current,
    Obsolete,
}

impl BrowserContext {
    /// Eligibility is native and conservative: this loader currently admits
    /// same-origin HTTP(S) roots; parsing a rule never grants network authority.
    pub(crate) fn reserve_native_prerender(
        &self,
        page: PageId,
        creator: &Creator,
        url: &str,
    ) -> Result<Scope, RuntimeError> {
        let Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "prerender requires a top-level document".into(),
            ));
        };
        let (_, _, origin) = self.window_worker_creator(
            page,
            &oryn_js::message_wire::WindowTarget::Root,
            *document_generation,
            *stream_epoch,
        )?;
        let url = url::Url::parse(url)
            .map_err(|_| RuntimeError::ProtocolViolation("invalid prerender URL".into()))?;
        let target = SecurityOrigin::from_url(url.as_str());
        if !matches!(url.scheme(), "http" | "https")
            || origin.opaque
            || target.security_token_key() != origin.authority
        {
            return Err(RuntimeError::ProtocolViolation(
                "prerender URL is not eligible for this native context".into(),
            ));
        }
        let group = crate::browsing_groups::BrowsingGroups::independent_document(
            origin.authority.clone(),
            crate::browsing_groups::Coop::UnsafeNone,
        )
        .map_err(RuntimeError::ContextService)?;
        let pages = self
            .inner
            .group
            .pages
            .lock()
            .map_err(|_| RuntimeError::ContextService("prerender pages poisoned".into()))?;
        if !pages.get(&page).is_some_and(|record| record.is_live()) {
            return Err(RuntimeError::PageNotFound(page));
        }
        let mut policies = self.inner.worker_policies.lock().map_err(|_| {
            RuntimeError::ContextService("prerender policy registry poisoned".into())
        })?;
        if policies.prerenders.len() >= 16 {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "prerender documents".into(),
            });
        }
        let mut hub = self
            .inner
            .group
            .messages
            .lock()
            .map_err(|_| RuntimeError::ContextService("prerender authority poisoned".into()))?;
        hub.root_document_authority(page.0, *document_generation, *stream_epoch)
            .map_err(RuntimeError::ProtocolViolation)?;
        let lifetime = policies
            .root_lifetimes
            .get_mut(&page)
            .ok_or(RuntimeError::PageNotFound(page))?;
        if !lifetime.current(*document_generation) {
            return Err(RuntimeError::NavigationCancelled);
        }
        let initiator_security_epoch = lifetime.epoch;
        let selected = lifetime
            .reserve_nonactive()
            .map_err(RuntimeError::ProtocolViolation)?;
        let token = match hub.reserve_prerender_document(
            page.0,
            (*document_generation, *stream_epoch),
            selected,
        ) {
            Ok(token) => token,
            Err(error) => {
                lifetime.abandon_nonactive(selected.0, selected.1);
                return Err(RuntimeError::ProtocolViolation(error));
            }
        };
        let scope = Scope {
            blank_source: None,
            blank_selected: false,
            token,
            initiator_security_epoch,
            initiator: (*document_generation, *stream_epoch),
            lifetime: selected,
            url: url.into(),
            policy: None,
            response_policy: None,
            request_origin: origin.clone(),
            origin,
            group,
            canceling: false,
            cleanup_stage: 0,
            cleanup_busy: false,
            cleanup_owners: Vec::new(),
            cleanup_owner: 0,
        };
        policies
            .prerenders
            .insert((page, selected.0, selected.1), scope.clone());
        Ok(scope)
    }

    /// The unguessable destination scope, never its current active document,
    /// owns an accepted candidate request. Selection transfers a blank-target
    /// candidate away from its initiating document's lifetime.
    pub(crate) fn authorize_prerender_navigation(
        &self,
        page: PageId,
        token: &oryn_js::message_wire::PrerenderToken,
        lifetime: (u64, u64),
        url: &str,
    ) -> Result<(), RuntimeError> {
        let scope = self.native_prerender_scope(page, token, lifetime)?;
        let target = url::Url::parse(url)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        if !matches!(target.scheme(), "http" | "https")
            || !target.username().is_empty()
            || target.password().is_some()
            || scope.request_origin.opaque
            || SecurityOrigin::from_url(target.as_str()).security_token_key()
                != scope.request_origin.authority
        {
            return Err(RuntimeError::ProtocolViolation(
                "prerender navigation left its admitted request origin".into(),
            ));
        }
        if !scope.blank_selected {
            let (source, creator) = scope.blank_source.clone().unwrap_or({
                (
                    page,
                    Creator::Window {
                        target: oryn_js::message_wire::WindowTarget::Root,
                        document_generation: scope.initiator.0,
                        stream_epoch: scope.initiator.1,
                    },
                )
            });
            let Creator::Window {
                target,
                document_generation,
                stream_epoch,
            } = creator
            else {
                return Err(RuntimeError::NavigationCancelled);
            };
            self.window_worker_creator(source, &target, document_generation, stream_epoch)?;
        }
        // Cancellation racing the source check must not grant a later hop.
        self.native_prerender_scope(page, token, lifetime)?;
        if !self
            .inner
            .group
            .pages
            .lock()
            .map_err(|_| RuntimeError::ContextService("prerender request pages poisoned".into()))?
            .get(&page)
            .is_some_and(|record| record.is_live())
        {
            return Err(RuntimeError::PageNotFound(page));
        }
        Ok(())
    }

    pub(crate) fn native_prerender_scope(
        &self,
        page: PageId,
        token: &oryn_js::message_wire::PrerenderToken,
        lifetime: (u64, u64),
    ) -> Result<Scope, RuntimeError> {
        let policies = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| RuntimeError::ContextService("prerender policies poisoned".into()))?;
        let scope = policies
            .prerenders
            .get(&(page, lifetime.0, lifetime.1))
            .filter(|scope| scope.token == *token && !scope.canceling)
            .ok_or(RuntimeError::NavigationCancelled)?;
        let hub = self
            .inner
            .group
            .messages
            .lock()
            .map_err(|_| RuntimeError::ContextService("prerender authority poisoned".into()))?;
        hub.prerender_document_authority(page.0, token, lifetime)
            .map_err(|_| RuntimeError::NavigationCancelled)?;
        Ok(scope.clone())
    }

    pub(crate) fn mark_native_prerender_ready(
        &self,
        page: PageId,
        token: &oryn_js::message_wire::PrerenderToken,
        lifetime: (u64, u64),
    ) -> Result<(), RuntimeError> {
        let policies = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| RuntimeError::ContextService("prerender policies poisoned".into()))?;
        let scope = policies
            .prerenders
            .get(&(page, lifetime.0, lifetime.1))
            .filter(|scope| scope.token == *token && !scope.canceling)
            .ok_or(RuntimeError::NavigationCancelled)?;
        if scope.policy.is_none() {
            return Err(RuntimeError::ProtocolViolation(
                "unprepared prerender cannot create a Document".into(),
            ));
        }
        let mut hub = self
            .inner
            .group
            .messages
            .lock()
            .map_err(|_| RuntimeError::ContextService("prerender authority poisoned".into()))?;
        hub.mark_prerender_ready(page.0, token, lifetime)
            .map_err(|_| RuntimeError::NavigationCancelled)
    }

    pub(crate) fn mark_native_prerender_loaded(
        &self,
        page: PageId,
        token: &oryn_js::message_wire::PrerenderToken,
        lifetime: (u64, u64),
    ) -> Result<(), RuntimeError> {
        let policies = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| RuntimeError::ContextService("prerender policies poisoned".into()))?;
        let scope = policies
            .prerenders
            .get(&(page, lifetime.0, lifetime.1))
            .filter(|scope| scope.token == *token && !scope.canceling)
            .ok_or(RuntimeError::NavigationCancelled)?;
        if scope.policy.is_none() {
            return Err(RuntimeError::ProtocolViolation(
                "unprepared prerender cannot complete loading".into(),
            ));
        }
        let mut hub = self
            .inner
            .group
            .messages
            .lock()
            .map_err(|_| RuntimeError::ContextService("prerender authority poisoned".into()))?;
        hub.mark_prerender_loaded(page.0, token, lifetime)
            .map_err(|_| RuntimeError::NavigationCancelled)
    }

    /// The candidate's complete session-history entry owns an independent
    /// browsing context/group. All fallible preparation precedes hub promotion;
    /// the locked publication never enters worker callbacks or repeats unload.
    pub(crate) fn activate_native_prerender(
        &self,
        page: PageId,
        token: &oryn_js::message_wire::PrerenderToken,
        lifetime: (u64, u64),
        predecessor: &oryn_js::message_wire::ParkedDocumentToken,
    ) -> Result<ActivationAck, RuntimeError> {
        let mut pages = self
            .inner
            .group
            .pages
            .lock()
            .map_err(|_| RuntimeError::ContextService("prerender pages poisoned".into()))?;
        let record = pages
            .get_mut(&page)
            .filter(|record| record.is_live())
            .ok_or(RuntimeError::PageNotFound(page))?;
        let mut groups = self
            .inner
            .group
            .browsing_groups
            .lock()
            .map_err(|_| RuntimeError::ContextService("prerender groups poisoned".into()))?;
        if groups.snapshot(page).is_none() {
            return Err(RuntimeError::PageNotFound(page));
        }
        let mut origins = self
            .inner
            .security_origins
            .lock()
            .map_err(|_| RuntimeError::ContextService("prerender origins poisoned".into()))?;
        let mut policies = self.inner.document_security.lock().map_err(|_| {
            RuntimeError::ContextService("prerender document policies poisoned".into())
        })?;
        let mut referrers = self
            .inner
            .document_referrer_policy
            .lock()
            .map_err(|_| RuntimeError::ContextService("prerender referrers poisoned".into()))?;
        let mut workers = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| RuntimeError::ContextService("prerender workers poisoned".into()))?;
        let mut hub = self.inner.group.messages.lock().map_err(|_| {
            RuntimeError::ContextService("prerender message authority poisoned".into())
        })?;
        let key = (page, lifetime.0, lifetime.1);
        if let Some(done) = workers.prerender_completions.get(&key) {
            if done.token != *token || done.predecessor != *predecessor {
                return Err(RuntimeError::ProtocolViolation(
                    "conflicting prerender activation replay".into(),
                ));
            }
            return Ok(
                if hub.root_document_identity(page.0) == Some(lifetime)
                    && workers
                        .root_lifetimes
                        .get(&page)
                        .is_some_and(|root| root.current(lifetime.0))
                {
                    ActivationAck::Current
                } else {
                    ActivationAck::Obsolete
                },
            );
        }
        let Some(candidate) = workers.prerenders.get(&key).cloned() else {
            // A consumed authenticated token can acknowledge disposal only.
            return if hub.prerender_token_consumed(page.0, token) {
                Ok(ActivationAck::Obsolete)
            } else {
                Err(RuntimeError::ProtocolViolation(
                    "unknown prerender activation".into(),
                ))
            };
        };
        if candidate.token != *token || candidate.canceling {
            return Err(RuntimeError::NavigationCancelled);
        }
        let policy = candidate.policy.as_ref().ok_or_else(|| {
            RuntimeError::ProtocolViolation("prerender response unprepared".into())
        })?;
        let origin = SecurityOrigin::from_broker_authority(
            &candidate.origin.serialized,
            &candidate.origin.authority,
        )
        .map_err(|error| RuntimeError::ProtocolViolation(error.into()))?;
        let csp = crate::csp::CspPolicySet::from_agent_policy(policy);
        let referrer = crate::referrer::ReferrerPolicy::default().after_response(&policy.headers);
        let isolation =
            crate::agent_isolation::DocumentIsolationPolicy::from_headers(&policy.headers);
        let isolation_permissions = policy
            .headers
            .iter()
            .filter(|(name, _)| {
                name.eq_ignore_ascii_case("permissions-policy")
                    || name.eq_ignore_ascii_case("origin-agent-cluster")
            })
            .cloned()
            .collect();
        let mut root = workers
            .root_lifetimes
            .get(&page)
            .cloned()
            .ok_or(RuntimeError::PageNotFound(page))?;
        root.activate_nonactive(candidate.initiator_security_epoch, lifetime.0, lifetime.1)
            .map_err(|_| RuntimeError::NavigationCancelled)?;
        hub.promote_prerender_after_departure(page.0, token, lifetime, predecessor)
            .map_err(RuntimeError::ProtocolViolation)?;
        // Presence was preflighted under this same guard, so restore cannot
        // fail after authority consumption. It performs no external work.
        groups
            .restore(page, candidate.group)
            .expect("preflighted browsing group");
        record.descriptor.page.opener_page_id = None;
        record.descriptor.page.url = candidate.url;
        if origins
            .get(&page)
            .is_some_and(|previous| previous.security_token_key() != origin.security_token_key())
        {
            record.target_name.clear();
        }
        origins.insert(page, origin);
        policies.insert(page, csp);
        referrers.insert(page, referrer);
        workers.root_lifetimes.insert(page, root);
        workers.root_isolation.insert(page, isolation);
        workers
            .root_isolation_permissions
            .insert(page, isolation_permissions);
        workers.prerenders.remove(&key);
        workers.prerender_completions.insert(
            key,
            Completion {
                token: token.clone(),
                predecessor: predecessor.clone(),
            },
        );
        // Keep the current ACK while its document is active; prune older
        // receipts per page. Expired receipts return disposal, never success.
        let old = workers
            .prerender_completions
            .keys()
            .filter(|(owner, _, _)| *owner == page)
            .copied()
            .collect::<Vec<_>>();
        for key_to_remove in old
            .iter()
            .filter(|old| **old != key)
            .take(old.len().saturating_sub(64))
        {
            workers.prerender_completions.remove(key_to_remove);
        }
        Ok(ActivationAck::Current)
    }

    /// Commit only the response policy for this nonactive scope. The current
    /// document's URL, origin, CSP, referrer and epoch are untouched.
    pub(crate) fn commit_prerender_response_policy(
        &self,
        page: PageId,
        scope: &Scope,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<oryn_js::worker_wire::AgentPolicy, RuntimeError> {
        let target = SecurityOrigin::from_url(url);
        if scope.request_origin.opaque
            || target.security_token_key() != scope.request_origin.authority
        {
            return Err(RuntimeError::ProtocolViolation(
                "prerender redirected outside its admitted origin".into(),
            ));
        }
        if headers.len() > 512
            || headers
                .iter()
                .map(|(name, value)| name.len().saturating_add(value.len()))
                .sum::<usize>()
                > 1024 * 1024
        {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "prerender response headers".into(),
            });
        }
        let mut csp = crate::csp::CspPolicySet::from_headers(url, headers);
        let opaque = csp
            .sandbox()
            .is_some_and(|sandbox| !sandbox.allow_same_origin);
        if opaque {
            csp.set_document_self_origin("null");
        }
        let mut policy = csp.worker_snapshot();
        policy.headers.extend(
            headers
                .iter()
                .filter(|(name, _)| {
                    [
                        "referrer-policy",
                        "cross-origin-opener-policy",
                        "cross-origin-embedder-policy",
                        "permissions-policy",
                        "origin-agent-cluster",
                    ]
                    .iter()
                    .any(|field| name.eq_ignore_ascii_case(field))
                })
                .cloned(),
        );
        let pages = self
            .inner
            .group
            .pages
            .lock()
            .map_err(|_| RuntimeError::ContextService("prerender pages poisoned".into()))?;
        if !pages.get(&page).is_some_and(|record| record.is_live()) {
            return Err(RuntimeError::PageNotFound(page));
        }
        let mut policies = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| RuntimeError::ContextService("prerender policies poisoned".into()))?;
        let hub = self
            .inner
            .group
            .messages
            .lock()
            .map_err(|_| RuntimeError::ContextService("prerender authority poisoned".into()))?;
        hub.prerender_document_authority(page.0, &scope.token, scope.lifetime)
            .map_err(RuntimeError::ProtocolViolation)?;
        let current = policies
            .prerenders
            .get_mut(&(page, scope.lifetime.0, scope.lifetime.1))
            .filter(|current| current.token == scope.token)
            .ok_or(RuntimeError::NavigationCancelled)?;
        if current.canceling {
            return Err(RuntimeError::NavigationCancelled);
        }
        if let Some(previous) = &current.response_policy {
            if previous.url != policy.url || previous.headers != policy.headers {
                return Err(RuntimeError::ProtocolViolation(
                    "prerender response policy already committed".into(),
                ));
            }
            return Ok(previous.clone());
        }
        if opaque {
            let origin = SecurityOrigin::opaque();
            current.origin = oryn_js::worker_wire::AgentOrigin {
                serialized: origin.serialize(),
                authority: origin.security_token_key(),
                opaque: true,
            };
        }
        current.group.origin = current.origin.authority.clone();
        current.group.policy = crate::browsing_groups::Coop::from_headers(headers);
        current.url = url.into();
        current.response_policy = Some(policy.clone());
        current.policy = Some(policy.clone());
        Ok(policy)
    }
}

// The policy entry owns cancellation progress until all native children have
// acknowledged retirement. No registry mutex crosses worker or service calls.
#[cfg(unix)]
struct CancellationLease<'a> {
    context: &'a BrowserContext,
    key: (PageId, u64, u64),
    token: oryn_js::message_wire::PrerenderToken,
}
#[cfg(unix)]
impl Drop for CancellationLease<'_> {
    fn drop(&mut self) {
        let mut policies = self
            .context
            .inner
            .worker_policies
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(scope) = policies
            .prerenders
            .get_mut(&self.key)
            .filter(|scope| scope.token == self.token)
        {
            scope.cleanup_busy = false;
        }
    }
}
#[cfg(unix)]
impl BrowserContext {
    pub(crate) fn cancel_native_prerender(
        &self,
        page: PageId,
        token: &oryn_js::message_wire::PrerenderToken,
        lifetime: (u64, u64),
    ) -> Result<(), RuntimeError> {
        let key = (page, lifetime.0, lifetime.1);
        {
            let mut policies =
                self.inner.worker_policies.lock().map_err(|_| {
                    RuntimeError::ContextService("prerender cleanup poisoned".into())
                })?;
            let mut hub = self
                .inner
                .group
                .messages
                .lock()
                .map_err(|_| RuntimeError::ContextService("prerender hub poisoned".into()))?;
            let mut owners = policies
                .frame_security_epochs
                .iter()
                .filter(|((owner, _, _, _), security)| *owner == page && **security == lifetime.1)
                .map(|((_, frame, generation, epoch), _)| AgentOwner::Window {
                    page,
                    target: oryn_js::message_wire::WindowTarget::Frame(frame.clone()),
                    generation: *generation,
                    epoch: *epoch,
                })
                .collect::<Vec<_>>();
            owners.push(AgentOwner::Window {
                page,
                target: oryn_js::message_wire::WindowTarget::Root,
                generation: lifetime.0,
                epoch: lifetime.1,
            });
            let Some(scope) = policies.prerenders.get_mut(&key) else {
                return if hub.prerender_token_consumed(page.0, token) {
                    Ok(())
                } else {
                    Err(RuntimeError::ProtocolViolation(
                        "unknown prerender cancellation".into(),
                    ))
                };
            };
            if scope.token != *token {
                return Err(RuntimeError::ProtocolViolation(
                    "foreign prerender cancellation".into(),
                ));
            }
            if scope.cleanup_busy {
                return Err(RuntimeError::ContextService(
                    "prerender cancellation is in progress".into(),
                ));
            }
            // Revoke first, even if a later child cleanup needs a retry.
            hub.cancel_prerender_document(page.0, token)
                .map_err(RuntimeError::ProtocolViolation)?;
            if !scope.canceling {
                scope.cleanup_owners = owners;
                scope.cleanup_owner = 0;
            }
            scope.canceling = true;
            scope.cleanup_busy = true;
        }
        let _lease = CancellationLease {
            context: self,
            key,
            token: token.clone(),
        };
        loop {
            let (owner, stage) = {
                let policies = self.inner.worker_policies.lock().map_err(|_| {
                    RuntimeError::ContextService("prerender cleanup poisoned".into())
                })?;
                let scope = policies
                    .prerenders
                    .get(&key)
                    .ok_or(RuntimeError::NavigationCancelled)?;
                let Some(owner) = scope.cleanup_owners.get(scope.cleanup_owner).cloned() else {
                    break;
                };
                (owner, scope.cleanup_stage)
            };
            #[cfg(test)]
            if PRERENDER_CLEANUP_FAULT.with(|fault| fault.get() == Some(stage)) {
                PRERENDER_CLEANUP_FAULT.with(|fault| fault.set(None));
                return Err(RuntimeError::ContextService(
                    "injected prerender cleanup failure".into(),
                ));
            }
            match stage {
                0 => self.retire_window_transactions(&owner)?,
                1 => self.retire_window_sockets(&owner)?,
                2 => {
                    if let Some(services) = self.existing_service_registry() {
                        services
                            .retire_client(&owner)
                            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
                    }
                }
                3 => self.clear_owner_blob_urls(&owner),
                4 => self
                    .dispatch_worker_registry(|agents, hub| agents.retire_owner(hub, &owner))
                    .map_err(|error| RuntimeError::ContextService(error.to_string()))?,
                _ => unreachable!(),
            }
            let mut policies =
                self.inner.worker_policies.lock().map_err(|_| {
                    RuntimeError::ContextService("prerender cleanup poisoned".into())
                })?;
            let Some(scope) = policies
                .prerenders
                .get_mut(&key)
                .filter(|scope| scope.token == *token)
            else {
                return Ok(());
            };
            scope.cleanup_stage = stage + 1;
            if scope.cleanup_stage == 5 {
                scope.cleanup_stage = 0;
                scope.cleanup_owner += 1;
                drop(policies);
                self.retire_worker_secure_context(&owner);
            }
        }
        let mut policies = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| RuntimeError::ContextService("prerender cleanup poisoned".into()))?;
        if policies
            .prerenders
            .get(&key)
            .is_some_and(|scope| scope.token == *token)
        {
            let scope = policies.prerenders.remove(&key).unwrap();
            policies
                .document_domains
                .discard_private_group(scope.group.token);
            for owner in scope.cleanup_owners {
                policies.activations.remove(&owner);
                policies.sticky_activations.remove(&owner);
                policies.activation_parents.remove(&owner);
                if let AgentOwner::Window {
                    page,
                    target: oryn_js::message_wire::WindowTarget::Frame(frame),
                    generation,
                    epoch,
                } = owner
                {
                    let key = (page, frame, generation, epoch);
                    policies.frames.remove(&key);
                    policies.frame_origins.remove(&key);
                    policies.frame_security_epochs.remove(&key);
                    policies.frame_isolation.remove(&key);
                }
            }
            if let Some(root) = policies.root_lifetimes.get_mut(&page) {
                root.abandon_nonactive(lifetime.0, lifetime.1);
            }
        }
        Ok(())
    }
    pub(crate) fn retry_prerender_cancellations(&self) -> Result<(), RuntimeError> {
        let pending = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| RuntimeError::ContextService("prerender cleanup poisoned".into()))?
            .prerenders
            .iter()
            .filter(|(_, scope)| scope.canceling && !scope.cleanup_busy)
            .map(|((page, _, _), scope)| (*page, scope.token.clone(), scope.lifetime))
            .collect::<Vec<_>>();
        let mut first = None;
        for (page, token, lifetime) in pending {
            if let Err(error) = self.cancel_native_prerender(page, &token, lifetime)
                && first.is_none()
            {
                first = Some(error);
            }
        }
        first.map_or(Ok(()), Err)
    }
}
#[cfg(all(test, unix))]
thread_local! {static PRERENDER_CLEANUP_FAULT:std::cell::Cell<Option<u8>>=const{std::cell::Cell::new(None)};}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn selected_blank_request_scope_survives_source_retirement_but_not_cancellation() {
        let context = Browser::native_test_with_config(Default::default()).new_context();
        let source_page = context.new_page();
        let source = source_page.page_id();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime
            .block_on(
                source_page.load_html("https://blank.test/source", "<!doctype html><p>source</p>"),
            )
            .unwrap();
        let creator = context.capture_command_navigation_source(source).unwrap();
        let (destination, _, scope) = context
            .reserve_blank_prerender(source, &creator, "https://blank.test/start")
            .unwrap();
        assert!(
            context
                .authorize_prerender_navigation(
                    destination,
                    &scope.token,
                    scope.lifetime,
                    &scope.url
                )
                .is_ok()
        );
        context
            .claim_blank_prerender(source, &creator, Some(destination), &scope.url)
            .unwrap()
            .unwrap();
        context
            .install_document_security(
                source,
                "https://replacement.test/".into(),
                &[(
                    "content-security-policy".into(),
                    "default-src 'none'".into(),
                )],
            )
            .unwrap();
        // Neither the original creator nor replacement authority is consulted.
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = &creator
        else {
            panic!("native source must be a window");
        };
        assert!(
            context
                .window_worker_creator(source, target, *document_generation, *stream_epoch)
                .is_err()
        );
        assert!(
            context
                .authorize_prerender_navigation(
                    destination,
                    &scope.token,
                    scope.lifetime,
                    "https://blank.test/redirect"
                )
                .is_ok()
        );
        for url in [
            "https://replacement.test/",
            "http://blank.test/",
            "data:text/html,x",
            "https://user:pass@blank.test/",
        ] {
            assert!(
                context
                    .authorize_prerender_navigation(destination, &scope.token, scope.lifetime, url)
                    .is_err()
            );
        }
        assert!(
            context
                .authorize_prerender_navigation(source, &scope.token, scope.lifetime, &scope.url)
                .is_err()
        );
        assert!(
            context
                .authorize_prerender_navigation(
                    destination,
                    &scope.token,
                    (scope.lifetime.0, scope.lifetime.1 + 1),
                    &scope.url
                )
                .is_err()
        );
        context
            .cancel_native_prerender(destination, &scope.token, scope.lifetime)
            .unwrap();
        assert!(
            context
                .authorize_prerender_navigation(
                    destination,
                    &scope.token,
                    scope.lifetime,
                    &scope.url
                )
                .is_err()
        );
    }
    #[test]
    fn unselected_candidate_does_not_gain_source_retirement_authority() {
        let context = Browser::native_test_with_config(Default::default()).new_context();
        let source_page = context.new_page();
        let source = source_page.page_id();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime
            .block_on(
                source_page.load_html("https://blank.test/source", "<!doctype html><p>source</p>"),
            )
            .unwrap();
        let creator = context.capture_command_navigation_source(source).unwrap();
        let (destination, _, scope) = context
            .reserve_blank_prerender(source, &creator, "https://blank.test/start")
            .unwrap();
        context
            .install_document_security(source, "https://blank.test/replacement".into(), &[])
            .unwrap();
        assert!(
            context
                .authorize_prerender_navigation(
                    destination,
                    &scope.token,
                    scope.lifetime,
                    &scope.url
                )
                .is_err()
        );
    }

    #[test]
    fn private_child_policy_response_and_promotion_keep_original_tree_authority() {
        use oryn_js::message_wire::{MessageRequest as Q, WindowTarget};
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://prerender.test/source".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                Q::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let original = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let scope = context
            .reserve_native_prerender(page, &original, "https://prerender.test/candidate")
            .unwrap();
        context
            .commit_prerender_response_policy(page, &scope, &scope.url, &[])
            .unwrap();
        let parent = Creator::Window {
            target: WindowTarget::Root,
            document_generation: scope.lifetime.0,
            stream_epoch: scope.lifetime.1,
        };
        let root = AgentOwner::Window {
            page,
            target: WindowTarget::Root,
            generation: scope.lifetime.0,
            epoch: scope.lifetime.1,
        };
        context.publish_worker_secure_context(root, true).unwrap();
        let frame = "a".repeat(64);
        context
            .register_native_prerender_frame(NativePrerenderFrameInputs {
                page,
                parent: &parent,
                frame: frame.clone(),
                generation: 1,
                epoch: 0,
                origin: "https://prerender.test".into(),
                inherit_origin: true,
            })
            .unwrap();
        context
            .inherit_frame_worker_policy_at_url(
                crate::runtime::worker_service::InheritedFramePolicyInputs {
                    page,
                    frame: frame.clone(),
                    generation: 1,
                    epoch: 0,
                    parent: &WindowTarget::Root,
                    parent_generation: scope.lifetime.0,
                    parent_epoch: scope.lifetime.1,
                    document_url: "about:blank",
                },
            )
            .unwrap();
        assert!(
            context
                .inner
                .group
                .messages
                .lock()
                .unwrap()
                .frame_document_authority(page.0, &frame, 1, 0)
                .is_err()
        );
        assert!(
            context
                .window_worker_creator(page, &WindowTarget::Frame(frame.clone()), 1, 0)
                .is_ok()
        );
        let ticket = context
            .prepare_frame_worker_policy_for_creator(
                page,
                "https://child.test/frame",
                &[],
                "",
                Some(&parent),
            )
            .unwrap();
        context
            .register_native_prerender_frame(NativePrerenderFrameInputs {
                page,
                parent: &parent,
                frame: frame.clone(),
                generation: 2,
                epoch: 0,
                origin: "https://child.test".into(),
                inherit_origin: false,
            })
            .unwrap();
        context
            .bind_frame_worker_policy(
                page,
                frame.clone(),
                2,
                0,
                "https://child.test/frame",
                &ticket,
            )
            .unwrap();
        assert!(
            context
                .window_worker_creator(page, &WindowTarget::Frame(frame.clone()), 1, 0)
                .is_err()
        );
        assert_eq!(
            context
                .window_worker_creator(page, &WindowTarget::Frame(frame.clone()), 2, 0)
                .unwrap()
                .2
                .serialized,
            "https://child.test"
        );
        context
            .mark_native_prerender_loaded(page, &scope.token, scope.lifetime)
            .unwrap();
        let predecessor = context
            .inner
            .group
            .messages
            .lock()
            .unwrap()
            .park_document_authority(page.0, 1, 0)
            .unwrap();
        context
            .activate_native_prerender(page, &scope.token, scope.lifetime, &predecessor)
            .unwrap();
        assert_eq!(
            context
                .window_worker_creator(page, &WindowTarget::Frame(frame.clone()), 2, 0)
                .unwrap()
                .2
                .serialized,
            "https://child.test"
        );
        assert!(
            context
                .inner
                .group
                .messages
                .lock()
                .unwrap()
                .frame_document_authority(page.0, &frame, 2, 0)
                .is_ok()
        );
    }
    #[test]
    fn activation_moves_group_once_and_disposal_cannot_retire_active_candidate() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://prerender.test/source".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let scope = context
            .reserve_native_prerender(page, &creator, "https://prerender.test/candidate")
            .unwrap();
        context
            .commit_prerender_response_policy(page, &scope, &scope.url, &[])
            .unwrap();
        context
            .mark_native_prerender_loaded(page, &scope.token, scope.lifetime)
            .unwrap();
        let predecessor = context
            .inner
            .group
            .messages
            .lock()
            .unwrap()
            .park_document_authority(page.0, 1, 0)
            .unwrap();
        for _ in 0..2 {
            assert_eq!(
                context
                    .activate_native_prerender(page, &scope.token, scope.lifetime, &predecessor)
                    .unwrap(),
                ActivationAck::Current
            );
        }
        assert_eq!(
            context
                .inner
                .group
                .browsing_groups
                .lock()
                .unwrap()
                .token(page),
            Some(scope.group.token)
        );
        assert_eq!(context.page_url(page).unwrap(), scope.url);
        context
            .cancel_native_prerender(page, &scope.token, scope.lifetime)
            .unwrap();
        assert_eq!(
            context
                .inner
                .group
                .messages
                .lock()
                .unwrap()
                .root_document_identity(page.0),
            Some(scope.lifetime)
        );
        context
            .inner
            .group
            .messages
            .lock()
            .unwrap()
            .park_document_authority(page.0, scope.lifetime.0, scope.lifetime.1)
            .unwrap();
        assert_eq!(
            context
                .activate_native_prerender(page, &scope.token, scope.lifetime, &predecessor)
                .unwrap(),
            ActivationAck::Obsolete
        );
    }
    #[test]
    fn loaded_milestone_requires_prepared_exact_current_scope_and_replays_without_activation() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://prerender.test/source".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let scope = context
            .reserve_native_prerender(page, &creator, "https://prerender.test/candidate")
            .unwrap();
        let source_group = context
            .inner
            .group
            .browsing_groups
            .lock()
            .unwrap()
            .token(page)
            .unwrap();
        assert_ne!(scope.group.token, source_group);
        assert!(
            context
                .mark_native_prerender_loaded(page, &scope.token, scope.lifetime)
                .is_err()
        );
        assert!(
            context
                .mark_native_prerender_ready(page, &scope.token, scope.lifetime)
                .is_err()
        );
        context
            .commit_prerender_response_policy(page, &scope, &scope.url, &[])
            .unwrap();
        for _ in 0..2 {
            context
                .mark_native_prerender_ready(page, &scope.token, scope.lifetime)
                .unwrap();
        }
        for _ in 0..2 {
            context
                .mark_native_prerender_loaded(page, &scope.token, scope.lifetime)
                .unwrap();
        }
        assert_eq!(
            context
                .inner
                .group
                .browsing_groups
                .lock()
                .unwrap()
                .token(page),
            Some(source_group)
        );
        assert_eq!(
            context
                .native_prerender_scope(page, &scope.token, scope.lifetime)
                .unwrap()
                .group
                .token,
            scope.group.token
        );
        assert_eq!(
            context
                .inner
                .group
                .messages
                .lock()
                .unwrap()
                .root_document_identity(page.0),
            Some((1, 0))
        );
        assert!(
            context
                .mark_native_prerender_loaded(
                    page,
                    &scope.token,
                    (scope.lifetime.0, scope.lifetime.1 + 1)
                )
                .is_err()
        );
        context
            .cancel_native_prerender(page, &scope.token, scope.lifetime)
            .unwrap();
        assert!(matches!(
            context.mark_native_prerender_loaded(page, &scope.token, scope.lifetime),
            Err(RuntimeError::NavigationCancelled)
        ));
    }
    #[test]
    fn sandboxed_candidate_gets_stable_private_opaque_origin_without_changing_initiator() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://prerender.test/source".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let scope = context
            .reserve_native_prerender(page, &creator, "https://prerender.test/candidate")
            .unwrap();
        let headers = vec![(
            "content-security-policy".into(),
            "sandbox allow-scripts".into(),
        )];
        context
            .commit_prerender_response_policy(page, &scope, &scope.url, &headers)
            .unwrap();
        let first = context
            .native_prerender_scope(page, &scope.token, scope.lifetime)
            .unwrap();
        assert!(first.origin.opaque);
        assert_eq!(first.origin.serialized, "null");
        context
            .commit_prerender_response_policy(page, &first, &scope.url, &headers)
            .unwrap();
        assert_eq!(
            context
                .native_prerender_scope(page, &scope.token, scope.lifetime)
                .unwrap()
                .origin
                .authority,
            first.origin.authority
        );
        assert!(
            !context
                .window_worker_creator(page, &oryn_js::message_wire::WindowTarget::Root, 1, 0)
                .unwrap()
                .2
                .opaque
        );
        context
            .cancel_native_prerender(page, &scope.token, scope.lifetime)
            .unwrap();
    }
    #[test]
    fn cancellation_revokes_before_cleanup_and_retries_exact_suffix() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://prerender.test/source".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let scope = context
            .reserve_native_prerender(page, &creator, "https://prerender.test/candidate")
            .unwrap();
        context
            .commit_prerender_response_policy(page, &scope, &scope.url, &[])
            .unwrap();
        PRERENDER_CLEANUP_FAULT.with(|fault| fault.set(Some(2)));
        assert!(
            context
                .cancel_native_prerender(page, &scope.token, scope.lifetime)
                .is_err()
        );
        assert!(
            context
                .window_worker_creator(
                    page,
                    &oryn_js::message_wire::WindowTarget::Root,
                    scope.lifetime.0,
                    scope.lifetime.1
                )
                .is_err()
        );
        let key = (page, scope.lifetime.0, scope.lifetime.1);
        {
            let policies = context.inner.worker_policies.lock().unwrap();
            let saved = &policies.prerenders[&key];
            assert!(saved.canceling);
            assert_eq!(saved.cleanup_stage, 2);
            assert!(!saved.cleanup_busy);
        }
        context.retry_prerender_cancellations().unwrap();
        assert!(
            !context
                .inner
                .worker_policies
                .lock()
                .unwrap()
                .prerenders
                .contains_key(&key)
        );
        context
            .cancel_native_prerender(page, &scope.token, scope.lifetime)
            .unwrap();
        assert_eq!(
            context
                .inner
                .group
                .messages
                .lock()
                .unwrap()
                .root_document_identity(page.0),
            Some((1, 0))
        );
    }
    #[test]
    fn response_policy_isolated_from_active_document_and_stale_source_rejected() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://prerender.test/source".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let scope = context
            .reserve_native_prerender(page, &creator, "https://prerender.test/candidate")
            .unwrap();
        let before = context
            .window_worker_policy(page, &oryn_js::message_wire::WindowTarget::Root, 1, 0)
            .unwrap();
        let own = context
            .commit_prerender_response_policy(
                page,
                &scope,
                "https://prerender.test/final",
                &[("content-security-policy".into(), "script-src 'none'".into())],
            )
            .unwrap();
        assert_eq!(
            context
                .window_worker_policy(
                    page,
                    &oryn_js::message_wire::WindowTarget::Root,
                    scope.lifetime.0,
                    scope.lifetime.1
                )
                .unwrap()
                .headers,
            own.headers
        );
        let candidate = context
            .window_worker_creator(
                page,
                &oryn_js::message_wire::WindowTarget::Root,
                scope.lifetime.0,
                scope.lifetime.1,
            )
            .unwrap()
            .0;
        context
            .install_realm_metadata(&candidate, Some("img-src 'none'"), Some("no-referrer"))
            .unwrap();
        context
            .update_realm_document_url(&candidate, "https://prerender.test/final#changed")
            .unwrap();
        let updated = context
            .window_worker_policy(
                page,
                &oryn_js::message_wire::WindowTarget::Root,
                scope.lifetime.0,
                scope.lifetime.1,
            )
            .unwrap();
        assert!(
            updated
                .headers
                .iter()
                .any(|(_, value)| value.contains("img-src 'none'"))
        );
        assert_eq!(updated.url, "https://prerender.test/final#changed");
        // A lost response-policy ACK replays the original response commitment,
        // without overwriting metadata installed by the candidate parser.
        assert_eq!(
            context
                .commit_prerender_response_policy(
                    page,
                    &scope,
                    "https://prerender.test/final",
                    &[("content-security-policy".into(), "script-src 'none'".into())]
                )
                .unwrap()
                .headers,
            own.headers
        );
        assert_eq!(
            context
                .window_worker_policy(
                    page,
                    &oryn_js::message_wire::WindowTarget::Root,
                    scope.lifetime.0,
                    scope.lifetime.1
                )
                .unwrap()
                .headers,
            updated.headers
        );
        assert_eq!(own.url, "https://prerender.test/final");
        assert_eq!(
            context
                .window_worker_policy(page, &oryn_js::message_wire::WindowTarget::Root, 1, 0)
                .unwrap()
                .headers,
            before.headers
        );
        assert_eq!(
            context
                .inner
                .group
                .messages
                .lock()
                .unwrap()
                .root_document_identity(page.0),
            Some((1, 0))
        );
        context
            .install_document_security(page, "https://prerender.test/new".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                    document_generation: 99,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .commit_prerender_response_policy(page, &scope, "https://prerender.test/final", &[])
                .is_err()
        );
    }
}
