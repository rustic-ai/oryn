//! Native keyword selection shares the authenticated creator ancestry, while
//! ordinary names continue through the case-sensitive familiar-context search.
use super::*;
impl BrowserContext {
    pub(super) fn keyword_navigation_target(
        &self,
        page: PageId,
        creator: &Creator,
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
        use oryn_js::message_wire::WindowTarget;
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "keyword navigation requires native Window".into(),
            ));
        };
        let (mut owner, _, _) =
            self.window_worker_creator(page, target, *document_generation, *stream_epoch)?;
        let keyword = name.to_ascii_lowercase();
        if keyword != "_self" {
            let parents = self
                .inner
                .worker_policies
                .lock()
                .map_err(|_| {
                    RuntimeError::ContextService("keyword navigation ancestry poisoned".into())
                })?
                .activation_parents
                .clone();
            let mut seen = BTreeSet::new();
            while let Some(parent) = parents.get(&owner) {
                if !seen.insert(owner.clone()) || seen.len() > 4096 {
                    return Err(RuntimeError::ProtocolViolation(
                        "keyword navigation ancestry cycle".into(),
                    ));
                }
                owner = parent.clone();
                if keyword == "_parent" {
                    break;
                }
            }
        }
        let AgentOwner::Window {
            page: selected_page,
            target,
            generation,
            epoch,
        } = owner
        else {
            return Err(RuntimeError::ProtocolViolation(
                "keyword target is not a Window".into(),
            ));
        };
        self.window_worker_creator(selected_page, &target, generation, epoch)?;
        let frame = match target {
            WindowTarget::Root => None,
            WindowTarget::Frame(token) => Some(
                local
                    .get(&selected_page)
                    .and_then(|inventory| {
                        inventory
                            .iter()
                            .find(|(_, snapshot)| {
                                snapshot.live
                                    && snapshot.frame_token.as_deref() == Some(token.as_str())
                                    && snapshot.document_generation == generation
                                    && snapshot.document_authority_epoch == epoch
                            })
                            .map(|(_, snapshot)| snapshot.clone())
                    })
                    .ok_or_else(|| {
                        RuntimeError::ProtocolViolation(
                            "keyword target native frame snapshot unavailable".into(),
                        )
                    })?,
            ),
        };
        Ok(Some((selected_page, frame)))
    }
}

impl BrowserContext {
    pub(super) fn preferred_local_named_target(
        &self,
        page: PageId,
        creator: &Creator,
        source: Option<&oryn_dom::DocumentProvenance>,
        identity: &oryn_js::origin::DomainAccessIdentity,
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
        use oryn_js::message_wire::WindowTarget;
        let Some(inventory) = local.get(&page) else {
            return Ok(None);
        };
        let Creator::Window { target, .. } = creator else {
            return Err(RuntimeError::ProtocolViolation(
                "named source is not a Window".into(),
            ));
        };
        let current = inventory
            .iter()
            .find(|(_, snapshot)| {
                snapshot.live
                    && match target {
                        WindowTarget::Root => snapshot.parent_realm.is_none(),
                        WindowTarget::Frame(token) => snapshot.frame_token.as_ref() == Some(token),
                    }
            })
            .map(|(_, snapshot)| snapshot.realm);
        let parents = inventory
            .iter()
            .map(|(_, snapshot)| (snapshot.realm, snapshot.parent_realm))
            .collect::<BTreeMap<_, _>>();
        let descendants = |realm| {
            let mut next = Some(realm);
            let mut seen = BTreeSet::new();
            while let Some(realm) = next {
                if Some(realm) == current {
                    return true;
                }
                if !seen.insert(realm) {
                    return false;
                }
                next = parents.get(&realm).copied().flatten();
            }
            false
        };
        let mut seen = BTreeSet::new();
        for subtree_only in [true, false] {
            for (target_name, snapshot) in inventory {
                if !snapshot.live
                    || subtree_only && !descendants(snapshot.realm)
                    || !seen.insert(snapshot.realm)
                    || target_name != name
                {
                    continue;
                }
                let target = match snapshot.frame_token.as_ref() {
                    Some(token) => WindowTarget::Frame(token.clone()),
                    None if snapshot.parent_realm.is_none() => WindowTarget::Root,
                    None => continue,
                };
                let (owner, _, _) = self.window_worker_creator(
                    page,
                    &target,
                    snapshot.document_generation,
                    snapshot.document_authority_epoch,
                )?;
                if !self.named_frame_familiar_from_domain(identity, &owner)? {
                    continue;
                }
                let destination = Creator::Window {
                    target,
                    document_generation: snapshot.document_generation,
                    stream_epoch: snapshot.document_authority_epoch,
                };
                let allowed = if let Some(source) = source {
                    self.document_target_navigation_allowed(page, source, &destination)?
                } else {
                    self.native_target_navigation_allowed(page, creator, page, &destination)?
                };
                if allowed {
                    return Ok(Some((
                        page,
                        snapshot.parent_realm.is_some().then(|| snapshot.clone()),
                    )));
                }
            }
        }
        Ok(None)
    }
}

