// Included by runtime.rs. The page actor owns exactly one uncommitted fetch;
// transport/policy jobs retain their own cleanup after this owner is dropped.
#[cfg(feature = "v8-host")]
enum NavigationProgress {
    Ready(NavigationResult),
    Pending(u64),
    Joint(
        oryn_js::joint_history::NativeJointTraversalReceipt,
        NavigationResult,
    ),
}

#[cfg(feature = "v8-host")]
struct NavigationTransaction {
    id: u64,
    #[cfg(unix)]
    ipc_identity: Option<crate::ipc_progress::Identity>,
    timing: oryn_js::navigation_timing::NavigationTiming,
    requested_url: String,
    current: Url,
    history: HistoryUpdate,
    history_entry: Option<NavigationHistoryEntry>,
    post: Option<oryn_js::navigation::PostResource>,
    form_authority: Option<oryn_js::navigation::FormNavigationAuthority>,
    destination: Option<oryn_js::navigation_api::NavigationHistorySnapshot>,
    operation: Option<oryn_js::navigation_api::NativeNavigationToken>,
    download: Option<String>,
    service_authority: Option<oryn_js::navigation::ServiceNavigationAdmission>,
    source_generation: u64,
    deferred_source: bool,
    source_load_epoch: Option<u64>,
    source_creator: Option<oryn_js::worker_wire::Creator>,
    source_url: String,
    cookie_context: crate::cookies::CookieRequestContext,
    #[cfg(unix)]
    network_source: Option<Arc<crate::context_service::ContextNavigationSourceLease>>,
    default_referrer: crate::referrer::ReferrerPolicy,
    referrer_override: Option<String>,
    script_source: bool,
    script_authority: Option<oryn_js::navigation::ScriptNavigationAdmission>,
    deadline: Instant,
    emitter: Option<crate::navigation_control::NavigationEmitter>,
    reply: Option<NavigationReply>,
    admission_reply: Option<oneshot::Sender<Result<(), RuntimeError>>>,
    checkpoint: Option<crate::navigation_control::PendingNavigationAck>,
    #[cfg(unix)]
    policies: VecDeque<(
        NavigationPolicyKind,
        crate::context_service::ContextServiceRequest,
    )>,
    phase: NavigationTransactionPhase,
}
#[cfg(all(feature = "v8-host", unix))]
#[derive(Clone, Copy)]
enum NavigationPolicyKind {
    Resubmission,
    FormSource,
    ScriptSource,
}
#[cfg(feature = "v8-host")]
enum NavigationTransactionPhase {
    Prelude,
    #[cfg(unix)]
    Prerender(Box<prerender_commit::Commit>),
    #[cfg(unix)]
    PrerenderWaiting {
        retry: Option<Instant>,
    },
    #[cfg(unix)]
    Policy(NavigationPolicyKind, navigation_fetch::PendingPolicy),
    Http(Box<navigation_http::HttpNavigation>),
    Mime {
        pending: crate::network::DocumentMimeSniff,
        response: navigation_http::Response<navigation_http::SniffedHeaders>,
    },
    #[cfg(unix)]
    Local(navigation_local::PendingLocal),
    #[cfg(unix)]
    DownloadAuthorization {
        pending: navigation_fetch::PendingPolicy,
        response: navigation_http::Response,
        filename: String,
        media_type: String,
    },
    #[cfg(unix)]
    DownloadBody {
        broker: SharedNetworkBroker,
        pending: navigation_fetch::PendingDownloadBody,
        url: String,
        status: u16,
        filename: String,
        media_type: String,
    },
    #[cfg(unix)]
    BlobDownload {
        pending: navigation_fetch::PendingPolicy,
        filename: String,
    },
    #[cfg(unix)]
    DownloadBytesAuthorization {
        pending: navigation_fetch::PendingPolicy,
        url: String,
        status: u16,
        filename: String,
        media_type: String,
        bytes: Vec<u8>,
    },
    #[cfg(unix)]
    DownloadPublication {
        pending: navigation_fetch::PendingPolicy,
        status: u16,
    },
    Departure {
        input: RootNavigationInput,
        job: Option<Box<RootDeparture>>,
        retry: Option<Instant>,
    },
    Publication {
        restored: bool,
        result: NavigationResult,
        identity: Option<(oryn_js::realm::RealmId, u64, u64)>,
        epoch: Option<u64>,
        retry: Option<Instant>,
        milestone: u8,
    },
    Finished,
}

#[cfg(feature = "v8-host")]
enum RootNavigationInput {
    Http(Box<navigation_http::Response>, String),
    #[cfg(unix)]
    Local(navigation_local::PreparedLocal),
    Buffered {
        media_type: Option<String>,
        bytes: Vec<u8>,
    },
    StandaloneBlob(crate::standalone_document_policy::BlobResource),
    Cached(usize),
}

#[cfg(feature = "v8-host")]
impl PageState {
    fn cancel_navigation_transaction(&mut self, error: RuntimeError) {
        if let Some(settlement) = self.settling_navigation.take()
            && let Some(reply) = settlement.reply
        {
            // A delayed ACK may already own the primary failure. Cleanup or
            // replacement must not overwrite it with a later cancellation.
            let cause = settlement.result.err().unwrap_or_else(|| error.clone());
            reply.complete(self, Err(cause));
        }
        let Some(mut transaction) = self.pending_navigation_transaction.take() else {
            return;
        };
        if let Some(reply) = transaction.admission_reply.take() {
            let _ = reply.send(Err(
                if matches!(&error, RuntimeError::Closed | RuntimeError::PageTaskYield) {
                    RuntimeError::Closed
                } else {
                    RuntimeError::NavigationCancelled
                },
            ));
        }
        #[cfg(unix)]
        self.retain_prerender_recovery(&mut transaction);
        let result = self.settle_failed_navigation(
            transaction.operation.as_ref(),
            transaction.source_generation,
            Err(error),
        );
        if let Some(reply) = transaction.reply.take() {
            reply.complete(self, result);
        }
        if let Some(emitter) = transaction.emitter.take() {
            let reason = emitter
                .checkpoint()
                .err()
                .unwrap_or(crate::navigation_control::NavigationStop::Superseded);
            let _ = emitter.publish(crate::navigation_control::NavigationEvent::Stopped(reason));
        }
    }

    fn navigation_transaction_live(
        &mut self,
        transaction: &NavigationTransaction,
    ) -> Result<(), RuntimeError> {
        if let NavigationTransactionPhase::Publication {
            identity, epoch, ..
        } = &transaction.phase
        {
            if Instant::now() >= transaction.deadline {
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "navigation publication wall time".into(),
                });
            }
            if transaction
                .reply
                .as_ref()
                .is_some_and(NavigationReply::is_closed)
                || transaction
                    .emitter
                    .as_ref()
                    .is_some_and(|emitter| emitter.cached_stop().is_some())
            {
                return Err(RuntimeError::NavigationCancelled);
            }
            if self
                .executable
                .as_ref()
                .and_then(|page| page.form_history_root_identity())
                != *identity
                || self
                    .executable
                    .as_ref()
                    .and_then(|page| page.native_navigation_load_epoch())
                    != *epoch
            {
                return Err(RuntimeError::NavigationCancelled);
            }
            return Ok(());
        }
        #[cfg(unix)]
        if let NavigationTransactionPhase::Prerender(job) = &transaction.phase {
            if Instant::now() >= transaction.deadline
                || transaction
                    .reply
                    .as_ref()
                    .is_some_and(NavigationReply::is_closed)
                || transaction
                    .emitter
                    .as_ref()
                    .is_some_and(|emitter| emitter.cached_stop().is_some())
            {
                return Err(RuntimeError::NavigationCancelled);
            }
            if job.installed() {
                return if self
                    .executable
                    .as_ref()
                    .is_some_and(|page| page.agent_root_realm() == job.destination())
                {
                    Ok(())
                } else {
                    Err(RuntimeError::NavigationCancelled)
                };
            }
            if job.source_suspended() {
                return if self.executable.is_none() {
                    Ok(())
                } else {
                    Err(RuntimeError::NavigationCancelled)
                };
            }
        }
        if self.document_generation != transaction.source_generation {
            return Err(RuntimeError::NavigationCancelled);
        }
        if Instant::now() >= transaction.deadline {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "navigation wall time".into(),
            });
        }
        if let Some(error) = transaction
            .emitter
            .as_ref()
            .and_then(|emitter| emitter.cached_stop())
        {
            return Err(RuntimeError::Network(error.to_string()));
        }
        if transaction
            .reply
            .as_ref()
            .is_some_and(NavigationReply::is_closed)
        {
            return Err(RuntimeError::NavigationCancelled);
        }
        if let Some(executable) = self.executable.as_mut() {
            if executable.native_navigation_load_epoch() != transaction.source_load_epoch {
                return Err(RuntimeError::NavigationCancelled);
            }
            Self::check_navigation_operation(executable, &transaction.operation)?;
            if transaction.source_creator.as_ref().is_some_and(|source| {
                executable.native_navigation_creator().ok().as_ref() != Some(source)
            }) {
                return Err(RuntimeError::NavigationCancelled);
            }
        } else if transaction.source_creator.is_some() {
            return Err(RuntimeError::NavigationCancelled);
        }
        Ok(())
    }

    fn incumbent_navigation_request_context(
        &self,
        history: HistoryUpdate,
    ) -> crate::cookies::CookieRequestContext {
        crate::cookies::CookieRequestContext {
            initiator_url: Some(self.url.clone()),
            fetch_metadata: crate::fetch_metadata::Metadata {
                opaque_initiator: self
                    .executable
                    .as_ref()
                    .and_then(|page| page.native_navigation_origin())
                    .is_none_or(|origin| origin == "null"),
                ..Default::default()
            },
            navigation: oryn_js::platform::NavigationRequestFlags {
                reload: matches!(history, HistoryUpdate::Reload),
                history: matches!(history, HistoryUpdate::Preserve(_)),
            },
            ..crate::cookies::CookieRequestContext::navigation(Some(self.url.clone()))
        }
    }

    fn begin_navigation_resource(
        &mut self,
        requested_url: String,
        history: HistoryUpdate,
        post: Option<oryn_js::navigation::PostResource>,
        referrer_policy: Option<String>,
        form_authority: Option<oryn_js::navigation::FormNavigationAuthority>,
        script_authority: Option<oryn_js::navigation::ScriptNavigationAdmission>,
    ) -> Result<NavigationProgress, RuntimeError> {
        #[cfg(unix)]
        self.poll_prerender_recovery();
        #[cfg(unix)]
        {
            self.poll_initial_document_cleanup();
            if self
                .initial_document_cleanup
                .as_ref()
                .is_some_and(|pending| pending.snapshot.is_some())
            {
                return Err(RuntimeError::NavigationCancelled);
            }
        }
        self.poll_failed_install_recovery();
        if self.failed_install_recovery.is_some() {
            return Err(RuntimeError::NavigationCancelled);
        }
        self.poll_source_resume();
        if self.root_source_resume.is_some() {
            return Err(RuntimeError::NavigationCancelled);
        }
        if let Some(executable) = self.executable.as_mut() {
            Self::check_navigation_operation(executable, &self.script_navigation_operation)?;
        }
        let current = Url::parse(&requested_url)
            .map_err(|error| RuntimeError::Network(format!("invalid URL: {error}")))?;
        if post.as_ref().is_some_and(|post| !post.valid_for(&current)) {
            return Err(RuntimeError::ProtocolViolation(
                "invalid navigation POST resource".into(),
            ));
        }
        let history = match history {
            HistoryUpdate::Preserve(index) => HistoryUpdate::Preserve(
                self.resolve_history_target(index, self.script_navigation_destination.as_ref())?,
            ),
            other => other,
        };
        let history_entry = match history {
            HistoryUpdate::Preserve(index) => Some(
                self.history
                    .get(index)
                    .ok_or(RuntimeError::HistoryBoundary)?
                    .clone(),
            ),
            HistoryUpdate::Reload => self
                .history_index
                .and_then(|index| self.history.get(index))
                .cloned(),
            _ => None,
        };
        let same_document_traversal = matches!(history, HistoryUpdate::Preserve(_))
            && history_entry
                .as_ref()
                .and_then(|entry| entry.navigation.as_ref())
                .zip(
                    self.history_index
                        .and_then(|index| self.history.get(index))
                        .and_then(|entry| entry.navigation.as_ref()),
                )
                .is_some_and(|(to, from)| to.document_id == from.document_id);
        let deadline = Instant::now()
            .checked_add(self.config.navigation_timeout)
            .ok_or_else(|| RuntimeError::Network("navigation deadline overflow".into()))?;
        let id = self
            .next_navigation_transaction
            .checked_add(1)
            .ok_or_else(|| RuntimeError::IdentityExhausted {
                namespace: "navigation transaction".into(),
            })?;
        let source_creator = self
            .executable
            .as_mut()
            .map(|page| page.native_navigation_creator().map_err(map_host_error))
            .transpose()?;
        #[cfg(unix)]
        let mut policies = VecDeque::new();
        #[cfg(unix)]
        let mut network_source = None;
        #[cfg(unix)]
        if let Some(client) = self.context_client.as_ref() {
            use crate::context_service::ContextServiceRequest as Request;
            if post.is_some()
                && !same_document_traversal
                && matches!(history, HistoryUpdate::Reload | HistoryUpdate::Preserve(_))
            {
                policies.push_back((
                    NavigationPolicyKind::Resubmission,
                    Request::AuthorizeFormSubmission {
                        report_source: None,
                        authority: form_authority.clone(),
                        page_id: self.page_id,
                        url: requested_url.clone(),
                        method: "POST".into(),
                    },
                ));
            }
            if let Some(admission) = self.service_navigation_authority.clone() {
                if post.is_some()
                    || form_authority.is_some()
                    || script_authority.is_some()
                    || referrer_policy.is_some()
                {
                    return Err(RuntimeError::ProtocolViolation(
                        "mixed native navigation source authorities".into(),
                    ));
                }
                network_source = Some(client.reserve_navigation_source(self.page_id)?);
                policies.push_back((
                    NavigationPolicyKind::ScriptSource,
                    Request::AcquireNavigationNetworkSource {
                        page_id: self.page_id,
                        nonce: network_source
                            .as_ref()
                            .expect("reserved source lease")
                            .nonce
                            .clone(),
                        target: admission.target,
                        url: requested_url.clone(),
                        authority: crate::context_service::NavigationNetworkAuthority::Service(
                            admission.authority,
                        ),
                    },
                ));
            }
            if !same_document_traversal && let Some(authority) = form_authority.clone() {
                policies.push_back((
                    NavigationPolicyKind::FormSource,
                    Request::CheckFormNavigationPolicy {
                        page_id: self.page_id,
                        authority,
                        url: requested_url.clone(),
                    },
                ));
            }
            if let Some(admission) = script_authority.clone() {
                network_source = Some(client.reserve_navigation_source(self.page_id)?);
                policies.push_back((
                    NavigationPolicyKind::ScriptSource,
                    Request::AcquireNavigationNetworkSource {
                        page_id: self.page_id,
                        nonce: network_source
                            .as_ref()
                            .expect("reserved source lease")
                            .nonce
                            .clone(),
                        target: admission.target,
                        url: requested_url.clone(),
                        authority: crate::context_service::NavigationNetworkAuthority::Script(
                            admission.authority,
                        ),
                    },
                ));
            }
        } else if script_authority.is_some() || self.service_navigation_authority.is_some() {
            return Err(RuntimeError::ProtocolViolation(
                "native navigation source service unavailable".into(),
            ));
        }
        #[cfg(not(unix))]
        if script_authority.is_some() || self.service_navigation_authority.is_some() {
            return Err(RuntimeError::ProtocolViolation(
                "native navigation source service unavailable".into(),
            ));
        }
        self.cancel_navigation_transaction(RuntimeError::NavigationCancelled);
        self.next_navigation_transaction = id;
        self.pending_navigation_transaction = Some(NavigationTransaction {
            #[cfg(unix)]
            ipc_identity: self
                .navigation_progress_identity
                .or_else(|| {
                    self.action_navigation
                        .as_ref()
                        .and_then(|barrier| barrier.ipc_identity)
                })
                .or_else(|| crate::ipc_progress::capture(self.page_id.0)),
            timing: oryn_js::navigation_timing::NavigationTiming::new(
                Instant::now(),
                match history {
                    HistoryUpdate::Reload => oryn_js::navigation_timing::Kind::Reload,
                    HistoryUpdate::Preserve(_) => oryn_js::navigation_timing::Kind::BackForward,
                    _ => oryn_js::navigation_timing::Kind::Navigate,
                },
            ),
            id,
            requested_url,
            current,
            history,
            history_entry,
            post,
            form_authority,
            destination: self.script_navigation_destination.clone(),
            operation: self.script_navigation_operation.clone(),
            download: self.script_navigation_download.clone(),
            service_authority: self.service_navigation_authority.clone(),
            source_generation: self.document_generation,
            deferred_source: {
                #[cfg(unix)]
                {
                    self.prerender_recovery.is_some()
                }
                #[cfg(not(unix))]
                {
                    false
                }
            },
            source_load_epoch: self
                .executable
                .as_ref()
                .and_then(|page| page.native_navigation_load_epoch()),
            source_creator,
            source_url: self.url.clone(),
            cookie_context: self.incumbent_navigation_request_context(history),
            #[cfg(unix)]
            network_source,
            default_referrer: self.referrer_policy,
            referrer_override: referrer_policy,
            script_source: script_authority.is_some(),
            script_authority,
            deadline,
            emitter: self.active_navigation.clone(),
            reply: None,
            admission_reply: None,
            checkpoint: None,
            #[cfg(unix)]
            policies,
            phase: NavigationTransactionPhase::Prelude,
        });
        Ok(NavigationProgress::Pending(id))
    }
}

