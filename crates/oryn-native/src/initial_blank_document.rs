//! Materialize the real initial root before layout or script observation.
use super::*;

impl PageState {
    #[cfg(feature = "v8-host")]
    pub(super) fn ensure_initial_blank_document(
        &mut self,
        context: &BrowserContext,
    ) -> Result<(), RuntimeError> {
        if self.failed_install_recovery.is_some() || self.page_task_terminal_control.retired() {
            return Err(RuntimeError::EvaluationUnavailable);
        }
        if self.document.is_some() && self.executable.is_some() {
            return Ok(());
        }
        // Never replace a failed/pending navigated root with fresh authority.
        if self.document.is_some()
            || self.executable.is_some()
            || !self.url.is_empty()
            || self.active_navigation.is_some()
            || self.pending_navigation_response.is_some()
            || self.history_index.is_some()
            || !self.history.is_empty()
        {
            return Err(RuntimeError::LayoutUnavailable(
                "page has no committed document".into(),
            ));
        }
        #[cfg(unix)]
        let (initial_creator, initial_policy, initial_origin, initial_isolation) = {
            let creator = context.native_initial_document_creator(self.page_id)?;
            let oryn_js::worker_wire::Creator::Window {
                target,
                document_generation,
                stream_epoch,
            } = &creator
            else {
                return Err(RuntimeError::ProtocolViolation(
                    "initial creator is not a Window".into(),
                ));
            };
            let (_, _, origin) = context.window_worker_creator(
                self.page_id,
                target,
                *document_generation,
                *stream_epoch,
            )?;
            let policy = context.window_worker_policy(
                self.page_id,
                target,
                *document_generation,
                *stream_epoch,
            )?;
            let isolation = context
                .inner
                .worker_policies
                .lock()
                .map_err(|_| RuntimeError::ContextService("initial policies poisoned".into()))?
                .root_isolation
                .get(&self.page_id)
                .copied()
                .unwrap_or_default();
            (creator, policy, origin, isolation)
        };
        #[cfg(unix)]
        let referrer =
            crate::referrer::ReferrerPolicy::default().after_response(&initial_policy.headers);
        #[cfg(not(unix))]
        let referrer = self.referrer_policy;
        let mut resources = BTreeMap::new();
        resources.insert(
            "oryn:native-scrollbar-width".into(),
            self.config.native_scrollbar_width.to_string(),
        );
        resources.insert("oryn:referrer-policy".into(), referrer.as_str().into());
        #[cfg(unix)]
        if let Some(base) = initial_policy.policy_self_url.as_ref() {
            resources.insert("oryn:inherited-navigation-base".into(), base.clone());
        }
        let mut executable = crate::executable::RustExecutableDocument::load_for_page(
            crate::executable::PageDocumentLoadInput {
                parsed: Some(crate::html::parse(
                    "<!doctype html><html><head></head><body></body></html>",
                )),
                source: None,
                html: "",
                resources: &resources,
                broker: self.broker.clone(),
                base_url: Url::parse("about:blank").ok(),
                max_heap_bytes: self.config.max_v8_heap_bytes,
                scheduled: false,
                #[cfg(unix)]
                context: self
                    .context_client
                    .clone()
                    .map(|client| (client, self.page_id, self.opener_page_id)),
                recovery: None,
                inherited_origin: None,
                terminal_control: self.page_task_terminal_control.clone(),
            },
        )
        .map_err(map_host_error)?;
        let provisional_agent_root = Some(executable.agent_root_realm());
        let prepared = (|| {
            let constructed_creator = executable
                .native_navigation_creator()
                .map_err(map_host_error)?;
            let oryn_js::worker_wire::Creator::Window {
                target,
                document_generation,
                stream_epoch,
            } = &constructed_creator
            else {
                return Err(RuntimeError::ProtocolViolation(
                    "initial root has no Window identity".into(),
                ));
            };
            #[cfg(unix)]
            {
                let (_, _, origin) = context.window_worker_creator(
                    self.page_id,
                    target,
                    *document_generation,
                    *stream_epoch,
                )?;
                let policy = context.window_worker_policy(
                    self.page_id,
                    target,
                    *document_generation,
                    *stream_epoch,
                )?;
                let oryn_js::worker_wire::Creator::Window {
                    target: initial_target,
                    document_generation: initial_generation,
                    stream_epoch: initial_epoch,
                } = &initial_creator
                else {
                    unreachable!()
                };
                if target != initial_target
                    || *document_generation < *initial_generation
                    || stream_epoch != initial_epoch
                    || policy != initial_policy
                    || origin != initial_origin
                    || policy.url != "about:blank"
                    || *stream_epoch != 0
                {
                    return Err(RuntimeError::ProtocolViolation(
                        "initial policy changed during native construction".into(),
                    ));
                }
            }
            let document = executable.semantic_snapshot().map_err(map_host_error)?;
            // Adopt the native initial entry before processing its retained
            // update. No navigation, new identity, or epoch is fabricated.
            let receipt = executable.peek_navigation_history_update()
                .ok_or(RuntimeError::HistoryBoundary)?;
            let update = receipt.update();
            if update.current != 0 || update.entries.len() != 1 {
                return Err(RuntimeError::HistoryBoundary);
            }
            let snapshot = &update.entries[0];
            if snapshot.url != "about:blank" || snapshot.key.is_empty()
                || snapshot.id.is_empty() || snapshot.document_id.is_empty()
                || !snapshot.scroll.valid() {
                return Err(RuntimeError::ProtocolViolation("invalid initial native history".into()));
            }
            let mut history = Vec::new();
            history.try_reserve_exact(1).map_err(|_|RuntimeError::WorkerLimitExceeded {
                resource: "initial native history allocation".into(),
            })?;
            let mut entry: NavigationHistoryEntry = snapshot.url.clone().into();
            #[cfg(unix)]
            { entry.origin = initial_origin.clone(); }
            entry.navigation = Some(snapshot.clone());
            history.push(entry);
            #[cfg(unix)]
            if context
                .inner
                .worker_policies
                .lock()
                .map_err(|_| RuntimeError::ContextService("initial policies poisoned".into()))?
                .root_isolation
                .get(&self.page_id)
                .copied()
                .unwrap_or_default()
                != initial_isolation
            {
                return Err(RuntimeError::ProtocolViolation(
                    "initial isolation changed during construction".into(),
                ));
            }
            #[cfg(not(unix))]
            let _ = (target, stream_epoch);
            if self.page_task_terminal_control.retired() {
                return Err(RuntimeError::EvaluationUnavailable);
            }
            let controller = self
                .page_task_terminal_control
                .controller()
                .cloned()
                .ok_or(RuntimeError::PageTaskYield)?;
            Ok((*document_generation, document, controller, history))
        })();
        let (generation, document, controller, history) = match prepared {
            Ok(value) => value,
            Err(error) => {
                // Retire native/root authority before dropping a failed draft.
                // Keep custody if retirement itself cannot be acknowledged.
                self.failed_install_recovery = Some(FailedInstallRecovery {
                    executable: Some(executable),
                    failed_realm: provisional_agent_root,
                    failed_authority_retired: false,
                    #[cfg(unix)]
                    failed_authority_requests: None,
                    source: None,
                    source_published: false,
                    discard_cache: None,
                    retry: None,
                });
                self.poll_failed_install_recovery();
                return Err(error);
            }
        };
        // All fallible preparation precedes publishing the initial native root.
        // The first normal semantic sync adopts native initial history/control
        // state; it does not fabricate a navigation commit or increment epoch.
        let url = String::from("about:blank");
        let graph = crate::semantic::SemanticGraph::new(self.page_id, generation);
        #[cfg(unix)]
        let csp = crate::csp::CspPolicySet::from_agent_policy(&initial_policy);
        let mut draft = Some((executable, document, graph, url));
        let admitted = controller.with_live_publication(|| {
            let (executable, document, graph, url) = draft.take().expect("initial draft retained");
            self.url = url;
            self.document_generation = generation;
            self.history = history;
            self.history_index = Some(0);
            self.referrer_policy = referrer;
            #[cfg(unix)]
            {
                self.csp = csp;
                self.response_policy_headers = initial_policy.headers;
                self.script_isolation = initial_isolation;
            }
            self.last_issued_document_generation =
                self.last_issued_document_generation.max(generation);
            self.semantic_graph = graph;
            self.document = Some(document);
            self.executable = Some(executable);
            self.semantic_publication_pending = true;
        });
        if admitted.is_err() {
            let (executable, _, _, _) = draft.take().expect("unpublished initial draft retained");
            self.failed_install_recovery = Some(FailedInstallRecovery {
                executable: Some(executable),
                failed_realm: provisional_agent_root,
                failed_authority_retired: false,
                #[cfg(unix)]
                failed_authority_requests: None,
                source: None,
                source_published: false,
                discard_cache: None,
                retry: None,
            });
            self.poll_failed_install_recovery();
            return Err(RuntimeError::PageTaskYield);
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "v8-host"))]
mod initial_blank_controls {
    use super::*;
    #[tokio::test(flavor = "current_thread")]
    async fn first_observation_materializes_native_blank_without_navigation() {
        let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        let page = context.new_page();
        let first = page.observe_profile(ProjectionProfile::Full).await.unwrap();
        assert_eq!(first.page.url, "about:blank");
        assert_eq!(
            page.evaluate("document.URL+'|'+document.readyState+'|'+(document.body!==null)")
                .await
                .unwrap(),
            "about:blank|complete|true"
        );
        let second = page.observe_profile(ProjectionProfile::Full).await.unwrap();
        assert_eq!(
            first.page.document_generation,
            second.page.document_generation
        );
        assert_eq!(
            page.evaluate("navigation.currentEntry.index")
                .await
                .unwrap(),
            "0"
        );
    }
    #[test]
    fn missing_navigated_document_cannot_materialize_fresh_blank() {
        let mut state = PageState::new(
            PageId(9908),
            None,
            BrowserConfig::default(),
            ContextOptions::default(),
            None,
            #[cfg(unix)]
            None,
        );
        state.url = "https://retired.example/".into();
        let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
        assert!(state.ensure_initial_blank_document(&context).is_err());
        assert!(state.document.is_none());
        assert!(state.executable.is_none());
        assert_eq!(state.url, "https://retired.example/");
    }
}
