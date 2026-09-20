//! Parent authority for legacy DOM-domain relaxation. Strict origin records are
//! never rewritten. Preparation is read-only; commit validates the same native
//! document again and compares the previous domain before publishing access.
use super::*;
use crate::agent_registry::AgentOwner;
use oryn_js::{
    message_wire::WindowTarget,
    origin::{DomainAccessIdentity, Origin},
    worker_wire::Creator,
};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DocumentDomainPreparation {
    pub previous_domain: Option<String>,
    pub domain: String,
    pub access_token_key: String,
    pub ignored: bool,
    pub affected: Vec<(PageId, Creator)>,
}
#[derive(Clone)]
pub(super) struct DocumentState {
    pub identity: DomainAccessIdentity,
    pub origin_keyed: bool,
    pub group: u64,
}
#[derive(Default)]
pub(super) struct Registry {
    pub documents: BTreeMap<AgentOwner, DocumentState>,
    pub initial: BTreeMap<PageId, (Option<String>, Option<u64>)>,
    next_group: u64,
    clusters: BTreeMap<(crate::browsing_groups::GroupToken, String), bool>,
}
impl Registry {
    pub(super) fn discard_private_group(&mut self, group: crate::browsing_groups::GroupToken) {
        self.clusters.retain(|(owner, _), _| *owner != group);
    }
}
fn denied(message: impl std::fmt::Display) -> String {
    format!("SecurityError: {message}")
}
fn requested_origin_keying(headers: &[(String, String)]) -> bool {
    let mut values = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("origin-agent-cluster"));
    let Some((_, value)) = values.next() else {
        return false;
    };
    if values.next().is_some() {
        return false;
    }
    // Reuse the native Structured Fields parameter parser after recognizing the
    // boolean bare item, rather than accepting a prefix of a malformed field.
    let Some(parameters) = value.trim_start_matches(' ').strip_prefix("?1") else {
        return false;
    };
    crate::agent_isolation::structured_token(&format!("enabled{parameters}")) == Some("enabled")
}
impl BrowserContext {
    pub(crate) fn register_document_domain_cluster(
        &self,
        owner: &AgentOwner,
        secure: bool,
    ) -> Result<(), String> {
        let AgentOwner::Window { page, .. } = owner else {
            return Ok(());
        };
        let (_, origin, policy) = self.native_worker_authority(owner).map_err(denied)?;
        let group = match self.private_prerender_group(owner).map_err(denied)? {
            Some(group) => group,
            None => {
                self.inner
                    .group
                    .browsing_groups
                    .lock()
                    .map_err(|_| denied("group registry poisoned"))?
                    .snapshot(*page)
                    .ok_or_else(|| denied("document group missing"))?
                    .token
            }
        };
        let strict =
            Origin::from_broker_authority(&origin.serialized, &origin.authority).map_err(denied)?;
        let isolated = self.domain_policy_isolated(owner)?;
        let mut policies = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| denied("document policy registry poisoned"))?;
        let prerender = matches!(owner,AgentOwner::Window{page,generation,epoch,target:WindowTarget::Root}if policies.prerenders.contains_key(&(*page,*generation,*epoch)));
        let registry = &mut policies.document_domains;
        if let Some(existing) = registry.documents.get(owner) {
            return if existing.identity.origin() == &strict {
                Ok(())
            } else {
                Err(denied("document origin changed"))
            };
        }
        if registry.documents.len() >= 4096 || registry.clusters.len() >= 4096 {
            return Err("QuotaExceededError: domain authority registry".into());
        }
        let origin_keyed = origin.opaque
            || isolated
            || *registry
                .clusters
                .entry((group, origin.authority.clone()))
                .or_insert_with(|| secure && requested_origin_keying(&policy.headers));
        let inherited = if !prerender
            && matches!(
                owner,
                AgentOwner::Window {
                    target: WindowTarget::Root,
                    ..
                }
            ) {
            registry.initial.remove(page)
        } else {
            None
        };
        let mut identity = DomainAccessIdentity::new(strict);
        let group = if let Some(group) =
            inherited
                .as_ref()
                .and_then(|(_, group)| *group)
                .filter(|group| {
                    *group > 0
                        && *group <= registry.next_group
                        && !registry.documents.values().any(|state| {
                            state.group == *group && state.identity.origin() != identity.origin()
                        })
                }) {
            group
        } else {
            registry.next_group = registry
                .next_group
                .checked_add(1)
                .ok_or_else(|| denied("domain alias identity exhausted"))?;
            registry.next_group
        };
        let domain = registry
            .documents
            .values()
            .find(|state| state.group == group)
            .and_then(|state| state.identity.relaxed_domain().map(str::to_owned))
            .or_else(|| inherited.and_then(|(domain, _)| domain));
        if let Some(domain) = domain.filter(|_| !identity.origin().is_opaque()) {
            identity
                .set_domain(&domain)
                .map_err(|_| denied("invalid inherited document domain"))?;
        }
        registry.documents.insert(
            owner.clone(),
            DocumentState {
                identity,
                origin_keyed,
                group,
            },
        );
        Ok(())
    }
    fn document_domain_allowed(&self, owner: &AgentOwner) -> Result<bool, String> {
        let mut current = owner.clone();
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(current.clone()) || visited.len() > 256 {
                return Err(denied("invalid document ancestry"));
            }
            let (_, origin, policy) = self.native_worker_authority(&current).map_err(denied)?;
            if !crate::isolation_permissions::allows_feature(
                &policy.headers,
                "document-domain",
                &origin.serialized,
                &origin.serialized,
            ) {
                return Ok(false);
            }
            match &current {
                AgentOwner::Window {
                    target: WindowTarget::Root,
                    ..
                } => return Ok(true),
                AgentOwner::Window { .. } => {
                    let (parent, container) = {
                        let policies = self
                            .inner
                            .worker_policies
                            .lock()
                            .map_err(|_| denied("document ancestry poisoned"))?;
                        (
                            policies.activation_parents.get(&current).cloned(),
                            policies.isolation_containers.get(&current).cloned(),
                        )
                    };
                    let (Some(parent), Some((allow, source, _))) = (parent, container) else {
                        return Err(denied("document container missing"));
                    };
                    let (_, parent_origin, parent_policy) =
                        self.native_worker_authority(&parent).map_err(denied)?;
                    if !crate::isolation_permissions::allows_feature(
                        &parent_policy.headers,
                        "document-domain",
                        &parent_origin.serialized,
                        &origin.serialized,
                    ) {
                        return Ok(false);
                    }
                    // document-domain's default allowlist is *, unlike the
                    // secure-context features sharing the native parser.
                    let declared = allow.split(';').any(|part| {
                        part.split_ascii_whitespace().next() == Some("document-domain")
                    });
                    if declared
                        && !crate::isolation_permissions::container_allows_feature(
                            &allow,
                            "document-domain",
                            &parent_origin.serialized,
                            &source,
                            &origin.serialized,
                        )
                    {
                        return Ok(false);
                    }
                    current = parent;
                }
                _ => return Err(denied("document domain requires a Window")),
            }
        }
    }
    fn domain_owner(&self, page: PageId, creator: &Creator) -> Result<AgentOwner, String> {
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(denied("document domain requires a Window"));
        };
        self.window_worker_creator(page, target, *document_generation, *stream_epoch)
            .map(|value| value.0)
            .map_err(denied)
    }
    fn domain_proposal(
        &self,
        owner: &AgentOwner,
        candidate: &str,
    ) -> Result<(DocumentState, DocumentDomainPreparation), String> {
        let (_, origin, policy) = self.native_worker_authority(owner).map_err(denied)?;
        // Every active sandbox includes the document-domain flag; allow-same-
        // origin clears only the origin flag, not the domain flag.
        if crate::csp::CspPolicySet::from_agent_policy(&policy)
            .sandbox()
            .is_some()
        {
            return Err(denied("sandboxed document-domain flag"));
        }
        if origin.opaque {
            return Err(denied("opaque document origin"));
        }
        if !self.document_domain_allowed(owner)? {
            return Err(denied("document-domain Permissions Policy"));
        }
        let mut state = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| denied("document domain registry poisoned"))?
            .document_domains
            .documents
            .get(owner)
            .cloned()
            .ok_or_else(|| denied("document cluster not admitted"))?;
        let previous_domain = state.identity.relaxed_domain().map(str::to_owned);
        let isolated = self
            .inner
            .group
            .shared_memory
            .lock()
            .map_err(|_| denied("agent cluster registry poisoned"))?
            .isolated(owner);
        let ignored = state.origin_keyed || isolated;
        if !ignored {
            state
                .identity
                .set_domain(candidate)
                .map_err(|error| denied(format!("invalid domain candidate: {error:?}")))?;
        }
        let affected = self.domain_alias_recipients(state.group)?;
        let preparation = DocumentDomainPreparation {
            previous_domain,
            domain: state.identity.domain(),
            access_token_key: state.identity.security_token_key(),
            ignored,
            affected,
        };
        Ok((state, preparation))
    }
    pub(crate) fn prepare_document_domain(
        &self,
        page: PageId,
        creator: &Creator,
        candidate: &str,
    ) -> Result<DocumentDomainPreparation, String> {
        let owner = self.domain_owner(page, creator)?;
        let (_, preparation) = self.domain_proposal(&owner, candidate)?;
        self.domain_owner(page, creator)?;
        Ok(preparation)
    }
    pub(crate) fn commit_document_domain(
        &self,
        page: PageId,
        creator: &Creator,
        candidate: &str,
        expected_domain: Option<&str>,
        expected_affected: &[(PageId, Creator)],
    ) -> Result<DocumentDomainPreparation, String> {
        let owner = self.domain_owner(page, creator)?;
        let (state, preparation) = self.domain_proposal(&owner, candidate)?;
        self.domain_owner(page, creator)?;
        if preparation.affected != expected_affected {
            return Err(denied(
                "document domain alias recipients changed since preparation",
            ));
        }
        let pages = self
            .inner
            .group
            .pages
            .lock()
            .map_err(|_| denied("domain page registry poisoned"))?;
        let mut policies = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| denied("document domain registry poisoned"))?;
        let hub = self
            .inner
            .group
            .messages
            .lock()
            .map_err(|_| denied("domain document registry poisoned"))?;
        let live = hub.registered_documents();
        let affected = policies
            .document_domains
            .documents
            .iter()
            .filter(|(_, alias)| alias.group == state.group)
            .filter_map(|(owner, _)| {
                let AgentOwner::Window {
                    page,
                    target,
                    generation,
                    epoch,
                } = owner
                else {
                    return None;
                };
                if !pages.get(page).is_some_and(|record| record.is_live()) {
                    return None;
                }
                let admitted = if let Some((token, lifetime)) =
                    hub.prerender_scope_for_document(page.0, target, *generation, *epoch)
                {
                    policies
                        .prerenders
                        .get(&(*page, lifetime.0, lifetime.1))
                        .is_some_and(|scope| scope.token == token && !scope.canceling)
                        && hub
                            .prerender_target_authority(
                                page.0,
                                &token,
                                lifetime,
                                target,
                                *generation,
                                *epoch,
                            )
                            .is_ok()
                        && match target {
                            WindowTarget::Root => true,
                            WindowTarget::Frame(frame) => {
                                policies
                                    .frame_security_epochs
                                    .get(&(*page, frame.clone(), *generation, *epoch))
                                    .copied()
                                    == Some(lifetime.1)
                            }
                        }
                } else {
                    (match target {
                        WindowTarget::Root => policies
                            .root_lifetimes
                            .get(page)
                            .is_some_and(|life| life.current(*generation)),
                        WindowTarget::Frame(frame) => {
                            policies
                                .frame_security_epochs
                                .get(&(*page, frame.clone(), *generation, *epoch))
                                .copied()
                                == Some(
                                    policies
                                        .root_lifetimes
                                        .get(page)
                                        .map_or(0, |life| life.epoch),
                                )
                        }
                    }) && live.iter().any(|(p, t, g, e)| {
                        *p == page.0 && t == target && g == generation && e == epoch
                    })
                };
                admitted.then(|| {
                    (
                        *page,
                        Creator::Window {
                            target: target.clone(),
                            document_generation: *generation,
                            stream_epoch: *epoch,
                        },
                    )
                })
            })
            .collect::<Vec<_>>();
        if affected != expected_affected {
            return Err(denied(
                "document domain alias recipients changed before commit",
            ));
        }
        let current = policies
            .document_domains
            .documents
            .get_mut(&owner)
            .ok_or_else(|| denied("document retired before commit"))?;
        if current.identity.relaxed_domain() != expected_domain
            || preparation.previous_domain.as_deref() != expected_domain
        {
            return Err(denied("document domain changed since preparation"));
        }
        if !preparation.ignored {
            let group = state.group;
            for alias in policies
                .document_domains
                .documents
                .values_mut()
                .filter(|alias| alias.group == group)
            {
                alias.identity = state.identity.clone();
            }
            for (domain, alias_group) in policies.document_domains.initial.values_mut() {
                if *alias_group == Some(group) {
                    *domain = state.identity.relaxed_domain().map(str::to_owned);
                }
            }
        }
        Ok(preparation)
    }
    fn domain_alias_recipients(&self, group: u64) -> Result<Vec<(PageId, Creator)>, String> {
        let owners = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| denied("domain aliases poisoned"))?
            .document_domains
            .documents
            .iter()
            .filter(|(_, state)| state.group == group)
            .map(|(owner, _)| owner.clone())
            .collect::<Vec<_>>();
        Ok(owners
            .into_iter()
            .filter(|owner| self.native_worker_authority(owner).is_ok())
            .filter_map(|owner| match owner {
                AgentOwner::Window {
                    page,
                    target,
                    generation,
                    epoch,
                } => Some((
                    page,
                    Creator::Window {
                        target,
                        document_generation: generation,
                        stream_epoch: epoch,
                    },
                )),
                _ => None,
            })
            .collect())
    }
    pub(crate) fn inherit_document_domain(
        &self,
        parent: &AgentOwner,
        child: &AgentOwner,
    ) -> Result<(), String> {
        let (_, origin, _) = self.native_worker_authority(child).map_err(denied)?;
        let source = self.native_document_domain_identity(parent)?;
        let target =
            Origin::from_broker_authority(&origin.serialized, &origin.authority).map_err(denied)?;
        if source.origin() != &target {
            return Ok(());
        } // A sandbox creates a new opaque origin.
        let secure = self.worker_secure_context(parent).map_err(denied)?;
        self.register_document_domain_cluster(parent, secure)?;
        let mut policies = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| denied("domain aliases poisoned"))?;
        let source = policies
            .document_domains
            .documents
            .get(parent)
            .cloned()
            .ok_or_else(|| denied("domain alias parent missing"))?;
        policies
            .document_domains
            .documents
            .entry(child.clone())
            .or_insert(source);
        Ok(())
    }
    fn domain_policy_isolated(&self, owner: &AgentOwner) -> Result<bool, String> {
        let mut current = owner.clone();
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(current.clone()) || seen.len() > 256 {
                return Err(denied("invalid isolation ancestry"));
            }
            let (_, origin, policy) = self.native_worker_authority(&current).map_err(denied)?;
            if origin.opaque
                || !self.worker_secure_context(&current).map_err(denied)?
                || !crate::isolation_permissions::allows(
                    &policy.headers,
                    &origin.serialized,
                    &origin.serialized,
                )
            {
                return Ok(false);
            }
            let isolation =
                crate::agent_isolation::DocumentIsolationPolicy::from_headers(&policy.headers);
            match &current {
                AgentOwner::Window {
                    target: WindowTarget::Root,
                    ..
                } => return Ok(isolation.top_level_isolated(true, true)),
                AgentOwner::Window { .. } => {
                    if !isolation.embedded_isolated(true, true, true) {
                        return Ok(false);
                    }
                    let (parent, container) = {
                        let policies = self
                            .inner
                            .worker_policies
                            .lock()
                            .map_err(|_| denied("isolation ancestry poisoned"))?;
                        (
                            policies.activation_parents.get(&current).cloned(),
                            policies.isolation_containers.get(&current).cloned(),
                        )
                    };
                    let (Some(parent), Some((allow, source, _))) = (parent, container) else {
                        return Ok(false);
                    };
                    let (_, parent_origin, parent_policy) =
                        self.native_worker_authority(&parent).map_err(denied)?;
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
                    current = parent;
                }
                _ => return Ok(false),
            }
        }
    }
    pub(crate) fn document_origin_agent_cluster(
        &self,
        page: PageId,
        creator: &Creator,
    ) -> Result<bool, String> {
        let owner = self.domain_owner(page, creator)?;
        let (_, origin, _) = self.native_worker_authority(&owner).map_err(denied)?;
        if origin.opaque {
            return Ok(true);
        }
        if self
            .inner
            .group
            .shared_memory
            .lock()
            .map_err(|_| denied("agent cluster registry poisoned"))?
            .isolated(&owner)
        {
            return Ok(true);
        }
        self.inner
            .worker_policies
            .lock()
            .map_err(|_| denied("document cluster registry poisoned"))?
            .document_domains
            .documents
            .get(&owner)
            .map(|state| state.origin_keyed)
            .ok_or_else(|| denied("document agent cluster not yet admitted"))
    }
    pub(crate) fn document_domain_identity(
        &self,
        page: PageId,
        creator: &Creator,
    ) -> Result<DomainAccessIdentity, String> {
        let owner = self.domain_owner(page, creator)?;
        self.native_document_domain_identity(&owner)
    }
    pub(crate) fn document_domain_alias_key(
        &self,
        page: PageId,
        creator: &Creator,
    ) -> Result<String, String> {
        let owner = self.domain_owner(page, creator)?;
        let policies = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| denied("document domain registry poisoned"))?;
        let state = policies
            .document_domains
            .documents
            .get(&owner)
            .ok_or_else(|| denied("document cluster not admitted"))?;
        Ok(format!("{}:{}", self.id(), state.group))
    }
    pub(crate) fn native_document_domain_identity(
        &self,
        owner: &AgentOwner,
    ) -> Result<DomainAccessIdentity, String> {
        let (_, origin, _) = self.native_worker_authority(owner).map_err(denied)?;
        if let Some(identity) = self
            .inner
            .worker_policies
            .lock()
            .map_err(|_| denied("document domain registry poisoned"))?
            .document_domains
            .documents
            .get(owner)
            .map(|state| state.identity.clone())
        {
            return Ok(identity);
        }
        // A committed document can be a navigation target before its bindings
        // publish secure-context/cluster state. No setter has run in that case.
        let mut identity = DomainAccessIdentity::new(
            Origin::from_broker_authority(&origin.serialized, &origin.authority).map_err(denied)?,
        );
        if let AgentOwner::Window {
            page,
            target: WindowTarget::Root,
            ..
        } = owner
        {
            let policies = self
                .inner
                .worker_policies
                .lock()
                .map_err(|_| denied("initial document domain poisoned"))?;
            if matches!(owner,AgentOwner::Window{page,generation,epoch,target:WindowTarget::Root}if policies.prerenders.contains_key(&(*page,*generation,*epoch)))
            {
                return Ok(identity);
            }
            if let Some((domain, group)) = policies.document_domains.initial.get(page) {
                let domain = group
                    .and_then(|group| {
                        policies
                            .document_domains
                            .documents
                            .values()
                            .find(|state| {
                                state.group == group && state.identity.origin() == identity.origin()
                            })
                            .and_then(|state| state.identity.relaxed_domain().map(str::to_owned))
                    })
                    .or_else(|| domain.clone());
                if let Some(domain) = domain.filter(|_| !identity.origin().is_opaque()) {
                    identity
                        .set_domain(&domain)
                        .map_err(|_| denied("invalid initial document domain"))?;
                }
            }
        }
        Ok(identity)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn commit(
        context: &BrowserContext,
        page: PageId,
        creator: &Creator,
        candidate: &str,
        expected: Option<&str>,
    ) -> Result<DocumentDomainPreparation, String> {
        let affected = context
            .prepare_document_domain(page, creator, candidate)?
            .affected;
        context.commit_document_domain(page, creator, candidate, expected, &affected)
    }
    use oryn_js::message_wire::MessageRequest;
    fn document(headers: Vec<(String, String)>) -> (BrowserContext, PageId, Creator, AgentOwner) {
        let context = Browser::in_process_with_config(Default::default()).new_context();
        let page = context.new_page().page_id();
        context
            .commit_page_url(page, "https://www.example.test/".into())
            .unwrap();
        context
            .install_document_security(page, "https://www.example.test/".into(), &headers)
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
        let owner = context
            .window_worker_creator(page, &WindowTarget::Root, 1, 0)
            .unwrap()
            .0;
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        (
            context,
            page,
            Creator::Window {
                target: WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 0,
            },
            owner,
        )
    }
    #[test]
    fn prepare_does_not_mutate_and_commit_rechecks_previous_domain() {
        let (context, page, creator, owner) = document(vec![]);
        let strict = context
            .window_worker_creator(page, &WindowTarget::Root, 1, 0)
            .unwrap()
            .2;
        let plan = context
            .prepare_document_domain(page, &creator, "EXAMPLE.TEST")
            .unwrap();
        assert_eq!(plan.domain, "example.test");
        assert!(!plan.ignored);
        assert_eq!(
            context
                .native_document_domain_identity(&owner)
                .unwrap()
                .relaxed_domain(),
            None
        );
        let committed = commit(&context, page, &creator, "EXAMPLE.TEST", None).unwrap();
        assert_eq!(committed, plan);
        assert!(commit(&context, page, &creator, "example.test", None).is_err());
        assert!(
            context
                .prepare_document_domain(page, &creator, "www.example.test")
                .is_err()
        );
        assert_eq!(
            context
                .window_worker_creator(page, &WindowTarget::Root, 1, 0)
                .unwrap()
                .2
                .authority,
            strict.authority
        );
        // Re-publication for the same Document (including parser replacement)
        // does not recreate its domain or cluster choice.
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        assert_eq!(
            context
                .native_document_domain_identity(&owner)
                .unwrap()
                .domain(),
            "example.test"
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
            commit(
                &context,
                page,
                &creator,
                "example.test",
                Some("example.test")
            )
            .is_err()
        );
        assert!(context.native_document_domain_identity(&owner).is_err());
    }
    #[test]
    fn sandbox_and_permissions_deny_before_candidate_or_oac_noop() {
        for header in [
            (
                "Content-Security-Policy",
                "sandbox allow-scripts allow-same-origin",
            ),
            ("Permissions-Policy", "document-domain=()"),
        ] {
            let (context, page, creator, _) = document(vec![
                (header.0.into(), header.1.into()),
                ("Origin-Agent-Cluster".into(), "?1".into()),
            ]);
            assert!(
                context
                    .prepare_document_domain(page, &creator, "example.test")
                    .unwrap_err()
                    .starts_with("SecurityError:")
            );
        }
    }
    #[test]
    fn origin_keyed_cluster_ignores_assignment_without_parsing_candidate() {
        let (context, page, creator, owner) = document(vec![(
            "Origin-Agent-Cluster".into(),
            "?1; report-to=\"group\"".into(),
        )]);
        let plan = context
            .prepare_document_domain(page, &creator, "not a host")
            .unwrap();
        assert!(plan.ignored);
        assert_eq!(plan.domain, "www.example.test");
        assert_eq!(
            commit(&context, page, &creator, "not a host", None).unwrap(),
            plan
        );
        assert_eq!(
            context
                .native_document_domain_identity(&owner)
                .unwrap()
                .relaxed_domain(),
            None
        );
    }
    #[test]
    fn bilateral_domain_relaxation_controls_native_ancestor_navigation() {
        let (context, page, root_creator, root_owner) = document(vec![]);
        let token = "d".repeat(64);
        let url = "https://sub.example.test/frame";
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterWindow {
                    frame: token.clone(),
                    document_generation: 1,
                    stream_epoch: 0,
                    origin: "https://sub.example.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        let ticket = context
            .prepare_frame_worker_policy(page, url, &[], "")
            .unwrap();
        context
            .bind_frame_worker_policy(page, token.clone(), 1, 0, url, &ticket)
            .unwrap();
        context
            .register_activation_parent(crate::runtime::worker_service::ActivationParentInputs {
                page,
                frame: token.clone(),
                generation: 1,
                epoch: 0,
                parent: WindowTarget::Root,
                parent_generation: 1,
                parent_epoch: 0,
            })
            .unwrap();
        context
            .register_isolation_container(
                crate::runtime::worker_service::IsolationContainerInputs {
                    page,
                    frame: token.clone(),
                    generation: 1,
                    epoch: 0,
                    allow: String::new(),
                    source: url.into(),
                    sandbox: None,
                },
            )
            .unwrap();
        let owner = context
            .window_worker_creator(page, &WindowTarget::Frame(token.clone()), 1, 0)
            .unwrap()
            .0;
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        let child = Creator::Window {
            target: WindowTarget::Frame(token),
            document_generation: 1,
            stream_epoch: 0,
        };
        assert!(
            !context
                .native_target_navigation_allowed(page, &child, page, &root_creator)
                .unwrap()
        );
        assert_eq!(
            context
                .read_related_window(page, &child, page, &WindowTarget::Root, None)
                .unwrap()
                .url,
            None
        );
        commit(&context, page, &child, "example.test", None).unwrap();
        assert!(
            !context
                .native_target_navigation_allowed(page, &child, page, &root_creator)
                .unwrap()
        );
        commit(&context, page, &root_creator, "example.test", None).unwrap();
        assert!(
            context
                .native_target_navigation_allowed(page, &child, page, &root_creator)
                .unwrap()
        );
        assert_eq!(
            context
                .read_related_window(page, &child, page, &WindowTarget::Root, None)
                .unwrap()
                .url,
            Some("https://www.example.test/".into())
        );
        assert!(
            context
                .native_document_domain_identity(&owner)
                .unwrap()
                .same_origin_domain(
                    &context
                        .native_document_domain_identity(&root_owner)
                        .unwrap()
                )
        );
    }
    #[test]
    fn inherited_blank_and_srcdoc_share_domain_and_prepare_checks_alias_roster() {
        let (context, page, creator, root) = document(vec![]);
        let stale = context
            .prepare_document_domain(page, &creator, "example.test")
            .unwrap();
        let mut aliases = Vec::new();
        for (letter, url) in [("a", "about:blank"), ("b", "about:srcdoc")] {
            let frame = letter.repeat(64);
            context
                .handle_message_wire(
                    page,
                    MessageRequest::RegisterWindow {
                        frame: frame.clone(),
                        document_generation: 2,
                        stream_epoch: 0,
                        origin: "https://www.example.test".into(),
                        inherit_origin: Some(WindowTarget::Root),
                    },
                )
                .unwrap();
            context
                .inherit_frame_worker_policy_at_url(
                    crate::runtime::worker_service::InheritedFramePolicyInputs {
                        page,
                        frame: frame.clone(),
                        generation: 2,
                        epoch: 0,
                        parent: &WindowTarget::Root,
                        parent_generation: 1,
                        parent_epoch: 0,
                        document_url: url,
                    },
                )
                .unwrap();
            context
                .register_isolation_container(
                    crate::runtime::worker_service::IsolationContainerInputs {
                        page,
                        frame: frame.clone(),
                        generation: 2,
                        epoch: 0,
                        allow: String::new(),
                        source: url.into(),
                        sandbox: None,
                    },
                )
                .unwrap();
            let owner = context
                .window_worker_creator(page, &WindowTarget::Frame(frame.clone()), 2, 0)
                .unwrap()
                .0;
            context
                .publish_worker_secure_context(owner.clone(), true)
                .unwrap();
            aliases.push((
                Creator::Window {
                    target: WindowTarget::Frame(frame),
                    document_generation: 2,
                    stream_epoch: 0,
                },
                owner,
            ));
        }
        assert!(
            context
                .commit_document_domain(page, &creator, "example.test", None, &stale.affected)
                .is_err()
        );
        assert_eq!(
            context
                .native_document_domain_identity(&root)
                .unwrap()
                .relaxed_domain(),
            None
        );
        let changed = commit(&context, page, &aliases[0].0, "example.test", None).unwrap();
        assert_eq!(changed.affected.len(), 3);
        for owner in [&root, &aliases[0].1, &aliases[1].1] {
            assert_eq!(
                context
                    .native_document_domain_identity(owner)
                    .unwrap()
                    .relaxed_domain(),
                Some("example.test")
            );
        }
    }
    #[test]
    fn committed_root_blank_navigation_retains_initiator_domain() {
        let (context, page, creator, _) = document(vec![]);
        commit(&context, page, &creator, "example.test", None).unwrap();
        let (token, _, _, _, _, _) = context
            .prepare_blank_navigation(
                page,
                creator,
                "about:blank",
                Some("https://www.example.test/"),
            )
            .unwrap();
        context
            .commit_prepared_local_navigation(page, &token)
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let owner = context
            .window_worker_creator(page, &WindowTarget::Root, 2, 0)
            .unwrap()
            .0;
        context
            .publish_worker_secure_context(owner.clone(), true)
            .unwrap();
        assert_eq!(
            context
                .native_document_domain_identity(&owner)
                .unwrap()
                .relaxed_domain(),
            Some("example.test")
        );
        assert_eq!(
            context
                .native_document_domain_identity(&owner)
                .unwrap()
                .origin()
                .serialize(),
            "https://www.example.test"
        );
    }
    #[test]
    fn signed_document_source_keeps_domain_after_retirement_without_changing_origin() {
        let (context, page, creator, _) = document(vec![]);
        commit(&context, page, &creator, "example.test", None).unwrap();
        let source = context
            .capture_document_source(page, creator, "https://www.example.test/", "no-referrer")
            .unwrap();
        assert_eq!(source.origin, "https://www.example.test");
        assert_ne!(
            source.origin_key,
            context
                .document_source_domain_identity(page, &source)
                .unwrap()
                .security_token_key()
        );
        let inert = context
            .derive_inert_document_source(page, &source, "about:blank")
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert_eq!(
            context
                .document_source_domain_identity(page, &source)
                .unwrap()
                .relaxed_domain(),
            Some("example.test")
        );
        assert_eq!(
            context
                .document_source_domain_identity(page, &inert)
                .unwrap()
                .relaxed_domain(),
            Some("example.test")
        );
        let mut forged = source.clone();
        forged.origin = "https://other.test".into();
        assert!(
            context
                .document_source_domain_identity(page, &forged)
                .is_err()
        );
    }
    #[test]
    fn origin_agent_cluster_reads_admitted_choice_and_isolation_without_binding_shared_memory() {
        let (context, page, creator, _) =
            document(vec![("Origin-Agent-Cluster".into(), "?1".into())]);
        assert!(
            context
                .document_origin_agent_cluster(page, &creator)
                .unwrap()
        );
        let (context, page, creator, _) = document(vec![
            ("Cross-Origin-Opener-Policy".into(), "same-origin".into()),
            ("Cross-Origin-Embedder-Policy".into(), "require-corp".into()),
        ]);
        assert!(
            context
                .document_origin_agent_cluster(page, &creator)
                .unwrap()
        );
        let (context, page, creator, _) = document(vec![]);
        assert!(
            !context
                .document_origin_agent_cluster(page, &creator)
                .unwrap()
        );
        context
            .install_document_security(
                page,
                "https://www.example.test/next".into(),
                &[("Origin-Agent-Cluster".into(), "?1".into())],
            )
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let owner = context
            .window_worker_creator(page, &WindowTarget::Root, 2, 0)
            .unwrap()
            .0;
        context.publish_worker_secure_context(owner, true).unwrap();
        assert!(
            context
                .document_origin_agent_cluster(page, &creator)
                .is_err()
        );
        let next = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 2,
            stream_epoch: 0,
        };
        assert!(
            !context.document_origin_agent_cluster(page, &next).unwrap(),
            "same group/origin retains first admitted choice"
        );
    }
    #[test]
    fn origin_keying_parses_complete_structured_boolean_and_ignores_report_only() {
        let header = |value: &str| vec![("Origin-Agent-Cluster".into(), value.into())];
        for value in ["?1", " ?1 ", "?1;foo", "?1;foo=?0"] {
            assert!(requested_origin_keying(&header(value)), "{value}");
        }
        for value in [
            "?0",
            "?10",
            "true",
            "?1, ?1",
            "?1;=bad",
            "?1;foo=\"unterminated",
        ] {
            assert!(!requested_origin_keying(&header(value)), "{value}");
        }
        assert!(!requested_origin_keying(&[(
            "Origin-Agent-Cluster-Report-Only".into(),
            "?1".into()
        )]));
        assert!(!requested_origin_keying(&[
            ("Origin-Agent-Cluster".into(), "?1".into()),
            ("Origin-Agent-Cluster".into(), "?1".into())
        ]));
    }
}