#[cfg(feature = "v8-host")]
impl PageState {
    fn with_navigation_transaction<T>(
        &mut self,
        transaction: &mut NavigationTransaction,
        run: impl FnOnce(&mut Self, &mut NavigationTransaction) -> Result<T, RuntimeError>,
    ) -> Result<T, RuntimeError> {
        let destination = std::mem::replace(
            &mut self.script_navigation_destination,
            transaction.destination.clone(),
        );
        let operation = std::mem::replace(
            &mut self.script_navigation_operation,
            transaction.operation.clone(),
        );
        let download = std::mem::replace(
            &mut self.script_navigation_download,
            transaction.download.clone(),
        );
        let service = std::mem::replace(
            &mut self.service_navigation_authority,
            transaction.service_authority.clone(),
        );
        let deferred = std::mem::replace(&mut self.defer_navigation_construction, true);
        self.active_navigation = transaction.emitter.clone();
        let result = run(self, transaction);
        self.script_navigation_destination = destination;
        self.script_navigation_operation = operation;
        self.script_navigation_download = download;
        self.service_navigation_authority = service;
        self.defer_navigation_construction = deferred;
        result
    }
    fn launch_navigation_transaction(
        &mut self,
        transaction: &mut NavigationTransaction,
    ) -> Result<Option<NavigationResult>, RuntimeError> {
        if transaction.current.scheme() == "javascript" {
            #[cfg(unix)]
            return self.launch_javascript_navigation(transaction);
            #[cfg(not(unix))]
            return Err(RuntimeError::NavigationCancelled);
        }
        let requested_url = transaction.requested_url.clone();
        let current = transaction.current.clone();
        let post = transaction.post.clone();
        let history_update = match transaction.history {
            HistoryUpdate::Preserve(index) => HistoryUpdate::Preserve(
                self.resolve_history_target(
                    index,
                    transaction
                        .history_entry
                        .as_ref()
                        .and_then(|entry| entry.navigation.as_ref()),
                )?,
            ),
            other => other,
        };
        transaction.history = history_update;
        let same_document_traversal = matches!(history_update, HistoryUpdate::Preserve(_))
            && transaction
                .history_entry
                .as_ref()
                .and_then(|entry| entry.navigation.as_ref())
                .zip(
                    self.history_index
                        .and_then(|index| self.history.get(index))
                        .and_then(|entry| entry.navigation.as_ref()),
                )
                .is_some_and(|(to, from)| to.document_id == from.document_id);
        self.trace.push(
            Revision(self.revision),
            None,
            TraceEventKind::NavigationStarted {
                url: requested_url.clone(),
            },
        );
        #[cfg(feature = "v8-host")]
        let explicit_download = self.script_navigation_download.is_some();
        #[cfg(not(feature = "v8-host"))]
        let explicit_download = false;
        if !explicit_download && (post.is_none() || same_document_traversal)
            && !matches!(history_update, HistoryUpdate::Reload)
            && self.document.is_some()
            // Only a destination with a fragment can take the fragment-only
            // navigation path. Following the current URL without a fragment
            // loads a new document, even when its URL is unchanged.
            && (same_document_traversal || (current.fragment().is_some()
                && same_document_url(&self.url, current.as_str())))
        {
            #[cfg(feature = "v8-host")]
            let previous_url = self.url.clone();
            self.url = current.to_string();
            self.revision = self.revision.saturating_add(1);
            if matches!(history_update, HistoryUpdate::Push) {
                self.truncate_forward_history();
                let mut entry = self
                    .history_index
                    .and_then(|index| self.history.get(index).cloned())
                    .unwrap_or_else(|| self.url.clone().into());
                entry.url = self.url.clone();
                #[cfg(feature = "v8-host")]
                if let Some(snapshot) = &mut entry.navigation {
                    snapshot.key = new_navigation_history_token()?;
                    snapshot.id = new_navigation_history_token()?;
                    snapshot.url = self.url.clone();
                    snapshot.state = None;
                    snapshot.history_state = None;
                }
                self.history.push(entry);
                self.history_index = Some(self.history.len() - 1);
            } else if matches!(history_update, HistoryUpdate::Replace)
                && let Some(index) = self.history_index
            {
                self.history[index].url = self.url.clone();
                #[cfg(feature = "v8-host")]
                if let Some(snapshot) = &mut self.history[index].navigation {
                    snapshot.id = new_navigation_history_token()?;
                    snapshot.url = self.url.clone();
                }
                #[cfg(feature = "v8-host")]
                self.back_forward_cache.remove(&index);
            } else if let HistoryUpdate::Preserve(index) = history_update {
                self.history_index = Some(index);
            }
            #[cfg(feature = "v8-host")]
            let history = Self::visible_navigation_history(
                &self.history,
                self.history_index.ok_or(RuntimeError::HistoryBoundary)?,
                match history_update {
                    HistoryUpdate::Push => "push",
                    HistoryUpdate::Replace => "replace",
                    _ => "traverse",
                },
            )?;
            #[cfg(feature = "v8-host")]
            let session = Self::full_session_history(
                &self.history,
                self.history_index.ok_or(RuntimeError::HistoryBoundary)?,
                &history.navigation_type,
            )?;
            #[cfg(feature = "v8-host")]
            if let Some(executable) = self.executable.as_mut() {
                let traversed = matches!(history_update, HistoryUpdate::Preserve(_));
                executable
                    .set_committed_location(&self.url)
                    .map_err(map_host_error)?;
                executable
                    .sync_navigation_history(history)
                    .map_err(map_host_error)?;
                executable
                    .sync_session_history(session)
                    .map_err(map_host_error)?;
                executable
                    .dispatch_history_transition(&previous_url, &self.url, traversed)
                    .map_err(map_host_error)?;
                self.sync_executable_semantics()?;
            }
            self.record_snapshot()?;
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::NavigationCommitted {
                    url: self.url.clone(),
                },
            );
            return Ok(Some(NavigationResult {
                page_id: self.page_id,
                requested_url,
                final_url: self.url.clone(),
                title: self
                    .document
                    .as_ref()
                    .map(document_title)
                    .unwrap_or_default(),
                status: 200,
                document_generation: self.document_generation,
                revision: Revision(self.revision),
                lifecycle: vec![LifecycleState::Commit],
                challenge: self.last_challenge.clone(),
            }));
        }
        #[cfg(feature = "v8-host")]
        if let Some(executable) = self.executable.as_mut() {
            if !executable
                .dispatch_before_unload()
                .map_err(map_host_error)?
            {
                return Err(RuntimeError::NavigationCancelled);
            }
            Self::check_navigation_operation(executable, &self.script_navigation_operation)?;
        }
        #[cfg(unix)]
        if matches!(history_update, HistoryUpdate::Push | HistoryUpdate::Replace)
            && post.is_none()
            && transaction.download.is_none()
            && let Some(snapshot) = self
                .executable
                .as_ref()
                .map(|page| page.prerender_source_snapshot().map_err(map_host_error))
                .transpose()?
                .flatten()
        {
            if let Some(candidate) = self
                .prerender_candidates
                .take_for_activation(&snapshot, current.as_str())
            {
                match prerender_commit::Commit::new(self, candidate, history_update) {
                    Ok(job) => {
                        transaction.phase = NavigationTransactionPhase::Prerender(Box::new(job));
                        return Ok(None);
                    }
                    Err((error, candidate)) => {
                        self.enqueue_retired_agent_root(candidate.executable.agent_root_realm());
                        drop(candidate);
                        return Err(error);
                    }
                }
            }
            if self
                .prerender_candidates
                .matching_pending(&snapshot, current.as_str())
            {
                transaction.phase = NavigationTransactionPhase::PrerenderWaiting { retry: None };
                return Ok(None);
            }
        }
        self.start_navigation_fetch(transaction)
    }
    /// This continuation is entered after beforeunload/admission, including a
    /// failed or removed speculative candidate. It never replays that prefix.
    fn start_navigation_fetch(
        &mut self,
        transaction: &mut NavigationTransaction,
    ) -> Result<Option<NavigationResult>, RuntimeError> {
        let current = transaction.current.clone();
        let post = transaction.post.clone();
        let history_update = transaction.history;
        if let HistoryUpdate::Preserve(index) = history_update
            && self
                .back_forward_cache
                .get(&index)
                .is_some_and(|entry| entry.reusable)
        {
            transaction.phase = NavigationTransactionPhase::Departure {
                input: RootNavigationInput::Cached(index),
                job: None,
                retry: None,
            };
            return Ok(None);
        }
        #[cfg(all(unix, feature = "v8-host"))]
        if current.scheme() == "about" && current.path() == "blank" && self.context_client.is_some()
        {
            return self.start_local_navigation_transaction(transaction, false);
        }
        if current.as_str() == "about:blank" {
            transaction.phase = NavigationTransactionPhase::Departure {
                input: RootNavigationInput::Buffered {
                    media_type: Some("text/html".into()),
                    bytes: b"<!doctype html>".to_vec(),
                },
                job: None,
                retry: None,
            };
            return Ok(None);
        }
        if current.scheme() == "data" {
            #[cfg(feature = "v8-host")]
            if let Some(filename) = self.script_navigation_download.clone() {
                #[cfg(unix)]
                {
                    let (media_type, body) = crate::network::decode_data_url(&current)
                        .map_err(|error| RuntimeError::Network(error.to_string()))?;
                    return self.start_navigation_download_bytes(
                        transaction,
                        current.to_string(),
                        200,
                        filename,
                        media_type.unwrap_or_else(|| "text/plain;charset=US-ASCII".into()),
                        body,
                    );
                }
                #[cfg(not(unix))]
                {
                    let _ = filename;
                    return Err(RuntimeError::Download(
                        "native download service unavailable".into(),
                    ));
                }
            }
            #[cfg(unix)]
            if self.context_client.is_some() {
                return self.start_local_navigation_transaction(transaction, false);
            }
            let (media_type, bytes) = crate::network::decode_data_url(&current)
                .map_err(|error| RuntimeError::Network(error.to_string()))?;
            transaction.phase = NavigationTransactionPhase::Departure {
                input: RootNavigationInput::Buffered { media_type, bytes },
                job: None,
                retry: None,
            };
            return Ok(None);
        }
        if current.scheme() == "blob" && {
            #[cfg(unix)]
            {
                self.context_client.is_none()
            }
            #[cfg(not(unix))]
            {
                true
            }
        } {
            let resource = self
                .executable
                .as_mut()
                .ok_or(RuntimeError::NavigationCancelled)?
                .standalone_navigation_blob(current.as_str())
                .map_err(map_host_error)?
                .ok_or_else(|| RuntimeError::Network("blob URL revoked or foreign".into()))?;
            if resource.bytes.len() > self.config.max_response_bytes {
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "blob navigation bytes".into(),
                });
            }
            transaction.phase = NavigationTransactionPhase::Departure {
                input: RootNavigationInput::StandaloneBlob(resource),
                job: None,
                retry: None,
            };
            return Ok(None);
        }
        #[cfg(all(unix, feature = "v8-host"))]
        if current.scheme() == "blob"
            && let Some(filename) = self.script_navigation_download.clone()
        {
            let creator = transaction.source_creator.clone().ok_or_else(|| {
                RuntimeError::Download("download source document unavailable".into())
            })?;
            let client = self
                .context_client
                .clone()
                .ok_or_else(|| RuntimeError::Download("parent blob service unavailable".into()))?;
            let mut lookup = current.clone();
            lookup.set_fragment(None);
            let pending = navigation_fetch::PendingPolicy::start_blob_read(
                client,
                crate::context_service::ContextServiceRequest::WorkerService {
                    source_page_id: self.page_id,
                    request: oryn_js::worker_wire::AgentServiceRequest::BlobResolve {
                        creator,
                        url: lookup.to_string(),
                    },
                },
                Some(transaction.deadline),
            )?;
            transaction.phase = NavigationTransactionPhase::BlobDownload { pending, filename };
            return Ok(None);
        }
        #[cfg(all(unix, feature = "v8-host"))]
        if current.scheme() == "blob" {
            return self.start_local_navigation_transaction(transaction, true);
        }
        let broker = self
            .network_transport
            .as_ref()
            .ok_or_else(|| RuntimeError::Network("network transport initialization failed".into()))?
            .clone()
            .new_request_scope();
        let mut referrer = if transaction.script_source {
            transaction.default_referrer
        } else {
            transaction
                .referrer_override
                .as_deref()
                .map(crate::referrer::ReferrerPolicy::parse_list)
                .unwrap_or(transaction.default_referrer)
        };
        let mut referrer_source = Url::parse(&transaction.source_url).ok();
        if let Some(saved) = transaction
            .history_entry
            .as_ref()
            .and_then(|entry| entry.request_state.as_ref())
        {
            transaction.cookie_context.initiator_url = saved.initiator_url.clone();
            transaction.cookie_context.fetch_metadata.opaque_initiator = saved.opaque_initiator;
            transaction.cookie_context.fetch_metadata.redirect_taint = Default::default();
            referrer_source = saved
                .referrer
                .as_deref()
                .and_then(|value| Url::parse(value).ok());
            referrer = crate::referrer::ReferrerPolicy::parse_list(&saved.referrer_policy);
        }
        transaction.phase = NavigationTransactionPhase::Http(Box::new(
            navigation_http::HttpNavigation::new(
                broker,
                #[cfg(unix)]
                self.context_client.clone(),
                self.page_id,
                #[cfg(unix)]
                transaction
                    .source_creator
                    .clone()
                    .map(crate::context_service::NavigationPolicySource::Document)
                    .unwrap_or(crate::context_service::NavigationPolicySource::InitialAboutBlank),
                current,
                post,
                transaction.form_authority.clone(),
                transaction.cookie_context.clone(),
                referrer_source,
                referrer,
                self.config.max_redirects,
                Some(transaction.deadline),
            )
            .with_timing(transaction.timing.clone())
            .with_diagnostics(
                self.trace.diagnostic_sink(Revision(self.revision)),
                {
                    #[cfg(unix)]
                    {
                        transaction.ipc_identity
                    }
                    #[cfg(not(unix))]
                    {
                        None
                    }
                },
                transaction.id,
                transaction.source_generation,
            ),
        ));
        Ok(None)
    }
    fn commit_navigation_transaction(
        &mut self,
        transaction: &mut NavigationTransaction,
        response: navigation_http::Response,
        computed_type: String,
    ) -> Result<NavigationResult, RuntimeError> {
        let requested_url = transaction.requested_url.clone();
        let current = response.current;
        let post = response.post;
        let cross_origin_redirect = response.cross_origin_redirect;
        let navigation_fetch::Headers {
            response_policy,
            status,
            headers: response_headers,
            operation,
            ..
        } = response.headers;
        let content_type = Some(computed_type);
        let navigation_deadline = Some(transaction.deadline);
        let form_authority = transaction.form_authority.clone();
        let history_update = match transaction.history {
            HistoryUpdate::Preserve(index) => HistoryUpdate::Preserve(
                self.resolve_history_target(
                    index,
                    transaction
                        .history_entry
                        .as_ref()
                        .and_then(|entry| entry.navigation.as_ref()),
                )?,
            ),
            other => other,
        };
        let essence = content_type
            .as_deref()
            .unwrap_or("text/html")
            .split(';')
            .next()
            .unwrap_or("text/html")
            .trim()
            .to_ascii_lowercase();
        let text_kind = crate::network::navigation_text_kind(content_type.as_deref());
        let capture = Arc::new(Mutex::new(NavigationBodyCapture::default()));
        let encoding = Arc::new(Mutex::new(None));
        let source = NavigationBodyChunks {
            timing: Some(response.timing.clone()),
            replay: Default::default(),
            operation,
            decoder: oryn_net::IncrementalTextDecoder::new(content_type.as_deref(), text_kind),
            capture: capture.clone(),
            encoding: encoding.clone(),
            deadline: navigation_deadline,
            navigation: self.active_navigation.clone(),
            finished: false,
            demand_pending: false,
        };
        self.load_document_with_navigation_resource(
            current.to_string(),
            "",
            None,
            Some(crate::host::DocumentStream {
                content_language: oryn_dom::language::content_language_header(
                    response_headers
                        .iter()
                        .filter(|(name, _)| name.eq_ignore_ascii_case("content-language"))
                        .map(|(_, value)| value.as_str()),
                ),
                chunks: Box::new(source),
                content_type: content_type.clone(),
                encoding,
            }),
            history_update,
            &BTreeMap::new(),
            Vec::new(),
            cross_origin_redirect,
            Some(response_headers.clone()),
            Some(NavigationCommitResource {
                response_policy,
                #[cfg(unix)]
                response_policy_cleanup: response.response_policy_cleanup,
                request_state: Some(response.request_state),
                broker: response.broker,
                post: post.clone(),
                form_authority: form_authority.clone(),
            }),
        )?;
        self.pending_navigation_response = Some(PendingNavigationResponse {
            generation: self.document_generation,
            root_identity: self
                .executable
                .as_ref()
                .and_then(|page| page.form_history_root_identity())
                .map(|(realm, generation, epoch)| (u64::from(realm.0), generation, epoch)),
            current: current.clone(),
            status,
            response_headers: response_headers.clone(),
            essence,
            capture,
        });
        self.finish_pending_navigation_response()?;
        Ok(NavigationResult {
            page_id: self.page_id,
            requested_url,
            final_url: self.url.clone(),
            title: self
                .document
                .as_ref()
                .map(document_title)
                .unwrap_or_default(),
            status,
            document_generation: self.document_generation,
            revision: Revision(self.revision),
            lifecycle: self.lifecycle.clone(),
            challenge: self.last_challenge.clone(),
        })
    }
}