impl BrowserContext {
    pub(crate) fn set_named_frame_opener(
        &self,
        source_page: PageId,
        source: &Creator,
        target_page: PageId,
        target: &Creator,
    ) -> Result<(), RuntimeError> {
        use oryn_js::message_wire::WindowTarget;
        let Creator::Window {
            target: source_target,
            document_generation,
            stream_epoch,
        } = source
        else {
            return Err(RuntimeError::ProtocolViolation(
                "opener requires native Window".into(),
            ));
        };
        self.window_worker_creator(
            source_page,
            source_target,
            *document_generation,
            *stream_epoch,
        )?;
        let Creator::Window {
            target: WindowTarget::Frame(token),
            document_generation,
            stream_epoch,
        } = target
        else {
            return Err(RuntimeError::ProtocolViolation(
                "named frame opener requires native frame".into(),
            ));
        };
        self.window_worker_creator(
            target_page,
            &WindowTarget::Frame(token.clone()),
            *document_generation,
            *stream_epoch,
        )?;
        let mut policies =
            self.inner.worker_policies.lock().map_err(|_| {
                RuntimeError::ContextService("frame opener registry poisoned".into())
            })?;
        let key = (target_page, token.clone());
        if !policies.frame_openers.contains_key(&key) && policies.frame_openers.len() >= 4096 {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "frame opener relationships".into(),
            });
        }
        policies
            .frame_openers
            .insert(key, (source_page, source_target.clone()));
        Ok(())
    }
    pub(crate) fn discard_native_frame_opener(
        &self,
        page: PageId,
        token: &str,
    ) -> Result<(), RuntimeError> {
        self.inner.worker_policies.lock().map_err(|_|RuntimeError::ContextService("frame opener cleanup poisoned".into()))?.frame_openers.retain(|(target_page,target_token),(source_page,source_target)|
            !(*target_page==page&&target_token==token)&&!(*source_page==page&&matches!(source_target,oryn_js::message_wire::WindowTarget::Frame(source_token)if source_token==token)));
        Ok(())
    }
    pub(crate) fn native_browsing_context_opener(
        &self,
        page: PageId,
        creator: &Creator,
    ) -> Result<Option<(PageId, oryn_js::message_wire::WindowTarget)>, RuntimeError> {
        use oryn_js::message_wire::WindowTarget;
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "opener requires native Window".into(),
            ));
        };
        self.window_worker_creator(page, target, *document_generation, *stream_epoch)?;
        match target {
            WindowTarget::Root => self
                .native_window_group(page, page, None)?
                .1
                .map(|opener| {
                    self.native_opener_target(page)
                        .map(|target| (opener, target))
                })
                .transpose(),
            WindowTarget::Frame(token) => Ok(self
                .inner
                .worker_policies
                .lock()
                .map_err(|_| RuntimeError::ContextService("frame opener registry poisoned".into()))?
                .frame_openers
                .get(&(page, token.clone()))
                .cloned()),
        }
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::*;
    #[test]
    fn frame_destruction_cleans_both_directions_once_without_touching_other_frames() {
        use oryn_js::message_wire::WindowTarget;
        let context = Browser::in_process_with_config(Default::default()).new_context();
        let page = context.new_page().page_id();
        let other = context.new_page().page_id();
        {
            let mut policies = context.inner.worker_policies.lock().unwrap();
            policies
                .frame_openers
                .insert((page, "removed".into()), (other, WindowTarget::Root));
            policies.frame_openers.insert(
                (other, "dependent".into()),
                (page, WindowTarget::Frame("removed".into())),
            );
            policies
                .frame_openers
                .insert((page, "kept".into()), (other, WindowTarget::Root));
        }
        context
            .discard_native_frame_opener(page, "removed")
            .unwrap();
        context
            .discard_native_frame_opener(page, "removed")
            .unwrap();
        let policies = context.inner.worker_policies.lock().unwrap();
        assert_eq!(policies.frame_openers.len(), 1);
        assert_eq!(
            policies.frame_openers[&(page, "kept".into())],
            (other, WindowTarget::Root)
        );
    }
}