#[cfg(feature = "v8-host")]
impl PageState {
    // Candidate construction can retain a full document in error results.
    // Keep those temporaries off ordinary navigation/realm-construction stacks.
    #[cfg(unix)]
    #[inline(never)]
    fn step_waiting_prerender_transaction(
        &mut self,
        transaction: &mut NavigationTransaction,
        retry: Option<Instant>,
    ) -> std::task::Poll<Result<NavigationResult, RuntimeError>> {
        use std::task::Poll;
        if retry.is_some_and(|time| time > Instant::now()) {
            transaction.phase = NavigationTransactionPhase::PrerenderWaiting { retry };
            return Poll::Pending;
        }
        let result = self.with_navigation_transaction(transaction, |state, transaction| {
            state.poll_prerender_candidates()?;
            state.navigation_transaction_live(transaction)?;
            if let Some(snapshot) = state
                .executable
                .as_ref()
                .map(|page| page.prerender_source_snapshot().map_err(map_host_error))
                .transpose()?
                .flatten()
            {
                if let Some(candidate) = state
                    .prerender_candidates
                    .take_for_activation(&snapshot, transaction.current.as_str())
                {
                    match prerender_commit::Commit::new(state, candidate, transaction.history) {
                        Ok(job) => {
                            transaction.phase =
                                NavigationTransactionPhase::Prerender(Box::new(job));
                            return Ok(None);
                        }
                        Err((error, candidate)) => {
                            state.enqueue_retired_agent_root(
                                candidate.executable.agent_root_realm(),
                            );
                            drop(candidate);
                            return Err(error);
                        }
                    }
                }
                if state
                    .prerender_candidates
                    .matching_pending(&snapshot, transaction.current.as_str())
                {
                    transaction.phase = NavigationTransactionPhase::PrerenderWaiting {
                        retry: Some(Instant::now() + Duration::from_millis(16)),
                    };
                    return Ok(None);
                }
            }
            state.start_navigation_fetch(transaction)
        });
        match result {
            Ok(None) => Poll::Pending,
            Ok(Some(result)) => Poll::Ready(Ok(result)),
            Err(error) => Poll::Ready(Err(error)),
        }
    }

    fn step_navigation_transaction(
        &mut self,
        transaction: &mut NavigationTransaction,
    ) -> std::task::Poll<Result<NavigationResult, RuntimeError>> {
        // A future actor turn may run for a different command. Carry admission
        // identity through this exact transaction rather than recapturing it.
        #[cfg(unix)]
        let previous = std::mem::replace(
            &mut self.navigation_progress_identity,
            transaction.ipc_identity,
        );
        let result = self.step_navigation_transaction_inner(transaction);
        #[cfg(unix)]
        {
            self.navigation_progress_identity = previous;
        }
        result
    }

    fn step_navigation_transaction_inner(
        &mut self,
        transaction: &mut NavigationTransaction,
    ) -> std::task::Poll<Result<NavigationResult, RuntimeError>> {
        use std::task::Poll;
        if transaction.deferred_source {
            #[cfg(unix)]
            if self.prerender_recovery.is_some() {
                return Poll::Pending;
            }
            // A new request may arrive while the previous activation ACK is
            // unresolved. Capture its authority only after exact rollback.
            transaction.source_generation = self.document_generation;
            transaction.source_load_epoch = self
                .executable
                .as_ref()
                .and_then(|page| page.native_navigation_load_epoch());
            transaction.source_creator = self
                .executable
                .as_mut()
                .map(|page| page.native_navigation_creator().map_err(map_host_error))
                .transpose()?;
            transaction.source_url = self.url.clone();
            let source_context = self.incumbent_navigation_request_context(transaction.history);
            transaction.cookie_context.initiator_url = source_context.initiator_url;
            transaction.cookie_context.top_level_url = source_context.top_level_url;
            transaction.cookie_context.fetch_metadata.opaque_initiator =
                source_context.fetch_metadata.opaque_initiator;
            transaction.default_referrer = self.referrer_policy;
            transaction.history_entry = match transaction.history {
                HistoryUpdate::Preserve(index) => Some(
                    self.history
                        .get(index)
                        .ok_or(RuntimeError::HistoryBoundary)?
                        .clone(),
                ),
                HistoryUpdate::Reload => self
                    .history_index
                    .and_then(|index| self.history.get(index))
                    .cloned(),
                _ => None,
            };
            transaction.deferred_source = false;
        }
        self.navigation_transaction_live(transaction)?;
        if let Some(emitter) = &transaction.emitter {
            if transaction.checkpoint.is_none() {
                transaction.checkpoint = Some(
                    emitter
                        .checkpoint_pending()
                        .map_err(|error| RuntimeError::Network(error.to_string()))?,
                );
            }
            match transaction.checkpoint.as_ref().unwrap().poll() {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(result) => {
                    transaction.checkpoint.take();
                    result.map_err(|error| RuntimeError::Network(error.to_string()))?;
                }
            }
        }
        if let Some(reply) = transaction.admission_reply.take() {
            let _ = reply.send(Ok(()));
        }
        let phase = std::mem::replace(&mut transaction.phase, NavigationTransactionPhase::Finished);
        match phase {
            #[cfg(unix)]
            NavigationTransactionPhase::PrerenderWaiting { retry } => {
                self.step_waiting_prerender_transaction(transaction, retry)
            }
            NavigationTransactionPhase::Departure { input, job, retry } => {
                self.step_departure_navigation_transaction(transaction, input, job, retry)
            }
            phase => self.step_nonwaiting_navigation_transaction(transaction, phase),
        }
    }

    // Keep policy, download and speculative-navigation temporaries out of the
    // native stack while document construction enters JavaScript. Dispatch this
    // phase directly rather than through the large nonwaiting phase matcher.
    #[inline(never)]
    fn step_departure_navigation_transaction(
        &mut self,
        transaction: &mut NavigationTransaction,
        input: RootNavigationInput,
        mut job: Option<Box<RootDeparture>>,
        retry: Option<Instant>,
    ) -> std::task::Poll<Result<NavigationResult, RuntimeError>> {
        use std::task::Poll;
        if retry.is_some_and(|time| time > Instant::now()) {
            transaction.phase = NavigationTransactionPhase::Departure { input, job, retry };
            return Poll::Pending;
        }
        // MIME/download decisions precede any departure event.
        let buffered_type = match &input {
            RootNavigationInput::StandaloneBlob(resource) => Some(
                oryn_net::mime_sniff::document_type(Some(&resource.mime), false, &resource.bytes),
            ),
            RootNavigationInput::Buffered { media_type, bytes } => Some(
                oryn_net::mime_sniff::document_type(media_type.as_deref(), false, bytes),
            ),
            #[cfg(unix)]
            RootNavigationInput::Local(prepared) => Some(oryn_net::mime_sniff::document_type(
                Some(&prepared.media_type),
                crate::network::response_no_sniff(&prepared.headers),
                &prepared.bytes,
            )),
            _ => None,
        };
        if buffered_type
            .as_deref()
            .is_some_and(crate::download::computed_type_is_download)
        {
            #[cfg(unix)]
            {
                let bytes = match input {
                    RootNavigationInput::StandaloneBlob(resource) => resource.bytes,
                    RootNavigationInput::Buffered { bytes, .. } => bytes,
                    RootNavigationInput::Local(prepared) => prepared.bytes.clone(),
                    _ => unreachable!(),
                };
                let filename = crate::download::response_download_filename(
                    &[],
                    &transaction.current,
                    Some(""),
                )
                .unwrap_or_else(|| "download".into());
                let current = transaction.current.to_string();
                return match self.start_navigation_download_bytes(
                    transaction,
                    current,
                    200,
                    filename,
                    buffered_type.unwrap(),
                    bytes,
                ) {
                    Ok(Some(result)) => Poll::Ready(Ok(result)),
                    Ok(None) => Poll::Pending,
                    Err(error) => Poll::Ready(Err(error)),
                };
            }
            #[cfg(not(unix))]
            return Poll::Ready(Err(RuntimeError::Download(
                "native download service unavailable".into(),
            )));
        }
        let (url, redirect, cached) = match &input {
            RootNavigationInput::Http(response, _) => (
                response.current.to_string(),
                response.cross_origin_redirect,
                false,
            ),
            RootNavigationInput::Cached(index) => (
                self.back_forward_cache
                    .get(index)
                    .ok_or(RuntimeError::HistoryBoundary)?
                    .url
                    .clone(),
                false,
                true,
            ),
            _ => (transaction.current.to_string(), false, false),
        };
        let progress = self.with_navigation_transaction(transaction, |state, transaction| {
            if job.is_none() {
                job = Some(Box::new(state.prepare_root_departure(
                    &url,
                    transaction.history,
                    redirect,
                    cached,
                )?));
            }
            state.poll_root_departure(job.as_mut().unwrap())
        });
        match progress {
            Ok(true) => {}
            Ok(false) => {
                transaction.phase = NavigationTransactionPhase::Departure {
                    input,
                    job,
                    retry: Some(Instant::now() + Duration::from_millis(16)),
                };
                return Poll::Pending;
            }
            Err(error) => {
                self.navigation_transaction_live(transaction)?;
                if matches!(error, RuntimeError::NavigationCancelled) {
                    return Poll::Ready(Err(error));
                }
                // The retained event engine owns the completed callback
                // prefix. Retry preparation/publication without re-fetch.
                transaction.phase = NavigationTransactionPhase::Departure {
                    input,
                    job,
                    retry: Some(Instant::now() + Duration::from_millis(25)),
                };
                return Poll::Pending;
            }
        }
        self.completed_root_departure = job.map(|job| *job);
        let cached_restore = matches!(&input, RootNavigationInput::Cached(_));
        let result =
            self.with_navigation_transaction(transaction, |state, transaction| match input {
                RootNavigationInput::Http(response, computed) => {
                    state.commit_navigation_transaction(transaction, *response, computed)
                }
                RootNavigationInput::Cached(index) => state
                    .restore_cached_document(index, transaction.requested_url.clone())?
                    .ok_or(RuntimeError::HistoryBoundary),
                RootNavigationInput::StandaloneBlob(mut resource) => {
                    let bytes = std::mem::take(&mut resource.bytes);
                    let mime = resource.mime.clone();
                    let headers = resource.document.headers.clone();
                    let previous = state.prepared_standalone_navigation.replace(resource);
                    let result = state.load_buffered_navigation_with_timing(
                        crate::runtime::BufferedNavigationInputs {
                            url: transaction.current.clone(),
                            requested_url: transaction.requested_url.clone(),
                            history: transaction.history,
                            content_type: Some(mime),
                            bytes,
                            response_headers: headers,
                            timing: transaction.timing.clone(),
                        },
                    );
                    state.prepared_standalone_navigation = previous;
                    result
                }
                RootNavigationInput::Buffered { media_type, bytes } => state
                    .load_buffered_navigation_with_timing(
                        crate::runtime::BufferedNavigationInputs {
                            url: transaction.current.clone(),
                            requested_url: transaction.requested_url.clone(),
                            history: transaction.history,
                            content_type: media_type,
                            bytes,
                            response_headers: Vec::new(),
                            timing: transaction.timing.clone(),
                        },
                    ),
                #[cfg(unix)]
                RootNavigationInput::Local(mut prepared) => {
                    let previous = state.prepared_local_navigation.replace((
                        prepared.token.clone(),
                        transaction.current.to_string(),
                        prepared.headers.clone(),
                        prepared.base_url.clone(),
                    ));
                    let result = state.load_buffered_navigation_with_timing(
                        crate::runtime::BufferedNavigationInputs {
                            url: transaction.current.clone(),
                            requested_url: transaction.requested_url.clone(),
                            history: transaction.history,
                            content_type: Some(prepared.media_type.clone()),
                            bytes: std::mem::take(&mut prepared.bytes),
                            response_headers: std::mem::take(&mut prepared.headers),
                            timing: transaction.timing.clone(),
                        },
                    );
                    state.prepared_local_navigation = previous;
                    result
                }
            });
        self.completed_root_departure.take();
        self.prepared_root_source.take();
        if cached_restore {
            let result = result?;
            transaction.phase = NavigationTransactionPhase::Publication {
                restored: true,
                result,
                identity: self
                    .executable
                    .as_ref()
                    .and_then(|page| page.form_history_root_identity()),
                epoch: self
                    .executable
                    .as_ref()
                    .and_then(|page| page.native_navigation_load_epoch()),
                retry: None,
                milestone: 0,
            };
            Poll::Pending
        } else {
            Poll::Ready(result)
        }
    }

    // Ordinary navigation's construction slots must not remain on the stack
    // while a pending speculative document runs its bootstrap.
    #[inline(never)]
    fn step_nonwaiting_navigation_transaction(
        &mut self,
        transaction: &mut NavigationTransaction,
        phase: NavigationTransactionPhase,
    ) -> std::task::Poll<Result<NavigationResult, RuntimeError>> {
        use std::task::Poll;
        match phase {
            #[cfg(unix)]
            NavigationTransactionPhase::PrerenderWaiting { retry } => {
                self.step_waiting_prerender_transaction(transaction, retry)
            }
            #[cfg(unix)]
            NavigationTransactionPhase::Prerender(mut job) => {
                let result =
                    self.with_navigation_transaction(transaction, |state, _| job.poll(state));
                match result {
                    Ok(true) => {
                        let result = NavigationResult {
                            page_id: self.page_id,
                            requested_url: transaction.requested_url.clone(),
                            final_url: self.url.clone(),
                            title: self
                                .document
                                .as_ref()
                                .map(document_title)
                                .unwrap_or_default(),
                            status: job.status(),
                            document_generation: self.document_generation,
                            revision: Revision(self.revision),
                            lifecycle: self.lifecycle.clone(),
                            challenge: self.last_challenge.clone(),
                        };
                        transaction.phase = NavigationTransactionPhase::Publication {
                            restored: false,
                            result,
                            identity: self
                                .executable
                                .as_ref()
                                .and_then(|page| page.form_history_root_identity()),
                            epoch: self
                                .executable
                                .as_ref()
                                .and_then(|page| page.native_navigation_load_epoch()),
                            retry: None,
                            milestone: job.publication_milestone(),
                        };
                        Poll::Pending
                    }
                    Ok(false) => {
                        transaction.phase = NavigationTransactionPhase::Prerender(job);
                        Poll::Pending
                    }
                    Err(RuntimeError::NavigationCancelled) => {
                        transaction.phase = NavigationTransactionPhase::Prerender(job);
                        Poll::Ready(Err(RuntimeError::NavigationCancelled))
                    }
                    Err(_) => {
                        transaction.phase = NavigationTransactionPhase::Prerender(job);
                        Poll::Pending
                    }
                }
            }
            NavigationTransactionPhase::Prelude => {
                #[cfg(unix)]
                if let Some((kind, request)) = transaction.policies.pop_front() {
                    let client = self.context_client.clone().ok_or_else(|| {
                        RuntimeError::ProtocolViolation("navigation policy endpoint retired".into())
                    })?;
                    transaction.phase = NavigationTransactionPhase::Policy(
                        kind,
                        if matches!(&request, crate::context_service::ContextServiceRequest::AcquireNavigationNetworkSource { .. }) {
                            navigation_fetch::PendingPolicy::start_navigation_source(
                                client, request, Some(transaction.deadline),
                                transaction.network_source.as_ref().ok_or_else(|| RuntimeError::ProtocolViolation("missing navigation source lease".into()))?,
                            )?
                        } else {
                            navigation_fetch::PendingPolicy::start(client, request, Some(transaction.deadline))?
                        },
                    );
                    return Poll::Pending;
                }
                match self
                    .with_navigation_transaction(transaction, Self::launch_navigation_transaction)?
                {
                    Some(result) => Poll::Ready(Ok(result)),
                    None => Poll::Pending,
                }
            }
            #[cfg(unix)]
            NavigationTransactionPhase::Policy(kind, mut pending) => match pending.poll() {
                Poll::Pending => {
                    transaction.phase = NavigationTransactionPhase::Policy(kind, pending);
                    Poll::Pending
                }
                Poll::Ready(result) => {
                    use crate::context_service::ContextServiceResponse as Response;
                    let source = match (kind, result?) {
                        (
                            NavigationPolicyKind::Resubmission,
                            Response::FormSubmissionAuthorized,
                        ) => None,
                        (
                            NavigationPolicyKind::FormSource,
                            Response::FormNavigationPolicyChecked { source },
                        ) => Some(source),
                        (
                            NavigationPolicyKind::ScriptSource,
                            Response::NavigationNetworkSource { source, bound },
                        ) => {
                            transaction.cookie_context.navigation_source = if bound {
                                Some(
                                    transaction
                                        .network_source
                                        .as_ref()
                                        .ok_or_else(|| {
                                            RuntimeError::ProtocolViolation(
                                                "missing navigation source lease".into(),
                                            )
                                        })?
                                        .nonce
                                        .clone(),
                                )
                            } else {
                                None
                            };
                            Some(source)
                        }
                        (_, Response::PageDenied { .. }) => {
                            return Poll::Ready(Err(RuntimeError::PolicyDenied {
                                action: ConsequentialAction::FormSubmission,
                            }));
                        }
                        _ => {
                            return Poll::Ready(Err(RuntimeError::ProtocolViolation(
                                "invalid retained navigation policy response".into(),
                            )));
                        }
                    };
                    if let Some(source) = source {
                        transaction.source_url = source.url.clone();
                        transaction.default_referrer =
                            crate::referrer::ReferrerPolicy::parse_list(&source.referrer_policy);
                        transaction.cookie_context.fetch_metadata.user_activation =
                            source.user_activation;
                        transaction.cookie_context.fetch_metadata.opaque_initiator =
                            source.opaque_origin;
                        transaction.cookie_context.initiator_url = Some(if source.opaque_origin {
                            "about:blank".into()
                        } else {
                            source.url
                        });
                        transaction.cookie_context.top_level_url = Some(source.top_level_url);
                        transaction.cookie_context.ancestor_urls = source.ancestor_urls;
                    }
                    transaction.phase = NavigationTransactionPhase::Prelude;
                    Poll::Pending
                }
            },
            #[cfg(unix)]
            NavigationTransactionPhase::Local(mut pending) => match pending.poll() {
                Poll::Pending => {
                    transaction.phase = NavigationTransactionPhase::Local(pending);
                    Poll::Pending
                }
                Poll::Ready(result) => {
                    let prepared = result?;
                    self.navigation_transaction_live(transaction)?;
                    transaction.phase = NavigationTransactionPhase::Departure {
                        input: RootNavigationInput::Local(prepared),
                        job: None,
                        retry: None,
                    };
                    Poll::Pending
                }
            },
            NavigationTransactionPhase::Departure { input, job, retry } => {
                self.step_departure_navigation_transaction(transaction, input, job, retry)
            }
            NavigationTransactionPhase::Http(mut fetch) => {
                let progress = fetch.poll();
                if let Some((method, url)) = fetch.take_started() {
                    self.trace.push(
                        Revision(self.revision),
                        None,
                        TraceEventKind::RequestStarted { method, url },
                    );
                }
                match progress {
                    Poll::Pending => {
                        transaction.phase = NavigationTransactionPhase::Http(fetch);
                        Poll::Pending
                    }
                    Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
                    Poll::Ready(Ok(response)) => {
                        self.navigation_transaction_live(transaction)?;
                        if matches!(response.headers.status, 204 | 205) {
                            let status = response.headers.status;
                            self.trace.push(
                                Revision(self.revision),
                                None,
                                TraceEventKind::ResponseReceived {
                                    url: response.current.to_string(),
                                    status,
                                    bytes: 0,
                                },
                            );
                            drop(response);
                            return Poll::Ready(self.with_navigation_transaction(
                                transaction,
                                |state, transaction| {
                                    state.finish_navigation_without_commit(
                                        transaction.requested_url.clone(),
                                        status,
                                    )
                                },
                            ));
                        }
                        let attribute = transaction.download.as_deref().filter(|_| {
                            self.executable.as_ref().is_some_and(|page| {
                                page.can_disclose_navigation_destination(response.current.as_str())
                            })
                        });
                        if let Some(filename) = crate::download::response_download_filename(
                            &response.headers.headers,
                            &response.current,
                            attribute,
                        ) {
                            #[cfg(unix)]
                            {
                                let creator =
                                    transaction.source_creator.clone().ok_or_else(|| {
                                        RuntimeError::Download(
                                            "download source document unavailable".into(),
                                        )
                                    })?;
                                let client = self.context_client.clone().ok_or_else(|| {
                                    RuntimeError::Download("download endpoint unavailable".into())
                                })?;
                                let media_type = oryn_common::mime_type::MimeType::from_headers(
                                    &response.headers.headers,
                                )
                                .map(|mime| mime.serialize())
                                .unwrap_or_else(|| "application/octet-stream".into());
                                let pending=navigation_fetch::PendingPolicy::start(client,crate::context_service::ContextServiceRequest::AuthorizePageDownload{page_id:self.page_id,creator,url:response.current.to_string()},Some(transaction.deadline))?;
                                transaction.phase =
                                    NavigationTransactionPhase::DownloadAuthorization {
                                        pending,
                                        response,
                                        filename,
                                        media_type,
                                    };
                                return Poll::Pending;
                            }
                            #[cfg(not(unix))]
                            {
                                let _ = filename;
                                return Poll::Ready(Err(RuntimeError::Download(
                                    "download endpoint unavailable".into(),
                                )));
                            }
                        }
                        let supplied = oryn_common::mime_type::MimeType::from_headers(
                            &response.headers.headers,
                        )
                        .map(|mime| mime.serialize());
                        let no_sniff = crate::network::response_no_sniff(&response.headers.headers);
                        let (response, operation) = response.split_body();
                        transaction.phase = NavigationTransactionPhase::Mime {
                            pending: crate::network::DocumentMimeSniff::new(
                                operation, supplied, no_sniff,
                            ),
                            response,
                        };
                        Poll::Pending
                    }
                }
            }
            NavigationTransactionPhase::Mime {
                mut pending,
                response,
            } => match pending.poll() {
                Poll::Pending => {
                    transaction.phase = NavigationTransactionPhase::Mime { pending, response };
                    Poll::Pending
                }
                Poll::Ready(Err(error)) => Poll::Ready(Err(RuntimeError::Network(error))),
                Poll::Ready(Ok((operation, computed_type))) => {
                    let response = response.restore_body(operation);
                    self.navigation_transaction_live(transaction)?;
                    if crate::download::computed_type_is_download(&computed_type) {
                        #[cfg(unix)]
                        {
                            let filename = crate::download::response_download_filename(
                                &response.headers.headers,
                                &response.current,
                                Some(""),
                            )
                            .ok_or_else(|| {
                                RuntimeError::Download(
                                    "computed download filename unavailable".into(),
                                )
                            })?;
                            let creator = transaction.source_creator.clone().ok_or_else(|| {
                                RuntimeError::Download(
                                    "download source document unavailable".into(),
                                )
                            })?;
                            let client = self.context_client.clone().ok_or_else(|| {
                                RuntimeError::Download("download endpoint unavailable".into())
                            })?;
                            let pending=navigation_fetch::PendingPolicy::start(client,crate::context_service::ContextServiceRequest::AuthorizePageDownload{page_id:self.page_id,creator,url:response.current.to_string()},Some(transaction.deadline))?;
                            transaction.phase = NavigationTransactionPhase::DownloadAuthorization {
                                pending,
                                response,
                                filename,
                                media_type: computed_type,
                            };
                            return Poll::Pending;
                        }
                        #[cfg(not(unix))]
                        return Poll::Ready(Err(RuntimeError::Download(
                            "download endpoint unavailable".into(),
                        )));
                    }
                    transaction.phase = NavigationTransactionPhase::Departure {
                        input: RootNavigationInput::Http(Box::new(response), computed_type),
                        job: None,
                        retry: None,
                    };
                    Poll::Pending
                }
            },
            #[cfg(unix)]
            NavigationTransactionPhase::DownloadAuthorization {
                mut pending,
                response,
                filename,
                media_type,
            } => match pending.poll() {
                Poll::Pending => {
                    transaction.phase = NavigationTransactionPhase::DownloadAuthorization {
                        pending,
                        response,
                        filename,
                        media_type,
                    };
                    Poll::Pending
                }
                Poll::Ready(Ok(
                    crate::context_service::ContextServiceResponse::PageDownloadAuthorized,
                )) => {
                    transaction.phase = NavigationTransactionPhase::DownloadBody {
                        broker: response.broker,
                        pending: navigation_fetch::PendingDownloadBody::new(
                            response.headers.operation,
                            Some(transaction.deadline),
                        ),
                        url: response.current.to_string(),
                        status: response.headers.status,
                        filename,
                        media_type,
                    };
                    Poll::Pending
                }
                Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
                Poll::Ready(Ok(_)) => Poll::Ready(Err(RuntimeError::ProtocolViolation(
                    "invalid retained download authorization".into(),
                ))),
            },
            #[cfg(unix)]
            NavigationTransactionPhase::DownloadBody {
                broker,
                mut pending,
                url,
                status,
                filename,
                media_type,
            } => match pending.poll() {
                Poll::Pending => {
                    transaction.phase = NavigationTransactionPhase::DownloadBody {
                        broker,
                        pending,
                        url,
                        status,
                        filename,
                        media_type,
                    };
                    Poll::Pending
                }
                Poll::Ready(result) => {
                    let bytes = result?;
                    self.navigation_transaction_live(transaction)?;
                    let creator = transaction
                        .source_creator
                        .clone()
                        .ok_or(RuntimeError::NavigationCancelled)?;
                    let client = self.context_client.clone().ok_or_else(|| {
                        RuntimeError::Download("download endpoint unavailable".into())
                    })?;
                    let pending = navigation_fetch::PendingPolicy::start_download_publication(
                        client,
                        crate::context_service::ContextServiceRequest::QuarantinePageDownload {
                            page_id: self.page_id,
                            creator,
                            url,
                            suggested_filename: filename,
                            media_type,
                            body_base64: BASE64.encode(bytes),
                        },
                        Some(transaction.deadline),
                    )?;
                    transaction.phase =
                        NavigationTransactionPhase::DownloadPublication { pending, status };
                    Poll::Pending
                }
            },
            #[cfg(unix)]
            NavigationTransactionPhase::BlobDownload {
                mut pending,
                filename,
            } => match pending.poll() {
                Poll::Pending => {
                    transaction.phase =
                        NavigationTransactionPhase::BlobDownload { pending, filename };
                    Poll::Pending
                }
                Poll::Ready(Ok(
                    crate::context_service::ContextServiceResponse::WorkerService {
                        result:
                            Ok(oryn_js::worker_wire::AgentServiceResponse::Blob { bytes, media_type }),
                    },
                )) => {
                    self.start_navigation_download_bytes(
                        transaction,
                        transaction.current.to_string(),
                        200,
                        filename,
                        media_type,
                        bytes,
                    )?;
                    Poll::Pending
                }
                Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
                Poll::Ready(Ok(
                    crate::context_service::ContextServiceResponse::WorkerService {
                        result: Err(error),
                    },
                )) => Poll::Ready(Err(RuntimeError::Download(error.to_string()))),
                Poll::Ready(Ok(_)) => Poll::Ready(Err(RuntimeError::ProtocolViolation(
                    "invalid blob download resolution".into(),
                ))),
            },
            #[cfg(unix)]
            NavigationTransactionPhase::DownloadBytesAuthorization {
                mut pending,
                url,
                status,
                filename,
                media_type,
                bytes,
            } => match pending.poll() {
                Poll::Pending => {
                    transaction.phase = NavigationTransactionPhase::DownloadBytesAuthorization {
                        pending,
                        url,
                        status,
                        filename,
                        media_type,
                        bytes,
                    };
                    Poll::Pending
                }
                Poll::Ready(Ok(
                    crate::context_service::ContextServiceResponse::PageDownloadAuthorized,
                )) => {
                    self.navigation_transaction_live(transaction)?;
                    let client = self.context_client.clone().ok_or_else(|| {
                        RuntimeError::Download("download endpoint unavailable".into())
                    })?;
                    let creator = transaction
                        .source_creator
                        .clone()
                        .ok_or(RuntimeError::NavigationCancelled)?;
                    let pending = navigation_fetch::PendingPolicy::start_download_publication(
                        client,
                        crate::context_service::ContextServiceRequest::QuarantinePageDownload {
                            page_id: self.page_id,
                            creator,
                            url,
                            suggested_filename: filename,
                            media_type,
                            body_base64: BASE64.encode(bytes),
                        },
                        Some(transaction.deadline),
                    )?;
                    transaction.phase =
                        NavigationTransactionPhase::DownloadPublication { pending, status };
                    Poll::Pending
                }
                Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
                Poll::Ready(Ok(_)) => Poll::Ready(Err(RuntimeError::ProtocolViolation(
                    "invalid download admission".into(),
                ))),
            },
            #[cfg(unix)]
            NavigationTransactionPhase::DownloadPublication {
                mut pending,
                status,
            } => match pending.poll() {
                Poll::Pending => {
                    transaction.phase =
                        NavigationTransactionPhase::DownloadPublication { pending, status };
                    Poll::Pending
                }
                Poll::Ready(Ok(
                    crate::context_service::ContextServiceResponse::DownloadQuarantined { .. },
                )) => Poll::Ready(self.with_navigation_transaction(
                    transaction,
                    |state, transaction| {
                        state.finish_navigation_without_commit(
                            transaction.requested_url.clone(),
                            status,
                        )
                    },
                )),
                Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
                Poll::Ready(Ok(_)) => Poll::Ready(Err(RuntimeError::ProtocolViolation(
                    "invalid retained download publication".into(),
                ))),
            },
            NavigationTransactionPhase::Publication {
                restored,
                mut result,
                identity,
                epoch,
                retry,
                mut milestone,
            } => {
                if retry.is_some_and(|time| time > Instant::now()) {
                    transaction.phase = NavigationTransactionPhase::Publication {
                        restored,
                        result,
                        identity,
                        epoch,
                        retry,
                        milestone,
                    };
                    return Poll::Pending;
                }
                let ready = (|| -> Result<bool, RuntimeError> {
                    if self.terminal_close_requested() {
                        return Ok(false);
                    }
                    self.poll_history_publications()?;
                    if self.terminal_close_requested() || !self.history_publications.is_empty() {
                        return Ok(false);
                    }
                    self.sync_executable_semantics()?;
                    if !self.refresh_page_task_layout()? {
                        return Ok(false);
                    }
                    let interactive = self
                        .executable
                        .as_ref()
                        .is_some_and(|page| page.document_interactive());
                    let loaded = self
                        .executable
                        .as_ref()
                        .is_some_and(|page| page.document_load_complete());
                    if !interactive {
                        return Ok(false);
                    }
                    if !self.lifecycle.contains(&LifecycleState::DomContentLoaded) {
                        self.lifecycle.push(LifecycleState::DomContentLoaded);
                    }
                    if let Some(emitter) = &transaction.emitter {
                        if milestone == 0 {
                            emitter
                                .publish(crate::navigation_control::NavigationEvent::Interactive {
                                    document_generation: self.document_generation,
                                })
                                .map_err(|error| RuntimeError::Network(error.to_string()))?;
                            milestone = 1;
                        }
                        if !loaded {
                            return Ok(false);
                        }
                        if milestone == 1 {
                            emitter
                                .publish(crate::navigation_control::NavigationEvent::Complete {
                                    document_generation: self.document_generation,
                                })
                                .map_err(|error| RuntimeError::Network(error.to_string()))?;
                            milestone = 2;
                        }
                        if self
                            .active_navigation
                            .as_ref()
                            .is_some_and(|active| active.same_job(emitter))
                        {
                            self.active_navigation = None;
                            self.navigation_committed = false;
                        }
                    }
                    if !loaded {
                        return Ok(false);
                    }
                    if !self.lifecycle.contains(&LifecycleState::Load) {
                        self.lifecycle.push(LifecycleState::Load);
                    }
                    Ok(true)
                })();
                if self
                    .executable
                    .as_ref()
                    .and_then(|page| page.form_history_root_identity())
                    != identity
                    || self
                        .executable
                        .as_ref()
                        .and_then(|page| page.native_navigation_load_epoch())
                        != epoch
                {
                    return Poll::Ready(Err(RuntimeError::NavigationCancelled));
                }
                match ready {
                    Ok(true) => {
                        if let Err(error)=self.record_snapshot(){return Poll::Ready(Err(error));}
                        result.revision = Revision(self.revision);
                        if !restored {
                            result.lifecycle = self.lifecycle.clone();
                        }
                        Poll::Ready(Ok(result))
                    }
                    _ => {
                        transaction.phase = NavigationTransactionPhase::Publication {
                            restored,
                            result,
                            identity,
                            epoch,
                            retry: Some(Instant::now() + Duration::from_millis(25)),
                            milestone,
                        };
                        Poll::Pending
                    }
                }
            }
            NavigationTransactionPhase::Finished => {
                Poll::Ready(Err(RuntimeError::NavigationCancelled))
            }
        }
    }

    // Follow only the two forward HTTP handoffs. A pending dependency or ACK
    // keeps its phase and yields immediately; departure retries are not polled
    // again. Each step retains the existing live-authority/deadline/checkpoint
    // checks, and at most one departure (which may invoke author code) can run.
    fn step_ready_navigation_transaction(
        &mut self,
        transaction: &mut NavigationTransaction,
    ) -> std::task::Poll<Result<NavigationResult, RuntimeError>> {
        for _ in 0..3 {
            let was_http = matches!(transaction.phase, NavigationTransactionPhase::Http(_));
            let was_mime = matches!(transaction.phase, NavigationTransactionPhase::Mime { .. });
            let progress = self.step_navigation_transaction(transaction);
            let forward = (was_http
                && matches!(transaction.phase, NavigationTransactionPhase::Mime { .. }))
                || (was_mime
                    && matches!(
                        transaction.phase,
                        NavigationTransactionPhase::Departure { .. }
                    ));
            if progress.is_ready() || !forward {
                return progress;
            }
        }
        std::task::Poll::Pending
    }

    fn poll_navigation_transaction(&mut self) -> Option<PendingNavigationReply> {
        #[cfg(unix)]
        if self.script_close_owns_departure() {
            return None;
        }
        if self.polling_navigation_transaction {
            return None;
        }
        if let Some(mut settlement) = self.settling_navigation.take() {
            let progress = if Instant::now() >= settlement.deadline {
                std::task::Poll::Ready(Err(crate::navigation_control::NavigationStop::Timeout))
            } else {
                settlement
                    .ack
                    .as_ref()
                    .expect("retained final navigation ACK")
                    .poll()
            };
            match progress {
                std::task::Poll::Pending => self.settling_navigation = Some(settlement),
                std::task::Poll::Ready(result) => {
                    if let Err(error) = result
                        && settlement.result.is_ok()
                    {
                        settlement.result = Err(RuntimeError::NavigationStopped(error));
                    }
                    if let Some(reply) = self.deliver_navigation_settlement(settlement) {
                        return Some(reply);
                    }
                }
            }
        }
        let mut transaction = self.pending_navigation_transaction.take()?;
        self.polling_navigation_transaction = true;
        let progress = self.step_ready_navigation_transaction(&mut transaction);
        self.polling_navigation_transaction = false;
        let result = match progress {
            std::task::Poll::Pending => {
                self.pending_navigation_transaction = Some(transaction);
                return None;
            }
            std::task::Poll::Ready(result) => result,
        };
        #[cfg(unix)]
        if result.is_err() {
            self.retain_prerender_recovery(&mut transaction);
        }
        if let Some(reply) = transaction.admission_reply.take() {
            let _ = reply.send(
                result
                    .as_ref()
                    .map(|_| ())
                    .map_err(|error| RuntimeError::Network(error.to_string())),
            );
        }
        let result = result.and_then(|result| {
            if let Some(emitter) = transaction
                .emitter
                .as_ref()
                .filter(|_| self.active_navigation.is_some())
            {
                use crate::navigation_control::NavigationEvent;
                if !self.navigation_committed {
                    emitter
                        .publish(NavigationEvent::Committed {
                            document_generation: self.document_generation,
                            url: self.url.clone(),
                        })
                        .map_err(|error| RuntimeError::Network(error.to_string()))?;
                    self.navigation_committed = true;
                }
                if self.document_generation == transaction.source_generation {
                    emitter
                        .publish(NavigationEvent::Interactive {
                            document_generation: self.document_generation,
                        })
                        .map_err(|error| RuntimeError::Network(error.to_string()))?;
                    emitter
                        .publish(NavigationEvent::Complete {
                            document_generation: self.document_generation,
                        })
                        .map_err(|error| RuntimeError::Network(error.to_string()))?;
                    self.active_navigation = None;
                    self.navigation_committed = false;
                }
            }
            if result.document_generation != transaction.source_generation
                && result.document_generation == self.document_generation
            {
                if matches!(transaction.history, HistoryUpdate::Reload)
                    && self.url == transaction.requested_url
                {
                    let current = self
                        .history_index
                        .and_then(|index| self.history.get(index))
                        .cloned();
                    if let Some(entry) = current.filter(|entry| {
                        entry
                            .navigation
                            .as_ref()
                            .zip(
                                transaction
                                    .history_entry
                                    .as_ref()
                                    .and_then(|old| old.navigation.as_ref()),
                            )
                            .is_some_and(|(entry, old)| entry.key == old.key)
                    }) {
                        self.restore_navigation_entry(&entry)?;
                    }
                } else if let Some(entry) = transaction.history_entry.as_ref() {
                    self.restore_navigation_entry(entry)?;
                }
            }
            Ok(result)
        });
        let result = self.settle_failed_navigation(
            transaction.operation.as_ref(),
            transaction.source_generation,
            result,
        );
        if result.is_err()
            && let Some(emitter) = transaction.emitter.as_ref()
        {
            let reason = emitter
                .checkpoint()
                .err()
                .unwrap_or_else(|| result.as_ref().err().unwrap().navigation_stop());
            let _ = emitter.publish(crate::navigation_control::NavigationEvent::Stopped(reason));
        }
        let mut settlement = NavigationSettlement {
            result,
            reply: transaction.reply.take(),
            source_generation: transaction.source_generation,
            deadline: transaction.deadline,
            identity: self
                .executable
                .as_ref()
                .and_then(|page| page.form_history_root_identity()),
            load_epoch: self
                .executable
                .as_ref()
                .and_then(|page| page.native_navigation_load_epoch()),
            ack: None,
        };
        if let Some(emitter) = &transaction.emitter {
            match emitter.flush_pending() {
                Ok(ack) => {
                    settlement.ack = Some(ack);
                    if let Some(previous) = self.settling_navigation.replace(settlement)
                        && let Some(reply) = previous.reply
                    {
                        reply.complete(self, Err(RuntimeError::NavigationCancelled));
                    }
                    return None;
                }
                Err(error) => {
                    if settlement.result.is_ok() {
                        settlement.result = Err(RuntimeError::NavigationStopped(error));
                    }
                }
            }
        }
        self.deliver_navigation_settlement(settlement)
    }
}

#[cfg(feature = "v8-host")]
impl PageState {
    fn begin_command_navigation(
        &mut self,
        url: String,
        post: Option<oryn_js::navigation::PostResource>,
        referrer_policy: Option<String>,
        form_authority: Option<oryn_js::navigation::FormNavigationAuthority>,
        service_authority: Option<oryn_js::navigation::ServiceNavigationAdmission>,
    ) -> Result<NavigationProgress, RuntimeError> {
        let Some(service_authority) = service_authority else {
            return self.begin_browser_navigation(
                url,
                HistoryUpdate::Push,
                post,
                referrer_policy,
                form_authority,
            );
        };
        if post.is_some() || form_authority.is_some() || referrer_policy.is_some() {
            return Err(RuntimeError::ProtocolViolation(
                "service navigation cannot carry window form or referrer overrides".into(),
            ));
        }
        #[cfg(all(unix, feature = "v8-host"))]
        {
            let previous = self.service_navigation_authority.replace(service_authority);
            let result = self.begin_browser_navigation(url, HistoryUpdate::Push, None, None, None);
            self.service_navigation_authority = previous;
            result
        }
        #[cfg(not(all(unix, feature = "v8-host")))]
        {
            let _ = (url, service_authority);
            Err(RuntimeError::ProtocolViolation(
                "service navigation source unavailable".into(),
            ))
        }
    }

    fn begin_browser_navigation(
        &mut self,
        requested_url: String,
        history_update: HistoryUpdate,
        post: Option<oryn_js::navigation::PostResource>,
        referrer_policy: Option<String>,
        form_authority: Option<oryn_js::navigation::FormNavigationAuthority>,
    ) -> Result<NavigationProgress, RuntimeError> {
        #[cfg(unix)]
        if self.script_close_departure_completed() {
            return Err(RuntimeError::Closed);
        }
        #[cfg(feature = "v8-host")]
        if !matches!(history_update, HistoryUpdate::Preserve(_))
            && let Some(executable) = self.executable.as_mut()
        {
            // Browser-originated loads must dispatch navigate before fetching.
            // Script/default requests enter navigate_script after admission.
            let admission = executable
                .admit_native_navigation(
                    &requested_url,
                    matches!(history_update, HistoryUpdate::Replace),
                    matches!(history_update, HistoryUpdate::Reload),
                )
                .map_err(map_host_error)?;
            match admission {
                oryn_js::navigation_api::NativeNavigationAdmission::Handled => {
                    self.sync_executable_semantics()?;
                    return Ok(NavigationProgress::Ready(NavigationResult {
                        page_id: self.page_id,
                        requested_url,
                        final_url: self.url.clone(),
                        title: self
                            .document
                            .as_ref()
                            .map(document_title)
                            .unwrap_or_default(),
                        status: 200,
                        document_generation: self.document_generation,
                        revision: Revision(self.revision),
                        lifecycle: self.lifecycle.clone(),
                        challenge: self.last_challenge.clone(),
                    }));
                }
                oryn_js::navigation_api::NativeNavigationAdmission::Proceed {
                    operation,
                    destination,
                } => {
                    let previous = std::mem::replace(
                        &mut self.script_navigation_destination,
                        destination.map(|entry| *entry),
                    );
                    let previous_operation =
                        std::mem::replace(&mut self.script_navigation_operation, operation.clone());
                    let source_generation = self.document_generation;
                    let result = self.begin_navigation_resource(
                        requested_url,
                        history_update,
                        post,
                        referrer_policy,
                        form_authority,
                        None,
                    );
                    self.script_navigation_destination = previous;
                    self.script_navigation_operation = previous_operation;
                    return self.settle_navigation_progress(
                        operation.as_ref(),
                        source_generation,
                        result,
                    );
                }
            }
        }
        self.begin_navigation_resource(
            requested_url,
            history_update,
            post,
            referrer_policy,
            form_authority,
            None,
        )
    }

    fn begin_script_navigation(
        &mut self,
        request: oryn_js::v8_bindings::LocationNavigation,
        history: HistoryUpdate,
    ) -> Result<NavigationProgress, RuntimeError> {
        let observer = request.execution_observer.clone();
        let result = self.begin_script_navigation_inner(request, history);
        crate::native_timing::point(
            "script_navigation_actor_admission",
            serde_json::json!({
                "page_id": self.page_id.0,
                "accepted": result.is_ok(),
                "canceled": matches!(&result, Err(RuntimeError::NavigationCancelled)),
                "completion_inferred": false
            }),
        );
        if let (Err(error), Some(observer)) = (&result, observer) {
            observer.complete(if matches!(error, RuntimeError::NavigationCancelled) {
                oryn_js::navigation::NavigationExecutionOutcome::Canceled
            } else {
                oryn_js::navigation::NavigationExecutionOutcome::Failed {
                    message: error.to_string(),
                }
            });
        }
        result
    }

    fn begin_script_navigation_inner(
        &mut self,
        mut request: oryn_js::v8_bindings::LocationNavigation,
        history: HistoryUpdate,
    ) -> Result<NavigationProgress, RuntimeError> {
        let history = if !request.reload {
            request
                .navigation_destination
                .as_ref()
                .and_then(|destination| {
                    self.history.iter().position(|entry| {
                        entry.navigation.as_ref().is_some_and(|entry| {
                            entry.id == destination.id && entry.key == destination.key
                        })
                    })
                })
                .map(HistoryUpdate::Preserve)
                .unwrap_or(history)
        } else {
            history
        };
        if request
            .navigation_destination
            .as_ref()
            .is_some_and(|entry| entry.url != request.url)
        {
            return Err(RuntimeError::ProtocolViolation(
                "Navigation API destination does not match its accepted request".into(),
            ));
        }
        if request.reload && request.post.is_none() {
            request.post = self
                .history_index
                .and_then(|index| self.history.get(index))
                .and_then(|entry| entry.post.clone());
        }
        if request.reload && request.form_authority.is_none() {
            request.form_authority = self
                .history_index
                .and_then(|index| self.history.get(index))
                .and_then(|entry| entry.form_authority.clone());
        }
        let operation = request.navigation_operation.take();
        {
            let executable = self
                .executable
                .as_mut()
                .ok_or(RuntimeError::NavigationCancelled)?;
            let token = operation.as_ref().ok_or_else(|| {
                RuntimeError::ProtocolViolation(
                    "queued navigation omitted its admission identity".into(),
                )
            })?;
            if !executable
                .native_navigation_matches(token)
                .map_err(map_host_error)?
            {
                return Err(RuntimeError::NavigationCancelled);
            }
        }
        let previous_destination = std::mem::replace(
            &mut self.script_navigation_destination,
            request.navigation_destination,
        );
        let previous_operation =
            std::mem::replace(&mut self.script_navigation_operation, operation.clone());
        let previous_download =
            std::mem::replace(&mut self.script_navigation_download, request.download);
        let source_generation = self.document_generation;
        let result = self.begin_navigation_resource(
            request.url,
            history,
            request.post,
            request.referrer_policy,
            request.form_authority,
            request.script_authority,
        );
        self.script_navigation_destination = previous_destination;
        self.script_navigation_operation = previous_operation;
        self.script_navigation_download = previous_download;
        let result = self.settle_navigation_progress(operation.as_ref(), source_generation, result);
        if let Some(observer) = request.execution_observer.take() {
            self.retain_navigation_execution(observer, source_generation, &result);
        }
        result
    }

    #[cfg(feature = "v8-host")]
    fn begin_queued_history_navigation(
        &mut self,
        mut task: oryn_js::v8_bindings::HistoryTraversalTask,
    ) -> Result<Option<NavigationProgress>, RuntimeError> {
        if !task.is_current() {
            task.acknowledge();
            return Ok(None);
        }
        if !task.is_runnable() {
            return Ok(None);
        }
        if task.delta() == 0 {
            task.acknowledge();
            return self.begin_refresh_navigation().map(Some);
        }
        self.executable
            .as_mut()
            .ok_or(RuntimeError::EvaluationUnavailable)?
            .run_history_task(task, None)
            .map_err(map_host_error)?;
        Ok(None)
    }

    fn begin_history_navigation(
        &mut self,
        delta: isize,
    ) -> Result<NavigationProgress, RuntimeError> {
        #[cfg(feature = "v8-host")]
        if let Some(executable) = self.executable.as_mut() {
            let receipt = executable
                .begin_joint_history_delta(delta as i64)
                .map_err(map_host_error)?
                .ok_or(RuntimeError::HistoryBoundary)?;
            return Ok(NavigationProgress::Joint(
                receipt,
                NavigationResult {
                    page_id: self.page_id,
                    requested_url: self.url.clone(),
                    final_url: self.url.clone(),
                    title: self
                        .document
                        .as_ref()
                        .map(document_title)
                        .unwrap_or_default(),
                    status: 200,
                    document_generation: self.document_generation,
                    revision: Revision(self.revision),
                    lifecycle: self.lifecycle.clone(),
                    challenge: self.last_challenge.clone(),
                },
            ));
        }
        let current = self.history_index.ok_or(RuntimeError::HistoryBoundary)? as isize;
        let next = current
            .checked_add(delta)
            .ok_or(RuntimeError::HistoryBoundary)?;
        if next < 0 || next >= self.history.len() as isize {
            return Err(RuntimeError::HistoryBoundary);
        }
        let requested = self.history[next as usize].clone();
        #[cfg(feature = "v8-host")]
        if let (Some(executable), Some(destination)) =
            (self.executable.as_mut(), requested.navigation.as_ref())
        {
            let admission = executable
                .admit_native_history_traversal(destination.clone())
                .map_err(map_host_error)?;
            match admission {
                oryn_js::navigation_api::NativeNavigationAdmission::Handled => {
                    self.sync_executable_semantics()?;
                    return Ok(NavigationProgress::Ready(NavigationResult {
                        page_id: self.page_id,
                        requested_url: requested.url,
                        final_url: self.url.clone(),
                        title: self
                            .document
                            .as_ref()
                            .map(document_title)
                            .unwrap_or_default(),
                        status: 200,
                        document_generation: self.document_generation,
                        revision: Revision(self.revision),
                        lifecycle: self.lifecycle.clone(),
                        challenge: self.last_challenge.clone(),
                    }));
                }
                oryn_js::navigation_api::NativeNavigationAdmission::Proceed {
                    operation,
                    destination,
                } => {
                    let previous = std::mem::replace(
                        &mut self.script_navigation_destination,
                        destination.map(|entry| *entry),
                    );
                    let previous_operation =
                        std::mem::replace(&mut self.script_navigation_operation, operation.clone());
                    let source_generation = self.document_generation;
                    let result = self.begin_history_navigation_to(next as usize, requested);
                    self.script_navigation_destination = previous;
                    self.script_navigation_operation = previous_operation;
                    return self.settle_navigation_progress(
                        operation.as_ref(),
                        source_generation,
                        result,
                    );
                }
            }
        }
        self.begin_history_navigation_to(next as usize, requested)
    }

    fn begin_refresh_navigation(&mut self) -> Result<NavigationProgress, RuntimeError> {
        if self.url.is_empty() {
            return Err(RuntimeError::HistoryBoundary);
        }
        let post = self
            .history_index
            .and_then(|index| self.history.get(index))
            .and_then(|entry| entry.post.clone());
        let authority = self
            .history_index
            .and_then(|index| self.history.get(index))
            .and_then(|entry| entry.form_authority.clone());
        self.begin_browser_navigation(
            self.url.clone(),
            HistoryUpdate::Reload,
            post,
            None,
            authority,
        )
    }

    fn settle_navigation_progress(
        &mut self,
        operation: Option<&oryn_js::navigation_api::NativeNavigationToken>,
        generation: u64,
        result: Result<NavigationProgress, RuntimeError>,
    ) -> Result<NavigationProgress, RuntimeError> {
        match result {
            Ok(result) => Ok(result),
            Err(error) => self
                .settle_failed_navigation(operation, generation, Err(error))
                .map(NavigationProgress::Ready),
        }
    }
    fn begin_history_navigation_to(
        &mut self,
        next: usize,
        requested: NavigationHistoryEntry,
    ) -> Result<NavigationProgress, RuntimeError> {
        let next = self.resolve_history_target(next, requested.navigation.as_ref())?;
        self.begin_navigation_resource(
            requested.url,
            HistoryUpdate::Preserve(next),
            requested.post,
            None,
            requested.form_authority,
            None,
        )
    }
    fn restore_navigation_entry(
        &mut self,
        requested: &NavigationHistoryEntry,
    ) -> Result<(), RuntimeError> {
        let owns_entry = self
            .history_index
            .and_then(|index| self.history.get(index))
            .and_then(|entry| entry.navigation.as_ref())
            .zip(requested.navigation.as_ref())
            .is_some_and(|(entry, expected)| entry.key == expected.key && entry.id == expected.id);
        if owns_entry
            && self.url == requested.url
            && let Some(executable) = self.executable.as_mut()
        {
            let identity = executable.form_history_root_identity();
            executable
                .restore_form_history(&requested.form_state)
                .map_err(map_host_error)?;
            if identity == executable.form_history_root_identity() {
                executable
                    .restore_child_form_history(&requested.frame_form_state)
                    .map_err(map_host_error)?;
            }
            self.sync_executable_semantics()?;
            self.refresh_page_task_layout()?;
        }
        Ok(())
    }
}

#[cfg(all(feature = "v8-host", unix))]
impl PageState {
    fn launch_javascript_navigation(
        &mut self,
        transaction: &mut NavigationTransaction,
    ) -> Result<Option<NavigationResult>, RuntimeError> {
        let admission = transaction
            .script_authority
            .clone()
            .ok_or(RuntimeError::NavigationCancelled)?;
        if transaction.network_source.is_none()
            || transaction.post.is_some()
            || transaction.download.is_some()
        {
            return Err(RuntimeError::NavigationCancelled);
        }
        let completion = self
            .executable
            .as_mut()
            .ok_or(RuntimeError::NavigationCancelled)?
            .run_javascript_navigation(transaction.current.as_str())
            .map_err(map_host_error)?;
        self.navigation_transaction_live(transaction)?;
        self.sync_executable_semantics()?;
        let Some(html) = completion else {
            return self.with_navigation_transaction(transaction, |state, transaction| {
                state
                    .finish_navigation_without_commit(transaction.requested_url.clone(), 200)
                    .map(Some)
            });
        };
        if html.len() > 8 * 1024 * 1024 {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "JavaScript navigation response bytes".into(),
            });
        }
        let executable = self
            .executable
            .as_mut()
            .ok_or(RuntimeError::NavigationCancelled)?;
        if !executable
            .dispatch_before_unload()
            .map_err(map_host_error)?
        {
            return Err(RuntimeError::NavigationCancelled);
        }
        self.navigation_transaction_live(transaction)?;
        let url = self.url.clone();
        let javascript_url = transaction.current.to_string();
        transaction.current =
            Url::parse(&url).map_err(|error| RuntimeError::Network(error.to_string()))?;
        transaction.history = HistoryUpdate::Replace;
        transaction.destination = None;
        transaction.phase =
            NavigationTransactionPhase::Local(navigation_local::PendingLocal::start(
                self.context_client
                    .clone()
                    .ok_or(RuntimeError::NavigationCancelled)?,
                self.page_id,
                crate::context_service::ContextServiceRequest::PrepareJavascriptNavigation {
                    page_id: self.page_id,
                    admission,
                    javascript_url,
                    url,
                    html,
                },
                transaction.deadline,
            )?);
        Ok(None)
    }

    fn start_local_navigation_transaction(
        &mut self,
        transaction: &mut NavigationTransaction,
        blob: bool,
    ) -> Result<Option<NavigationResult>, RuntimeError> {
        use crate::context_service::ContextServiceRequest as Request;
        let client = self
            .context_client
            .clone()
            .ok_or_else(|| RuntimeError::Network("local navigation endpoint unavailable".into()))?;
        let url = transaction.current.to_string();
        let blank =
            transaction.current.scheme() == "about" && transaction.current.path() == "blank";
        let request = if let Some(admission) = transaction.service_authority.clone() {
            if blank {
                return Err(RuntimeError::ProtocolViolation(
                    "service navigation cannot target about:blank".into(),
                ));
            }
            Request::PrepareServiceLocalNavigation {
                page_id: self.page_id,
                admission,
                url,
                blob,
            }
        } else if blank {
            let base_url = self
                .executable
                .as_mut()
                .map(|page| page.native_navigation_base_url().map_err(map_host_error))
                .transpose()?;
            Request::PrepareBlankNavigation {
                page_id: self.page_id,
                creator: transaction.source_creator.clone(),
                url,
                base_url,
            }
        } else if let Some(creator) = transaction.source_creator.clone() {
            if blob {
                Request::PrepareBlobNavigation {
                    page_id: self.page_id,
                    creator,
                    url,
                }
            } else {
                Request::PrepareLocalDataNavigation {
                    page_id: self.page_id,
                    creator,
                    url,
                }
            }
        } else {
            Request::PrepareInitialLocalNavigation {
                page_id: self.page_id,
                url,
                blob,
            }
        };
        transaction.phase =
            NavigationTransactionPhase::Local(navigation_local::PendingLocal::start(
                client,
                self.page_id,
                request,
                transaction.deadline,
            )?);
        Ok(None)
    }
}

#[cfg(all(feature = "v8-host", unix))]
impl PageState {
    fn start_navigation_download_bytes(
        &mut self,
        transaction: &mut NavigationTransaction,
        url: String,
        status: u16,
        filename: String,
        media_type: String,
        bytes: Vec<u8>,
    ) -> Result<Option<NavigationResult>, RuntimeError> {
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "download bytes".into(),
            });
        }
        let creator = transaction
            .source_creator
            .clone()
            .ok_or(RuntimeError::NavigationCancelled)?;
        let client = self
            .context_client
            .clone()
            .ok_or_else(|| RuntimeError::Download("download endpoint unavailable".into()))?;
        let pending = navigation_fetch::PendingPolicy::start(
            client,
            crate::context_service::ContextServiceRequest::AuthorizePageDownload {
                page_id: self.page_id,
                creator,
                url: url.clone(),
            },
            Some(transaction.deadline),
        )?;
        transaction.phase = NavigationTransactionPhase::DownloadBytesAuthorization {
            pending,
            url,
            status,
            filename,
            media_type,
            bytes,
        };
        Ok(None)
    }
}

// Direct source fixtures keep a synchronous adapter around the same state
// machine. Production command/task producers call begin_* and never enter it.
#[cfg(all(test, feature = "v8-host"))]
impl PageState {
    fn navigate_resource(
        &mut self,
        requested_url: String,
        history: HistoryUpdate,
        post: Option<oryn_js::navigation::PostResource>,
        referrer_policy: Option<String>,
        form_authority: Option<oryn_js::navigation::FormNavigationAuthority>,
        script_authority: Option<oryn_js::navigation::ScriptNavigationAdmission>,
    ) -> Result<NavigationResult, RuntimeError> {
        let progress = self.begin_navigation_resource(
            requested_url,
            history,
            post,
            referrer_policy,
            form_authority,
            script_authority,
        )?;
        let NavigationProgress::Pending(id) = progress else {
            let NavigationProgress::Ready(result) = progress else {
                unreachable!()
            };
            return Ok(result);
        };
        let (sender, mut receiver) = oneshot::channel();
        self.pending_navigation_transaction
            .as_mut()
            .filter(|transaction| transaction.id == id)
            .ok_or(RuntimeError::NavigationCancelled)?
            .reply = Some(NavigationReply::Navigation(sender));
        let mut readiness = None;
        loop {
            if let Some(pending) = self.poll_navigation_transaction() {
                readiness = Some(pending);
            }
            match self.poll_page_tasks() {
                Ok(()) => {},
                // Projection readiness is a retained actor turn, not a
                // terminal navigation failure. Keep the original reply.
                Err(RuntimeError::PageTaskYield) if !self.terminal_close_requested() => {},
                Err(error) => return Err(error),
            }
            if let Some(pending) = self.pending_committed_navigation_reply.take() {
                readiness = Some(pending);
            }
            if let Some(pending) = readiness.take() {
                readiness = self.poll_navigation_reply(pending);
            }
            match receiver.try_recv() {
                Ok(result) => return result,
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    return Err(RuntimeError::NavigationCancelled);
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
            }
            std::thread::park_timeout(Duration::from_millis(1));
        }
    }
}

#[cfg(all(test, feature = "v8-host", feature = "in-process-probe"))]
mod actor_navigation_continuation_tests {
    use super::*;
    use crate::network::{NetworkError, NetworkEvent, NetworkOperation, NetworkResponse};
    type HeldNetworkRequests = Arc<Mutex<Vec<(u64, tokio::sync::mpsc::Sender<NetworkEvent>)>>>;

    struct HeldHeaders {
        started: tokio::sync::mpsc::UnboundedSender<String>,
        held: HeldNetworkRequests,
        cancelled: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl NetworkBroker for HeldHeaders {
        fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            unreachable!("actor uses owned operations")
        }
        fn start_operation(
            &self,
            request: NetworkRequest,
        ) -> Result<NetworkOperation, NetworkError> {
            let (sender, events) = tokio::sync::mpsc::channel(8);
            if request.url.ends_with("/fast") {
                sender
                    .try_send(NetworkEvent::Headers {
                        response_policy: None,
                        timing: None,
                        request_id: request.request_id,
                        status: 200,
                        headers: vec![("content-type".into(), "text/html".into())],
                        content_length: None,
                        upload: None,
                    })
                    .unwrap();
                sender
                    .try_send(NetworkEvent::Chunk {
                        request_id: request.request_id,
                        bytes: b"<title>Fast destination</title>".to_vec().into(),
                    })
                    .unwrap();
                sender
                    .try_send(NetworkEvent::Complete {
                        encoded_body_size: None,
                        request_id: request.request_id,
                    })
                    .unwrap();
            } else {
                self.held.lock().unwrap().push((request.request_id, sender));
            }
            let _ = self.started.send(request.url);
            Ok(NetworkOperation {
                body_demand_pending: Default::default(),
                prefetched: Default::default(),
                request_id: request.request_id,
                events,
                demand: None,
            })
        }
        fn cancel_operation(&self, _: u64) -> Result<(), NetworkError> {
            self.cancelled.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
    type NavigationFixture = (
        BrowserContext,
        PageHandle,
        tokio::sync::mpsc::UnboundedReceiver<String>,
        HeldNetworkRequests,
        Arc<std::sync::atomic::AtomicUsize>,
    );

    fn fixture(config: BrowserConfig) -> NavigationFixture {
        let (started, receiver) = tokio::sync::mpsc::unbounded_channel();
        let held = Arc::new(Mutex::new(Vec::new()));
        let cancelled = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let context = Browser::in_process_with_config(config).new_context_with_broker(
            ContextOptions::default(),
            SharedNetworkBroker::new(HeldHeaders {
                started,
                held: held.clone(),
                cancelled: cancelled.clone(),
            }),
        );
        let page = context.new_page();
        (context, page, receiver, held, cancelled)
    }
    #[tokio::test(flavor = "current_thread")]
    async fn held_headers_yield_to_evaluate_stop_and_close_commands() {
        for terminal in ["stop", "close"] {
            let (_context, page, mut started, _held, _cancelled) =
                fixture(BrowserConfig::default());
            page.load_html("https://actor.test/source", "<title>Original</title>")
                .await
                .unwrap();
            let mut navigation = Box::pin(page.goto("https://actor.test/held"));
            tokio::select! {result=&mut navigation=>panic!("held response settled: {result:?}"),url=started.recv()=>assert_eq!(url.as_deref(),Some("https://actor.test/held")),_=tokio::time::sleep(Duration::from_secs(5))=>panic!("actor never admitted navigation")}
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(1), page.evaluate("document.title"))
                    .await
                    .unwrap()
                    .unwrap(),
                "Original"
            );
            if terminal == "stop" {
                tokio::time::timeout(Duration::from_secs(1), page.evaluate("stop();'stopped'"))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(
                    tokio::time::timeout(Duration::from_secs(2), navigation)
                        .await
                        .unwrap()
                        .is_err()
                );
                assert_eq!(page.evaluate("document.title").await.unwrap(), "Original");
                page.close().await.unwrap();
            } else {
                tokio::time::timeout(Duration::from_secs(1), page.close())
                    .await
                    .unwrap()
                    .unwrap();
                assert!(
                    tokio::time::timeout(Duration::from_secs(2), navigation)
                        .await
                        .unwrap()
                        .is_err()
                );
            }
        }
    }
    #[tokio::test(flavor = "current_thread")]
    async fn newer_navigation_commits_while_older_headers_are_held() {
        let (context, page, mut started, held, _cancelled) = fixture(BrowserConfig::default());
        page.load_html("https://actor.test/source", "<title>Original</title>")
            .await
            .unwrap();
        let mut first = Box::pin(page.goto("https://actor.test/held"));
        tokio::select! {result=&mut first=>panic!("held response settled: {result:?}"),_=started.recv()=>{},_=tokio::time::sleep(Duration::from_secs(5))=>panic!("missing start")}
        let result =
            tokio::time::timeout(Duration::from_secs(5), page.goto("https://actor.test/fast"))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(result.title, "Fast destination");
        assert!(first.await.is_err());
        // Even a transport which reports a late old response cannot replace the
        // exact newer native document after its receiver has been abandoned.
        for (id, sender) in held.lock().unwrap().iter() {
            let _ = sender.try_send(NetworkEvent::Headers {
                response_policy: None,
                timing: None,
                request_id: *id,
                status: 200,
                headers: Vec::new(),
                content_length: None,
                upload: None,
            });
        }
        assert_eq!(
            page.evaluate("document.title").await.unwrap(),
            "Fast destination"
        );
        page.close().await.unwrap();
        drop(context);
    }
    #[tokio::test(flavor = "current_thread")]
    async fn initial_documentless_actor_still_polls_header_deadline() {
        let (_context, page, _started, _held, _cancelled) = fixture(BrowserConfig {
            navigation_timeout: Duration::from_millis(50),
            ..Default::default()
        });
        let result =
            tokio::time::timeout(Duration::from_secs(5), page.goto("https://actor.test/held"))
                .await
                .unwrap();
        assert!(matches!(
            result,
            Err(RuntimeError::WorkerLimitExceeded { .. })
        ));
        page.close().await.unwrap();
    }
}

#[cfg(feature = "v8-host")]
struct NavigationSettlement {
    result: Result<NavigationResult, RuntimeError>,
    reply: Option<NavigationReply>,
    source_generation: u64,
    deadline: Instant,
    identity: Option<(oryn_js::realm::RealmId, u64, u64)>,
    load_epoch: Option<u64>,
    ack: Option<crate::navigation_control::PendingNavigationAck>,
}
#[cfg(feature = "v8-host")]
impl PageState {
    fn deliver_navigation_settlement(
        &mut self,
        mut settlement: NavigationSettlement,
    ) -> Option<PendingNavigationReply> {
        if settlement
            .result
            .as_ref()
            .is_ok_and(|result| result.document_generation != self.document_generation)
            || settlement.identity
                != self
                    .executable
                    .as_ref()
                    .and_then(|page| page.form_history_root_identity())
            || settlement.load_epoch
                != self
                    .executable
                    .as_ref()
                    .and_then(|page| page.native_navigation_load_epoch())
        {
            settlement.result = Err(RuntimeError::NavigationCancelled);
        }
        // The document can finish while its final retained ACK is pending.
        // Refresh the current receipt just as the deferred readiness path does.
        if let Ok(result) = &mut settlement.result
            && result.document_generation != settlement.source_generation
            && self
                .executable
                .as_ref()
                .is_some_and(|page| page.document_load_complete())
        {
            result.final_url = self.url.clone();
            result.title = self
                .document
                .as_ref()
                .map(document_title)
                .unwrap_or_default();
            result.revision = Revision(self.revision);
            result.lifecycle = self.lifecycle.clone();
            result.challenge = self.last_challenge.clone();
        }
        #[cfg(test)]
        if let Ok(result) = &settlement.result
            && result.document_generation != settlement.source_generation
            && !result.lifecycle.contains(&LifecycleState::Load)
            && self
                .executable
                .as_ref()
                .is_some_and(|page| page.document_load_complete())
        {
            eprintln!(
                "repair44 navigation ready-result mismatch {}",
                repair44_bounded_diagnostic(format_args!(
                    "url={:?} result_lifecycle={:?} current_lifecycle={:?} response_pending={} identity={:?} load_epoch={:?}",
                    repair44_diagnostic_prefix(&self.url),
                    result.lifecycle,
                    self.lifecycle,
                    self.pending_navigation_response.is_some(),
                    settlement.identity,
                    settlement.load_epoch
                ))
            );
        }
        if let Some(reply) = settlement.reply.take() {
            match settlement.result {
                Ok(result)
                    if !matches!(&reply, NavigationReply::Controlled(_))
                        && result.document_generation != settlement.source_generation
                        && (!self.lifecycle.contains(&LifecycleState::Load)
                            || self
                                .executable
                                .as_ref()
                                .is_some_and(|page| !page.document_load_complete())) =>
                {
                    Some(PendingNavigationReply {
                        joint_interactive_published: false,
                        joint_eager_reply: false,
                        joint_checkpoint: None,
                        joint_publication: None,
                        joint: None,
                        generation: result.document_generation,
                        load_epoch: settlement.load_epoch,
                        identity: settlement.identity,
                        deadline: settlement.deadline,
                        result,
                        reply,
                    })
                }
                result => {
                    reply.complete(self, result);
                    None
                }
            }
        } else {
            if let Err(error) = settlement.result {
                self.trace.push(
                    Revision(self.revision),
                    None,
                    TraceEventKind::Console {
                        level: "error".into(),
                        message: format!("navigation: {error}"),
                    },
                );
            }
            None
        }
    }
}

#[cfg(feature = "v8-host")]
impl PageState {
    fn poll_navigation_control_checkpoint(&mut self) -> Result<(), RuntimeError> {
        if let Some((emitter, ack)) = self.navigation_control_checkpoint.take()
            && self
                .active_navigation
                .as_ref()
                .is_some_and(|current| current.same_job(&emitter))
        {
            match ack.poll() {
                std::task::Poll::Pending => {
                    self.navigation_control_checkpoint = Some((emitter, ack));
                    return Ok(());
                }
                std::task::Poll::Ready(Err(reason)) => {
                    return Err(
                        self.stop_failed_navigation_load(RuntimeError::NavigationStopped(reason))
                    );
                }
                std::task::Poll::Ready(Ok(())) => {}
            }
        }
        // An uncommitted transaction already owns its ordered authority check.
        // Once it commits, keep refreshing cancellation during parser readiness.
        if self.pending_navigation_transaction.is_none()
            && let Some(emitter) = self.active_navigation.clone()
        {
            match emitter.checkpoint_pending() {
                Ok(ack) => self.navigation_control_checkpoint = Some((emitter, ack)),
                Err(reason) => {
                    return Err(
                        self.stop_failed_navigation_load(RuntimeError::NavigationStopped(reason))
                    );
                }
            }
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "v8-host", feature = "in-process-probe"))]
mod actor_navigation_strategy_tests {
    use super::*;
    use crate::network::{NetworkError, NetworkEvent, NetworkOperation, NetworkResponse};
    struct NeverHeaders(Mutex<Vec<tokio::sync::mpsc::Sender<NetworkEvent>>>);
    impl NetworkBroker for NeverHeaders {
        fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            unreachable!()
        }
        fn start_operation(
            &self,
            request: NetworkRequest,
        ) -> Result<NetworkOperation, NetworkError> {
            let (sender, events) = tokio::sync::mpsc::channel(1);
            self.0.lock().unwrap().push(sender);
            Ok(NetworkOperation {
                body_demand_pending: Default::default(),
                prefetched: Default::default(),
                request_id: request.request_id,
                events,
                demand: None,
            })
        }
        fn cancel_operation(&self, _: u64) -> Result<(), NetworkError> {
            Ok(())
        }
    }
    #[tokio::test(flavor = "current_thread")]
    async fn none_strategy_acknowledges_admission_without_waiting_for_headers() {
        use crate::navigation_control::{
            NavigationCommand, NavigationPhase, NavigationRequest, PageLoadStrategy,
        };
        let context = Browser::in_process_with_config(BrowserConfig::default())
            .new_context_with_broker(
                Default::default(),
                SharedNetworkBroker::new(NeverHeaders(Mutex::new(Vec::new()))),
            );
        let page = context.new_page();
        page.load_html("https://strategy.test/source", "<title>Source</title>")
            .await
            .unwrap();
        let job = page
            .navigation_control()
            .unwrap()
            .begin(NavigationRequest {
                command: NavigationCommand::Navigate {
                    url: "https://strategy.test/held".into(),
                },
                strategy: PageLoadStrategy::None,
                timeout_ms: None,
            })
            .unwrap();
        tokio::time::timeout(
            Duration::from_secs(2),
            page.run_controlled_navigation(job.clone()),
        )
        .await
        .unwrap()
        .unwrap();
        let ready = job.wait_ready().await.unwrap();
        assert_eq!(ready.phase, NavigationPhase::Started);
        assert_eq!(
            ready.document_generation, None,
            "None readiness does not promise a committed generation"
        );
        assert_eq!(page.evaluate("document.title").await.unwrap(), "Source");
        page.close().await.unwrap();
    }
}

#[cfg(all(test, feature = "v8-host"))]
mod retained_navigation_history_tests {
    use super::*;
    use crate::network::{NetworkError, NetworkEvent, NetworkOperation, NetworkResponse};
    type PendingNetworkRequest = Option<(u64, tokio::sync::mpsc::Sender<NetworkEvent>)>;

    struct Held(Arc<Mutex<PendingNetworkRequest>>);
    impl NetworkBroker for Held {
        fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            unreachable!()
        }
        fn start_operation(
            &self,
            request: NetworkRequest,
        ) -> Result<NetworkOperation, NetworkError> {
            let (sender, events) = tokio::sync::mpsc::channel(4);
            *self.0.lock().unwrap() = Some((request.request_id, sender));
            Ok(NetworkOperation {
                body_demand_pending: Default::default(),
                prefetched: Default::default(),
                request_id: request.request_id,
                events,
                demand: None,
            })
        }
        fn cancel_operation(&self, _: u64) -> Result<(), NetworkError> {
            Ok(())
        }
    }
    #[test]
    fn retained_traversal_rejects_replaced_destination_before_commit_or_form_restore() {
        let held = Arc::new(Mutex::new(None));
        let mut page = PageState::new(
            PageId(330),
            None,
            BrowserConfig::default(),
            ContextOptions::default(),
            Some(SharedNetworkBroker::new(Held(held.clone()))),
            #[cfg(unix)]
            None,
        );
        page.bind_direct_fixture_terminal_owner();
        page.load_document(
            "https://retained.test/old".into(),
            "<input id=value value=old>",
            None,
            None,
            HistoryUpdate::Push,
            &BTreeMap::new(),
            Vec::new(),
        )
        .unwrap();
        page.load_document(
            "https://retained.test/current".into(),
            "<input id=value value=current>",
            None,
            None,
            HistoryUpdate::Push,
            &BTreeMap::new(),
            Vec::new(),
        )
        .unwrap();
        page.back_forward_cache.clear();
        let (reply, mut response) = oneshot::channel();
        // The actor owns the joint receipt separately from the loader transaction.
        let mut pending =
            page.dispatch_navigation_reply(reply, |page| page.begin_history_navigation(-1));
        assert!(pending.as_ref().is_some_and(|reply| reply.joint.is_some()));
        assert!(matches!(
            response.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        let deadline = Instant::now() + Duration::from_secs(3);
        while held.lock().unwrap().is_none() {
            page.poll_page_tasks_inner().unwrap();
            pending = pending.and_then(|reply| page.poll_navigation_reply(reply));
            assert!(
                matches!(
                    response.try_recv(),
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty)
                ),
                "traversal cannot settle before held headers"
            );
            assert!(Instant::now() < deadline);
            std::thread::park_timeout(Duration::from_millis(1));
        }
        page.history[0].navigation.as_mut().unwrap().id = new_navigation_history_token().unwrap();
        let (id, sender) = held.lock().unwrap().take().unwrap();
        sender
            .try_send(NetworkEvent::Headers {
                response_policy: None,
                timing: None,
                request_id: id,
                status: 200,
                headers: vec![("content-type".into(), "text/html".into())],
                content_length: None,
                upload: None,
            })
            .unwrap();
        sender
            .try_send(NetworkEvent::Chunk {
                request_id: id,
                bytes: b"<input id=value value=new>".to_vec().into(),
            })
            .unwrap();
        sender
            .try_send(NetworkEvent::Complete {
                encoded_body_size: None,
                request_id: id,
            })
            .unwrap();
        let result = loop {
            page.poll_page_tasks_inner().unwrap();
            pending = pending.and_then(|reply| page.poll_navigation_reply(reply));
            match response.try_recv() {
                Ok(result) => break result,
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                Err(error) => panic!("lost navigation reply: {error}"),
            };
            assert!(Instant::now() < deadline);
            std::thread::park_timeout(Duration::from_millis(1));
        };
        assert!(matches!(result, Err(RuntimeError::NavigationCancelled)));
        assert_eq!(page.url, "https://retained.test/current");
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("document.getElementById('value').value")
                .unwrap(),
            "current"
        );
    }
    #[test]
    fn reload_restores_controls_captured_after_pagehide_before_new_parser_completion() {
        let held = Arc::new(Mutex::new(None));
        let mut page = PageState::new(
            PageId(331),
            None,
            BrowserConfig::default(),
            ContextOptions::default(),
            Some(SharedNetworkBroker::new(Held(held.clone()))),
            #[cfg(unix)]
            None,
        );
        page.bind_direct_fixture_terminal_owner();
        page.load_document("https://retained.test/reload".into(),"<input id=value value=initial><script>addEventListener('pagehide',()=>document.getElementById('value').value='saved-at-pagehide')</script>",None,None,HistoryUpdate::Push,&BTreeMap::new(),Vec::new()).unwrap();
        let old_key = page.history[0].navigation.as_ref().unwrap().key.clone();
        assert!(matches!(
            page.begin_refresh_navigation().unwrap(),
            NavigationProgress::Pending(_)
        ));
        let deadline = Instant::now() + Duration::from_secs(3);
        while held.lock().unwrap().is_none() {
            page.poll_navigation_transaction();
            assert!(Instant::now() < deadline);
            std::thread::park_timeout(Duration::from_millis(1));
        }
        let (id, sender) = held.lock().unwrap().take().unwrap();
        sender
            .try_send(NetworkEvent::Headers {
                response_policy: None,
                timing: None,
                request_id: id,
                status: 200,
                headers: vec![("content-type".into(), "text/html".into())],
                content_length: None,
                upload: None,
            })
            .unwrap();
        sender
            .try_send(NetworkEvent::Chunk {
                request_id: id,
                bytes: b"<input id=value value=from-response>".to_vec().into(),
            })
            .unwrap();
        sender
            .try_send(NetworkEvent::Complete {
                encoded_body_size: None,
                request_id: id,
            })
            .unwrap();
        loop {
            page.poll_page_tasks_inner().unwrap();
            if page.pending_navigation_transaction.is_none()
                && page
                    .executable
                    .as_ref()
                    .is_some_and(|document| document.document_load_complete())
            {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::park_timeout(Duration::from_millis(1));
        }
        assert_eq!(
            page.history[page.history_index.unwrap()]
                .navigation
                .as_ref()
                .unwrap()
                .key,
            old_key
        );
        assert_eq!(
            page.executable
                .as_mut()
                .unwrap()
                .evaluate("document.getElementById('value').value")
                .unwrap(),
            "saved-at-pagehide"
        );
    }
}

#[cfg(all(test, feature = "v8-host", unix))]
mod download_body_owner_tests {
    use super::*;
    #[test]
    fn download_body_phase_retains_network_owner_until_completion_or_cancellation() {
        struct Owner {
            _retained: std::sync::Arc<()>,
        }
        impl crate::network::NetworkBroker for Owner {
            fn send(
                &self,
                _: crate::network::NetworkRequest,
            ) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
                Err(crate::network::NetworkError::Transport(
                    "unused fixture operation".into(),
                ))
            }
        }
        for complete in [false, true] {
            let owner = std::sync::Arc::new(());
            let retained = std::sync::Arc::downgrade(&owner);
            let broker = SharedNetworkBroker::new(Owner { _retained: owner });
            let (sender, events) = tokio::sync::mpsc::channel(2);
            let operation = crate::network::NetworkOperation {
                request_id: 713,
                events,
                demand: None,
                prefetched: Default::default(),
                body_demand_pending: Default::default(),
            };
            let mut phase = NavigationTransactionPhase::DownloadBody {
                broker,
                pending: navigation_fetch::PendingDownloadBody::new(operation, None),
                url: "https://download.test/body".into(),
                status: 200,
                filename: "exact.bin".into(),
                media_type: "application/octet-stream".into(),
            };
            let NavigationTransactionPhase::DownloadBody { pending, .. } = &mut phase else {
                unreachable!()
            };
            assert!(pending.poll().is_pending());
            assert!(
                retained.upgrade().is_some(),
                "a suspended download retains its native request owner"
            );
            if complete {
                sender
                    .try_send(crate::network::NetworkEvent::Chunk {
                        request_id: 713,
                        bytes: bytes::Bytes::from_static(&[0, 255, 128, 1]),
                    })
                    .unwrap();
                sender
                    .try_send(crate::network::NetworkEvent::Complete {
                        request_id: 713,
                        encoded_body_size: None,
                    })
                    .unwrap();
                assert!(
                    matches!(pending.poll(), std::task::Poll::Ready(Ok(bytes)) if bytes == [0, 255, 128, 1])
                );
            }
            drop(phase);
            assert!(
                sender.is_closed(),
                "completion and cancellation both release the body receiver"
            );
            let deadline = Instant::now() + Duration::from_secs(2);
            while retained.upgrade().is_some() && Instant::now() < deadline {
                std::thread::yield_now();
            }
            assert!(
                retained.upgrade().is_none(),
                "the completed or cancelled phase releases its broker owner"
            );
        }
    }
}

#[cfg(all(test, feature = "v8-host"))]
mod incumbent_request_identity_tests {
    use super::*;
    #[test]
    fn navigation_request_retains_incumbent_url_and_native_opaque_identity() {
        for (url, opaque) in [
            ("https://source.test/document", false),
            ("data:text/html,opaque", true),
        ] {
            let mut page = PageState::new(
                PageId(977),
                None,
                BrowserConfig::default(),
                ContextOptions::default(),
                None,
                #[cfg(unix)]
                None,
            );
            page.load_document(
                url.into(),
                "<title>Source</title>",
                None,
                None,
                HistoryUpdate::Push,
                &BTreeMap::new(),
                Vec::new(),
            )
            .unwrap();
            for history in [
                HistoryUpdate::Push,
                HistoryUpdate::Reload,
                HistoryUpdate::Preserve(0),
            ] {
                let request = page.incumbent_navigation_request_context(history);
                assert_eq!(request.initiator_url.as_deref(), Some(url));
                assert_eq!(request.top_level_url.as_deref(), Some(url));
                assert_eq!(request.fetch_metadata.opaque_initiator, opaque);
                assert_eq!(
                    request.navigation.reload,
                    matches!(history, HistoryUpdate::Reload)
                );
                assert_eq!(
                    request.navigation.history,
                    matches!(history, HistoryUpdate::Preserve(_))
                );
            }
        }
    }
}

#[cfg(all(test, feature = "v8-host"))]
mod settled_navigation_result_tests {
    use super::*;

    #[test]
    fn loaded_executable_waits_for_published_lifecycle_before_reply() {
        let mut page = PageState::new(
            PageId(909),
            None,
            BrowserConfig::default(),
            ContextOptions::default(),
            None,
            #[cfg(unix)]
            None,
        );
        page.bind_direct_fixture_terminal_owner();
        page.load_document(
            "https://settlement.test/current".into(),
            "<title>Loaded title</title>",
            None,
            None,
            HistoryUpdate::Push,
            &BTreeMap::new(),
            Vec::new(),
        )
        .unwrap();
        assert!(page.executable.as_ref().unwrap().document_load_complete());
        let identity = page
            .executable
            .as_ref()
            .unwrap()
            .form_history_root_identity();
        let epoch = page
            .executable
            .as_ref()
            .unwrap()
            .native_navigation_load_epoch();
        page.lifecycle = vec![LifecycleState::Commit];
        let (sender, mut receiver) = oneshot::channel();
        let result = NavigationResult {
            page_id: page.page_id,
            requested_url: page.url.clone(),
            final_url: page.url.clone(),
            title: String::new(),
            status: 200,
            document_generation: page.document_generation,
            revision: Revision(0),
            lifecycle: page.lifecycle.clone(),
            challenge: page.last_challenge.clone(),
        };
        let pending = page
            .deliver_navigation_settlement(NavigationSettlement {
                result: Ok(result),
                reply: Some(NavigationReply::Navigation(sender)),
                source_generation: 0,
                deadline: Instant::now() + Duration::from_secs(2),
                identity,
                load_epoch: epoch,
                ack: None,
            })
            .expect("lifecycle publication pending");
        assert!(matches!(
            receiver.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        let pending = page
            .poll_navigation_reply(pending)
            .expect("still unpublished");
        assert!(matches!(
            receiver.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        page.sync_executable_semantics().unwrap();
        assert!(page.poll_navigation_reply(pending).is_none());
        let result = receiver.try_recv().unwrap().unwrap();
        assert_eq!(
            result.lifecycle,
            vec![
                LifecycleState::Commit,
                LifecycleState::DomContentLoaded,
                LifecycleState::Load
            ]
        );
        assert_eq!(result.title, "Loaded title");
    }

    #[test]
    fn late_settlement_reports_loaded_document_and_rejects_retired_generation() {
        let mut page = PageState::new(
            PageId(908),
            None,
            BrowserConfig::default(),
            ContextOptions::default(),
            None,
            #[cfg(unix)]
            None,
        );
        page.bind_direct_fixture_terminal_owner();
        page.load_document(
            "https://settlement.test/current".into(),
            "<title>Loaded title</title><p>Loaded document</p>",
            None,
            None,
            HistoryUpdate::Push,
            &BTreeMap::new(),
            Vec::new(),
        )
        .unwrap();
        assert!(page.executable.as_ref().unwrap().document_load_complete());
        let generation = page.document_generation;
        let identity = page
            .executable
            .as_ref()
            .unwrap()
            .form_history_root_identity();
        let epoch = page
            .executable
            .as_ref()
            .unwrap()
            .native_navigation_load_epoch();
        for retired in [false, true] {
            let (sender, mut receiver) = oneshot::channel();
            let result = NavigationResult {
                page_id: page.page_id,
                requested_url: "https://settlement.test/request".into(),
                final_url: page.url.clone(),
                title: String::new(),
                status: 207,
                document_generation: generation - u64::from(retired),
                revision: Revision(0),
                lifecycle: vec![LifecycleState::Commit],
                challenge: page.last_challenge.clone(),
            };
            assert!(
                page.deliver_navigation_settlement(NavigationSettlement {
                    result: Ok(result),
                    reply: Some(NavigationReply::Navigation(sender)),
                    source_generation: 0,
                    deadline: Instant::now() + Duration::from_secs(2),
                    identity,
                    load_epoch: epoch,
                    ack: None
                })
                .is_none()
            );
            let result = receiver.try_recv().unwrap();
            if retired {
                assert!(matches!(result, Err(RuntimeError::NavigationCancelled)));
            } else {
                let result = result.unwrap();
                assert_eq!(result.title, "Loaded title");
                assert_eq!(result.lifecycle, page.lifecycle);
                assert_eq!(result.lifecycle.last(), Some(&LifecycleState::Load));
                assert_eq!(result.revision, Revision(page.revision));
                assert_eq!(result.requested_url, "https://settlement.test/request");
                assert_eq!(result.status, 207);
                assert_eq!(result.document_generation, generation);
            }
        }
    }
}

// Diagnostic-only state for the currently committed native document. The owner
// is captured before queued continuation; no later active request is consulted.
// Native identity/load epoch checks suppress stale attribution after document
// replacement, document.open, cancellation or history activation.
#[cfg(all(unix, feature = "v8-host"))]
struct CommittedNavigationProgress {
    owner: crate::ipc_progress::Identity,
    generation: u64,
    native_identity: (oryn_js::realm::RealmId, u64, u64),
    load_epoch: u64,
    reported: u8,
}
#[cfg(all(unix, feature = "v8-host"))]
#[derive(Clone, Copy)]
enum NavigationReadinessStage {
    LayoutCompleted,
    ParserSettled,
    LoadReady,
}
#[cfg(all(unix, feature = "v8-host"))]
impl CommittedNavigationProgress {
    fn select(
        &mut self,
        stage: NavigationReadinessStage,
        generation: u64,
        native_identity: Option<(oryn_js::realm::RealmId, u64, u64)>,
        load_epoch: Option<u64>,
    ) -> Option<(crate::ipc_progress::Identity, crate::ipc_progress::Phase)> {
        if self.generation != generation
            || native_identity != Some(self.native_identity)
            || load_epoch != Some(self.load_epoch)
        {
            return None;
        }
        use crate::ipc_progress::Phase;
        let (bit, phase) = match stage {
            NavigationReadinessStage::LayoutCompleted => (1, Phase::NavigationLayoutCompleted),
            NavigationReadinessStage::ParserSettled => (2, Phase::NavigationParserSettled),
            NavigationReadinessStage::LoadReady => (4, Phase::NavigationLoadReady),
        };
        if self.reported & bit != 0 {
            return None;
        }
        self.reported |= bit;
        Some((self.owner, phase))
    }
}
#[cfg(all(unix, feature = "v8-host"))]
impl PageState {
    fn capture_committed_navigation_progress(&mut self) {
        self.committed_navigation_progress = (|| {
            let owner = self.navigation_progress_identity?;
            let executable = self.executable.as_ref()?;
            Some(CommittedNavigationProgress {
                owner,
                generation: self.document_generation,
                native_identity: executable.form_history_root_identity()?,
                load_epoch: executable.native_navigation_load_epoch()?,
                reported: 0,
            })
        })();
    }
    fn record_committed_navigation_progress(&mut self, stage: NavigationReadinessStage) {
        // A temporary selected-frame view is not the committed root even when
        // its numeric document generation happens to match.
        if self.frame_scope.is_some() {
            return;
        }
        let Some(progress) = self.committed_navigation_progress.as_mut() else {
            return;
        };
        // These getters read existing native slots; no parent RPC, authority
        // lookup, task pump, retry, or new deadline is introduced by logging.
        let native_identity = self
            .executable
            .as_ref()
            .and_then(|page| page.form_history_root_identity());
        let load_epoch = self
            .executable
            .as_ref()
            .and_then(|page| page.native_navigation_load_epoch());
        if let Some((owner, phase)) =
            progress.select(stage, self.document_generation, native_identity, load_epoch)
        {
            // Independent of context_rpc's sampling budget. The general sink
            // retains its own explicit truncation and regular-file-only rules.
            crate::ipc_progress::record(Some(owner), phase, None, Some(self.document_generation));
        }
    }
}

#[cfg(all(test, unix, feature = "v8-host"))]
mod navigation_readiness_trace_tests {
    use super::*;

    fn progress(page: u64) -> CommittedNavigationProgress {
        let route = crate::worker_route::WorkerRoute::issue(801, page, 7).unwrap();
        let request = crate::ipc_progress::request(
            &route,
            4,
            &crate::worker::WorkerRequest::Observe { profile: None },
        );
        CommittedNavigationProgress {
            owner: request.identity().unwrap(),
            generation: 5,
            native_identity: (oryn_js::realm::RealmId(31), 5, 8),
            load_epoch: 9,
            reported: 0,
        }
    }

    #[test]
    fn readiness_trace_rejects_stale_document_stream_and_cancel_epoch() {
        let mut trace = progress(890_301);
        let original = trace.native_identity;
        for (generation, native, epoch) in [
            (6, Some(original), Some(9)),
            (5, None, Some(9)),
            (5, Some((oryn_js::realm::RealmId(32), 5, 8)), Some(9)),
            (5, Some((oryn_js::realm::RealmId(31), 6, 8)), Some(9)),
            (5, Some((oryn_js::realm::RealmId(31), 5, 9)), Some(9)),
            (5, Some(original), None),
            (5, Some(original), Some(10)),
        ] {
            assert!(
                trace
                    .select(
                        NavigationReadinessStage::LoadReady,
                        generation,
                        native,
                        epoch
                    )
                    .is_none()
            );
            assert_eq!(
                trace.reported, 0,
                "a stale check cannot consume a current stage"
            );
        }
        assert!(
            trace
                .select(
                    NavigationReadinessStage::LoadReady,
                    5,
                    Some(original),
                    Some(9)
                )
                .is_some()
        );
    }

    #[test]
    fn readiness_trace_retains_owner_and_emits_each_actual_stage_once() {
        let mut trace = progress(890_302);
        let original = trace.owner;
        let native = trace.native_identity;
        let route = crate::worker_route::WorkerRoute::issue(801, 890_302, 7).unwrap();
        let later = crate::ipc_progress::request(
            &route,
            5,
            &crate::worker::WorkerRequest::Observe { profile: None },
        );
        assert!(later.identity() != Some(original));
        let mut phases = Vec::new();
        for stage in [
            NavigationReadinessStage::LayoutCompleted,
            NavigationReadinessStage::ParserSettled,
            NavigationReadinessStage::LoadReady,
        ] {
            let (owner, phase) = trace.select(stage, 5, Some(native), Some(9)).unwrap();
            assert!(
                owner == original,
                "continuation cannot adopt the later command"
            );
            phases.push(serde_json::to_value(phase).unwrap());
            for _ in 0..32 {
                assert!(trace.select(stage, 5, Some(native), Some(9)).is_none());
            }
        }
        assert_eq!(
            serde_json::Value::Array(phases),
            serde_json::json!([
                "navigation_layout_completed",
                "navigation_parser_settled",
                "navigation_load_ready"
            ])
        );
        assert_eq!(trace.reported, 7);
    }
}

#[cfg(all(test, feature = "v8-host"))]
#[path = "navigation_ready_progress_tests.rs"]
mod navigation_ready_progress_tests;
