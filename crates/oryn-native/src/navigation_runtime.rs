/// Inputs for load buffered navigation with timing.
struct BufferedNavigationInputs {
    url: Url,
    requested_url: String,
    history: HistoryUpdate,
    content_type: Option<String>,
    bytes: Vec<u8>,
    response_headers: Vec<(String, String)>,
    timing: oryn_js::navigation_timing::NavigationTiming,
}
// Included in runtime.rs: native controlled navigation shares parent lifecycle
// state, while loader execution stays in the page's existing actor.

#[cfg(all(test, feature = "in-process-probe", feature = "v8-host"))]
mod data_navigation_generation_tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn controlled_data_navigation_respects_requested_readiness_strategy() {
        use crate::navigation_control::*;
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        for strategy in [
            PageLoadStrategy::None,
            PageLoadStrategy::Eager,
            PageLoadStrategy::Normal,
        ] {
            let control = page.navigation_control().unwrap();
            let job = control
                .begin(NavigationRequest {
                    command: NavigationCommand::Navigate {
                        url: "data:text/html,%3Ctitle%3EReady%3C/title%3E".into(),
                    },
                    strategy,
                    timeout_ms: None,
                })
                .unwrap();
            page.run_controlled_navigation(job.clone()).await.unwrap();
            let progress = job.wait_ready().await.unwrap();
            match strategy {
                PageLoadStrategy::None => assert!(matches!(
                    progress.phase,
                    NavigationPhase::Started
                        | NavigationPhase::Committed
                        | NavigationPhase::Interactive
                        | NavigationPhase::Complete
                )),
                PageLoadStrategy::Eager => assert!(matches!(
                    progress.phase,
                    NavigationPhase::Interactive | NavigationPhase::Complete
                )),
                PageLoadStrategy::Normal => assert_eq!(progress.phase, NavigationPhase::Complete),
            }
            let progress = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let current = job.progress().unwrap();
                    assert!(
                        current.stopped.is_none(),
                        "data navigation stopped before commit: {current:?}"
                    );
                    if current.document_generation.is_some() {
                        break current;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("admitted data navigation must eventually commit");
            assert!(progress.document_generation.is_some());
            assert!(progress.stopped.is_none());
        }
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn blank_navigation_inherits_native_origin_and_document_base() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        page.load_html(
            "https://blank.test/source",
            "<base href='https://assets.test/directory/'><title>Source</title>",
        )
        .await
        .unwrap();
        page.goto("about:blank").await.unwrap();
        eprintln!(
            "repair44 native29/50 {}",
            repair44_bounded_diagnostic(format_args!(
                "parent_origin={:?} parent_url={:?}",
                context.page_origin(page.page_id()),
                context.page_url(page.page_id())
            ))
        );
        assert_eq!(
            page.evaluate("self.origin").await.unwrap(),
            "https://blank.test"
        );
        assert_eq!(
            page.evaluate("document.baseURI").await.unwrap(),
            "https://assets.test/directory/"
        );
        assert_eq!(
            context.page_origin(page.page_id()).unwrap(),
            "https://blank.test"
        );
        assert_eq!(
            page.evaluate("navigation.entries().map(e=>e.url).join(',')")
                .await
                .unwrap(),
            "https://blank.test/source,about:blank"
        );
        page.goto("about:blank#next").await.unwrap();
        assert_eq!(
            page.evaluate("document.baseURI").await.unwrap(),
            "https://assets.test/directory/"
        );
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn navigation_receipts_use_parent_root_identity_after_reserved_generation_gaps() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let id = page.page_id();
        // Parent-issued identities need not equal the page actor's local
        // preflight cursor. Consume genuine unused reservations first.
        let first = context
            .native_document_authority(id, "about:blank")
            .unwrap()
            .3;
        let second = context
            .native_document_authority(id, "about:blank")
            .unwrap()
            .3;
        assert!(second > first);
        page.load_html("https://generation.test/source", "<title>Tuple</title>")
            .await
            .unwrap();
        let tuple = context
            .pages()
            .into_iter()
            .find(|entry| entry.page.page_id == id)
            .unwrap()
            .page;
        let active = || {
            context
                .inner
                .group
                .messages
                .lock()
                .unwrap()
                .root_document_identity(id.0)
                .unwrap()
                .0
        };
        assert_eq!(tuple.document_generation, active());
        assert!(tuple.document_generation > second);
        assert!(
            context
                .window_worker_creator(id, &oryn_js::message_wire::WindowTarget::Root, first, 0)
                .is_err(),
            "an unused old reservation must not become the installed root"
        );
        assert_eq!(context.page_origin(id).unwrap(), "https://generation.test");
        let blank = page.goto("about:blank").await.unwrap();
        assert_eq!(blank.document_generation, active());
        assert!(blank.document_generation > tuple.document_generation);
        assert_eq!(context.page_origin(id).unwrap(), "https://generation.test");
        assert_eq!(
            page.evaluate("self.origin").await.unwrap(),
            "https://generation.test"
        );
        let opaque = page
            .goto("data:text/html,%3Ctitle%3EOpaque%3C/title%3E")
            .await
            .unwrap();
        assert_eq!(opaque.document_generation, active());
        assert!(opaque.document_generation > blank.document_generation);
        let key = context.inner.security_origins.lock().unwrap()[&id].security_token_key();
        page.record_navigation(&opaque);
        assert_eq!(
            context.inner.security_origins.lock().unwrap()[&id].security_token_key(),
            key,
            "receipt bookkeeping must retain the exact opaque principal"
        );
        assert_eq!(context.page_origin(id).unwrap(), "null");
        assert!(
            context
                .native_document_authority(id, "https://generation.test/foreign")
                .is_err(),
            "generation adoption must not weaken exact policy URL admission"
        );
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn public_resource_navigation_obeys_native_cancellation_before_fetch() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        page.load_html("https://admission.test/source","<title>Retained</title><script>globalThis.admissions=0;navigation.addEventListener('navigate',e=>{admissions++;e.preventDefault()})</script>").await.unwrap();
        let result = page
            .goto_resource(
                "https://admission.test/blocked".into(),
                None,
                Some("no-referrer".into()),
                None,
            )
            .await
            .unwrap();
        assert_eq!(result.final_url, "https://admission.test/source");
        assert_eq!(result.title, "Retained");
        assert_eq!(page.evaluate("String(admissions)").await.unwrap(), "1");
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn navigation_storage_uses_native_partitions_without_author_snapshots() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        page.load_html("https://storage-a.test/", "<title>A</title>")
            .await
            .unwrap();
        page.evaluate("localStorage.setItem('owned','local-a');sessionStorage.setItem('owned','session-a');Object.defineProperty(Storage.prototype,'items',{get(){throw Error('author snapshot trap')}});'ready'").await.unwrap();
        page.load_html("https://storage-b.test/", "<title>B</title>")
            .await
            .unwrap();
        assert_eq!(
            page.evaluate(
                "String(localStorage.getItem('owned'))+':'+String(sessionStorage.getItem('owned'))"
            )
            .await
            .unwrap(),
            "null:null"
        );
        page.evaluate("localStorage.setItem('owned','local-b');sessionStorage.setItem('owned','session-b');'ready'").await.unwrap();
        page.load_html("https://storage-a.test/fresh", "<title>A fresh</title>")
            .await
            .unwrap();
        assert_eq!(
            page.evaluate("localStorage.getItem('owned')+':'+sessionStorage.getItem('owned')")
                .await
                .unwrap(),
            "local-a:session-a"
        );
        page.back().await.unwrap();
        assert_eq!(
            page.evaluate("localStorage.getItem('owned')+':'+sessionStorage.getItem('owned')")
                .await
                .unwrap(),
            "local-b:session-b"
        );
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn load_html_preserves_outgoing_authority_until_pagehide_and_cached_restore() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        page.load_html("https://incumbent.test/","<title>Incumbent</title><script>globalThis.kept=document;onpagehide=()=>{try{localStorage.setItem('departure','stored');globalThis.departure='ok'}catch(e){globalThis.departure=e.name}}</script>").await.unwrap();
        page.load_html("https://destination.test/", "<title>Destination</title>")
            .await
            .unwrap();
        assert_eq!(
            context.page_origin(page.page_id()).unwrap(),
            "https://destination.test"
        );
        page.back().await.unwrap();
        assert_eq!(page.evaluate("departure").await.unwrap(), "ok");
        assert_eq!(
            page.evaluate("localStorage.getItem('departure')")
                .await
                .unwrap(),
            "stored"
        );
        assert_eq!(page.evaluate("kept===document").await.unwrap(), "true");
        assert_eq!(
            context.page_origin(page.page_id()).unwrap(),
            "https://incumbent.test"
        );
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn data_navigation_uses_native_html_parser_and_history() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let first = "data:text/html;charset=utf-8,%3Ctitle%3EFirst%3C/title%3E%3Cscript%3Edocument.title+=' native'%3C/script%3E";
        let result = page.goto(first).await.unwrap();
        assert_eq!(result.title, "First native");
        assert_eq!(context.page_origin(page.page_id()).unwrap(), "null");
        let second = page
            .goto("data:text/html,%3Ctitle%3ESecond%3C/title%3E")
            .await
            .unwrap();
        assert!(second.document_generation > result.document_generation);
        let restored = page.back().await;
        eprintln!(
            "repair44 native29/53 {}",
            repair44_bounded_diagnostic(format_args!(
                "back={restored:?} parent_origin={:?}",
                context.page_origin(page.page_id())
            ))
        );
        assert_eq!(restored.unwrap().title, "First native");
        let restored = page.forward().await;
        eprintln!(
            "repair44 native29/53 {}",
            repair44_bounded_diagnostic(format_args!(
                "forward={restored:?} parent_origin={:?}",
                context.page_origin(page.page_id())
            ))
        );
        assert_eq!(restored.unwrap().title, "Second");
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn data_navigation_preserves_plain_text_and_xml_mime() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        page.goto("data:text/plain,%3Ctitle%3ENot%20markup%3C/title%3E")
            .await
            .unwrap();
        assert_eq!(
            page.evaluate("document.contentType").await.unwrap(),
            "text/plain"
        );
        assert_eq!(
            page.evaluate("document.body.textContent").await.unwrap(),
            "<title>Not markup</title>"
        );
        page.goto("data:application/xml,%3Croot%3Evalue%3C/root%3E")
            .await
            .unwrap();
        assert_eq!(
            page.evaluate("document.contentType").await.unwrap(),
            "application/xml"
        );
        assert_eq!(
            page.evaluate("document.documentElement.localName")
                .await
                .unwrap(),
            "root"
        );
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn buffered_data_navigation_decodes_bom_and_preserves_text_document_identity() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let bytes = [0xff, 0xfe, b'<', 0, b'p', 0, b'>', 0, 0xe9, 0];
        let url = format!("data:text/plain;base64,{}", BASE64.encode(bytes));
        page.goto(url.clone()).await.unwrap();
        assert_eq!(
            page.evaluate("document.body.textContent").await.unwrap(),
            "<p>é"
        );
        assert_eq!(
            page.evaluate("document.characterSet").await.unwrap(),
            "UTF-16LE"
        );
        assert_eq!(
            page.evaluate("document.contentType").await.unwrap(),
            "text/plain"
        );
        assert_eq!(page.evaluate("location.href").await.unwrap(), url);
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn blob_document_navigation_uses_native_creator_origin_and_mime() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        page.load_html("https://blob.test/source", "<title>Source</title>")
            .await
            .unwrap();
        let blob=page.evaluate("URL.createObjectURL(new Blob(['<title>Blob destination</title><p>body</p>'],{type:'text/html'}))").await.unwrap();
        let result = page.goto(blob.clone()).await.unwrap();
        assert_eq!(result.title, "Blob destination");
        assert_eq!(result.final_url, blob);
        assert_eq!(
            page.evaluate("self.origin").await.unwrap(),
            "https://blob.test"
        );
        assert_eq!(
            page.evaluate("document.contentType").await.unwrap(),
            "text/html"
        );
        assert_eq!(
            context.page_origin(page.page_id()).unwrap(),
            "https://blob.test"
        );
        page.close().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn blob_commit_requires_current_native_policy_and_survives_cached_restore() {
        let context = Browser::in_process_probe().new_context();
        let page = context.new_page();
        let other = context.new_page();
        page.load_html("https://blob.test/source", "<title>Source</title>")
            .await
            .unwrap();
        let blob = page.evaluate("URL.createObjectURL(new Blob(['<title>Retained blob</title>'],{type:'text/html'}))").await.unwrap();
        assert!(
            context
                .validate_page_commit_url(page.page_id(), &blob)
                .is_err(),
            "a created URL is not an admitted navigation"
        );
        page.goto(blob.clone()).await.unwrap();
        assert!(
            context
                .validate_page_commit_url(page.page_id(), &blob)
                .is_ok()
        );
        assert!(
            context
                .validate_page_commit_url(other.page_id(), &blob)
                .is_err()
        );
        assert!(
            context
                .validate_page_commit_url(page.page_id(), "blob:https://blob.test/forged")
                .is_err()
        );
        assert_eq!(
            context.page_origin(page.page_id()).unwrap(),
            "https://blob.test"
        );
        page.load_html("https://blob.test/after", "<title>After</title>")
            .await
            .unwrap();
        assert!(
            context
                .validate_page_commit_url(page.page_id(), &blob)
                .is_err(),
            "a cached document alone cannot authorize the current URL"
        );
        assert_eq!(page.back().await.unwrap().title, "Retained blob");
        assert!(
            context
                .validate_page_commit_url(page.page_id(), &blob)
                .is_ok()
        );
        assert_eq!(
            context.page_origin(page.page_id()).unwrap(),
            "https://blob.test"
        );
        assert_eq!(page.forward().await.unwrap().title, "After");
        assert!(
            context
                .validate_page_commit_url(page.page_id(), &blob)
                .is_err()
        );
        other.close().await.unwrap();
        page.close().await.unwrap();
    }
}

impl PageHandle {
    pub async fn run_controlled_navigation(
        &self,
        job: Arc<crate::navigation_control::NavigationJob>,
    ) -> Result<(), RuntimeError> {
        use crate::navigation_control::{NavigationCommand, NavigationEmitter};
        let control = self.navigation_control()?;
        let admitted = control
            .active(job.id)
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        if !Arc::ptr_eq(&admitted, &job) {
            return Err(RuntimeError::ProtocolViolation(
                "navigation job belongs to another page".into(),
            ));
        }
        let context = self.owning_context().ok_or(RuntimeError::Closed)?;
        struct DispatchCustody(Option<Arc<crate::navigation_control::NavigationJob>>);
        impl Drop for DispatchCustody {
            fn drop(&mut self) {
                if let Some(job) = self.0.take() {
                    let _ = job.stop(crate::navigation_control::NavigationStop::Cancelled);
                }
            }
        }
        let mut dispatch = DispatchCustody(Some(job.clone()));
        if let NavigationCommand::Navigate { url } = &job.request.command {
            let source_origin = context
                .page_origin(self.page_id())
                .unwrap_or_else(|_| "null".into());
            let target_origin = Url::parse(url)
                .map(|url| url.origin().ascii_serialization())
                .unwrap_or_else(|_| "null".into());
            if source_origin != target_origin && source_origin != "null" {
                context.require_policy(
                    PolicyRequest {
                        action: ConsequentialAction::CrossOriginNavigation,
                        source_origin,
                        target_origin: Some(target_origin),
                        detail: Some(url.clone()),
                    },
                    None,
                )?;
                #[cfg(unix)]
                {
                    job.install_navigation_grant(|| context.grant_navigation_chain(self.page_id()))
                        .map_err(|error| RuntimeError::Network(error.to_string()))?;
                }
            }
        }
        let result = async {
            #[cfg(unix)]
            if let Some(response) = self
                .worker_request(crate::worker::WorkerRequest::ControlledNavigate {
                    id: job.id,
                    request: job.request.clone(),
                })
                .await
            {
                return match response? {
                    crate::worker::WorkerResponse::Satisfied => Ok(()),
                    response => Err(unexpected_worker_response(response)),
                };
            }
            self.dispatch_controlled_navigation(
                job.request.clone(),
                NavigationEmitter::Local(job.clone()),
            )
            .await
        }
        .await;
        if let Err(error) = &result {
            let _ = job.stop(error.navigation_stop());
        }
        dispatch.0.take();
        result
    }
    #[cfg(unix)]
    pub(crate) async fn run_remote_navigation(
        &self,
        id: crate::navigation_control::NavigationId,
        request: crate::navigation_control::NavigationRequest,
    ) -> Result<(), RuntimeError> {
        #[cfg(feature = "v8-host")]
        {
            let context = self.owning_context().ok_or(RuntimeError::Closed)?;
            let client = context.inner.context_client.clone().ok_or_else(|| {
                RuntimeError::ProtocolViolation(
                    "controlled worker navigation has no parent channel".into(),
                )
            })?;
            self.dispatch_controlled_navigation(
                request,
                crate::navigation_control::NavigationEmitter::remote(client, self.page_id(), id)?,
            )
            .await
        }
        #[cfg(not(feature = "v8-host"))]
        {
            let _ = (id, request);
            Err(RuntimeError::ProtocolViolation(
                "worker navigation requires native host".into(),
            ))
        }
    }
    async fn dispatch_controlled_navigation(
        &self,
        request: crate::navigation_control::NavigationRequest,
        emitter: crate::navigation_control::NavigationEmitter,
    ) -> Result<(), RuntimeError> {
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::ControlledNavigate {
                request,
                emitter,
                reply,
            })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }
}
impl PageState {
    fn stop_navigation_load(&mut self) -> Result<(), RuntimeError> {
        self.stop_navigation_load_with_error(RuntimeError::NavigationCancelled)
    }

    fn stop_failed_navigation_load(&mut self, cause: RuntimeError) -> RuntimeError {
        let cleanup = self.stop_navigation_load_with_error(cause.clone());
        self.finish_navigation_load_cleanup(cause, cleanup)
    }

    fn finish_navigation_load_cleanup(
        &mut self,
        cause: RuntimeError,
        cleanup: Result<(), RuntimeError>,
    ) -> RuntimeError {
        match cleanup {
            Ok(()) => cause,
            Err(cleanup) => {
                let error = RuntimeError::NavigationCleanupFailed {
                    cause: Box::new(cause),
                    cleanup: Box::new(cleanup),
                };
                // The controller retains a typed stop (e.g. Timeout). Record
                // secondary detail once, with bounded payload, in the page trace.
                let mut detail = error.to_string();
                if detail.len() > 4096 {
                    let mut end = 4096;
                    while !detail.is_char_boundary(end) {
                        end -= 1;
                    }
                    detail.truncate(end);
                    detail.push_str(" [truncated]");
                }
                self.trace.push(
                    Revision(self.revision),
                    None,
                    TraceEventKind::ScriptDiagnostic {
                        capability: "navigation.cleanup".into(),
                        detail,
                    },
                );
                error
            }
        }
    }

    fn stop_navigation_load_with_error(
        &mut self,
        _cause: RuntimeError,
    ) -> Result<(), RuntimeError> {
        // Clear admission even when native cleanup reports an error. A failed
        // operation must not publish late milestones or stop a later navigation.
        #[cfg(feature = "v8-host")]
        self.cancel_navigation_transaction(_cause);
        #[cfg(feature = "v8-host")]
        for pending in &self.pending_execution_replies {
            if let Some(receipt) = &pending.joint {
                receipt.cancel();
            }
        }
        self.active_navigation = None;
        self.navigation_committed = false;
        #[cfg(feature = "v8-host")]
        {
            self.pending_navigation_response.take();
        }
        #[cfg(feature = "v8-host")]
        if let Some(executable) = self.executable.as_mut() {
            executable.cancel_joint_history_traversals();
            executable
                .stop_pending_document_load()
                .map_err(map_host_error)?;
        }
        Ok(())
    }

    #[cfg(all(test, feature = "v8-host"))]
    fn navigate_data_document(
        &mut self,
        url: Url,
        requested_url: String,
        history: HistoryUpdate,
    ) -> Result<NavigationResult, RuntimeError> {
        let timing = oryn_js::navigation_timing::NavigationTiming::new(
            Instant::now(),
            match history {
                HistoryUpdate::Reload => oryn_js::navigation_timing::Kind::Reload,
                HistoryUpdate::Preserve(_) => oryn_js::navigation_timing::Kind::BackForward,
                _ => oryn_js::navigation_timing::Kind::Navigate,
            },
        );
        self.navigate_data_document_with_timing(url, requested_url, history, timing)
    }
    #[cfg(any(test, not(feature = "v8-host")))]
    fn navigate_data_document_with_timing(
        &mut self,
        url: Url,
        requested_url: String,
        history: HistoryUpdate,
        timing: oryn_js::navigation_timing::NavigationTiming,
    ) -> Result<NavigationResult, RuntimeError> {
        #[cfg(all(unix, feature = "v8-host"))]
        if self.context_client.is_some() {
            return self.navigate_prepared_local_document(
                url,
                requested_url,
                history,
                false,
                timing,
            );
        }
        if let Some(job) = &self.active_navigation {
            job.checkpoint()
                .map_err(|error| RuntimeError::Network(error.to_string()))?;
        }
        let (content_type, bytes) = crate::network::decode_data_url(&url)
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        // A binary local response does not replace the incumbent Document or
        // its policy, even when the standalone runtime has no quarantine host.
        if crate::download::computed_type_is_download(&oryn_net::mime_sniff::document_type(
            content_type.as_deref(),
            false,
            &bytes,
        )) {
            return self.load_buffered_navigation_with_timing(BufferedNavigationInputs {
                url,
                requested_url,
                history,
                content_type,
                bytes,
                response_headers: Vec::new(),
                timing,
            });
        }
        self.csp = crate::csp::CspPolicySet::from_headers(url.to_string(), &[]);
        self.response_policy_headers.clear();
        self.script_isolation = crate::agent_isolation::DocumentIsolationPolicy::from_headers(&[]);
        self.referrer_policy = crate::referrer::ReferrerPolicy::default();
        self.set_challenge_assessment(ChallengeAssessment::default());
        self.load_buffered_navigation_with_timing(BufferedNavigationInputs {
            url,
            requested_url,
            history,
            content_type,
            bytes,
            response_headers: Vec::new(),
            timing,
        })
    }

    #[cfg(all(unix, feature = "v8-host"))]
    #[cfg(test)]
    fn navigate_prepared_local_document(
        &mut self,
        url: Url,
        requested_url: String,
        history: HistoryUpdate,
        blob: bool,
        timing: oryn_js::navigation_timing::NavigationTiming,
    ) -> Result<NavigationResult, RuntimeError> {
        let client = self
            .context_client
            .clone()
            .ok_or_else(|| RuntimeError::Network("local navigation service unavailable".into()))?;
        let blank = url.scheme() == "about" && url.path() == "blank";
        let request = if let Some(admission) = self.service_navigation_authority.clone() {
            if blank {
                return Err(RuntimeError::ProtocolViolation(
                    "service navigation cannot target about:blank".into(),
                ));
            }
            crate::context_service::ContextServiceRequest::PrepareServiceLocalNavigation {
                page_id: self.page_id,
                admission,
                url: url.to_string(),
                blob,
            }
        } else if blank {
            let (creator, base_url) = if let Some(executable) = self.executable.as_mut() {
                (
                    Some(
                        executable
                            .native_navigation_creator()
                            .map_err(map_host_error)?,
                    ),
                    Some(
                        executable
                            .native_navigation_base_url()
                            .map_err(map_host_error)?,
                    ),
                )
            } else {
                (None, None)
            };
            crate::context_service::ContextServiceRequest::PrepareBlankNavigation {
                page_id: self.page_id,
                creator,
                url: url.to_string(),
                base_url,
            }
        } else if let Some(executable) = self.executable.as_mut() {
            let creator = executable
                .native_navigation_creator()
                .map_err(map_host_error)?;
            if blob {
                crate::context_service::ContextServiceRequest::PrepareBlobNavigation {
                    page_id: self.page_id,
                    creator,
                    url: url.to_string(),
                }
            } else {
                crate::context_service::ContextServiceRequest::PrepareLocalDataNavigation {
                    page_id: self.page_id,
                    creator,
                    url: url.to_string(),
                }
            }
        } else {
            crate::context_service::ContextServiceRequest::PrepareInitialLocalNavigation {
                page_id: self.page_id,
                url: url.to_string(),
                blob,
            }
        };
        let (token, encoded, media_type, headers, base_url) = match client.request(request)? {
            crate::context_service::ContextServiceResponse::BlobNavigationPrepared {
                token,
                body_base64,
                media_type,
                headers,
                ..
            } if blob && !blank => (token, body_base64, media_type, headers, None),
            crate::context_service::ContextServiceResponse::LocalDataNavigationPrepared {
                token,
                body_base64,
                media_type,
                headers,
                ..
            } if !blob && !blank => (token, body_base64, media_type, headers, None),
            crate::context_service::ContextServiceResponse::BlankNavigationPrepared {
                token,
                body_base64,
                media_type,
                headers,
                base_url,
                ..
            } if blank => (token, body_base64, media_type, headers, Some(base_url)),
            _ => {
                return Err(RuntimeError::ProtocolViolation(
                    "invalid local navigation preparation response".into(),
                ));
            }
        };
        let result = (|| {
            if encoded.len() > 12 * 1024 * 1024 {
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "local navigation encoded bytes".into(),
                });
            }
            let bytes = BASE64.decode(encoded).map_err(|error| {
                RuntimeError::ProtocolViolation(format!("invalid local navigation bytes: {error}"))
            })?;
            if bytes.len() > 8 * 1024 * 1024 {
                return Err(RuntimeError::WorkerLimitExceeded {
                    resource: "local navigation bytes".into(),
                });
            }
            let previous = self.prepared_local_navigation.replace((
                token.clone(),
                url.to_string(),
                headers.clone(),
                base_url,
            ));
            let result = self.load_buffered_navigation_with_timing(BufferedNavigationInputs {
                url,
                requested_url,
                history,
                content_type: Some(media_type),
                bytes,
                response_headers: headers,
                timing,
            });
            self.prepared_local_navigation = previous;
            result
        })();
        // Discard is idempotent after consumption and also releases preparation
        // when a callback cancels before the native commit boundary.
        let cleanup = client.request(
            crate::context_service::ContextServiceRequest::DiscardPreparedLocalNavigation {
                page_id: self.page_id,
                token,
            },
        );
        match (result, cleanup) {
            (
                Ok(result),
                Ok(crate::context_service::ContextServiceResponse::LocalNavigationDiscarded),
            ) => Ok(result),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Ok(_), Ok(_)) => Err(RuntimeError::ProtocolViolation(
                "invalid local navigation discard acknowledgement".into(),
            )),
        }
    }

    /// One MIME/encoding/parser path for already captured navigation bodies.
    /// The caller must prepare the native destination policy before entering.
    #[cfg(all(test, feature = "v8-host"))]
    fn load_buffered_navigation(
        &mut self,
        url: Url,
        requested_url: String,
        history: HistoryUpdate,
        content_type: Option<String>,
        bytes: Vec<u8>,
        response_headers: Vec<(String, String)>,
    ) -> Result<NavigationResult, RuntimeError> {
        let timing = oryn_js::navigation_timing::NavigationTiming::new(
            Instant::now(),
            match history {
                HistoryUpdate::Reload => oryn_js::navigation_timing::Kind::Reload,
                HistoryUpdate::Preserve(_) => oryn_js::navigation_timing::Kind::BackForward,
                _ => oryn_js::navigation_timing::Kind::Navigate,
            },
        );
        self.load_buffered_navigation_with_timing(BufferedNavigationInputs {
            url,
            requested_url,
            history,
            content_type,
            bytes,
            response_headers,
            timing,
        })
    }
    fn load_buffered_navigation_with_timing(
        &mut self,
        params: BufferedNavigationInputs,
    ) -> Result<NavigationResult, RuntimeError> {
        let BufferedNavigationInputs {
            url,
            requested_url,
            history,
            content_type,
            bytes,
            response_headers,
            timing,
        } = params;

        let content_type = Some(oryn_net::mime_sniff::document_type(
            content_type.as_deref(),
            crate::network::response_no_sniff(&response_headers),
            &bytes,
        ));
        if content_type
            .as_deref()
            .is_some_and(crate::download::computed_type_is_download)
        {
            #[cfg(all(unix, feature = "v8-host"))]
            {
                let creator = self
                    .executable
                    .as_mut()
                    .ok_or_else(|| {
                        RuntimeError::Download("download source document unavailable".into())
                    })?
                    .native_navigation_creator()
                    .map_err(map_host_error)?;
                let filename =
                    crate::download::response_download_filename(&response_headers, &url, Some(""))
                        .unwrap_or_else(|| "download".into());
                self.quarantine_navigation_bytes(
                    creator,
                    url.to_string(),
                    filename,
                    content_type.clone().unwrap(),
                    &bytes,
                )?;
                return self.finish_navigation_without_commit(requested_url, 200);
            }
            #[cfg(not(all(unix, feature = "v8-host")))]
            return Err(RuntimeError::Download(
                "native download service unavailable".into(),
            ));
        }
        // Buffered parent parsing has no script-visible navigation timing consumer.
        #[cfg(not(feature = "v8-host"))]
        let _ = &timing;
        #[cfg(not(feature = "v8-host"))]
        let (source, encoding) =
            crate::network::decode_navigation_text(&bytes, content_type.as_deref());
        #[cfg(feature = "v8-host")]
        let capture = Arc::new(Mutex::new(NavigationBodyCapture {
            source: String::new(),
            bytes: bytes.len(),
            completed: false,
        }));
        #[cfg(feature = "v8-host")]
        {
            let observed = capture.clone();
            let stream = crate::html_encoding_replay::BufferedDecodedInput::stream(
                bytes,
                content_type.clone(),
                None,
                Some(Box::new(move |text| {
                    let mut capture = observed.lock().expect("buffered navigation capture");
                    capture.source = text.into();
                    capture.completed = true;
                })),
            );
            let received = Instant::now();
            timing
                .response(oryn_js::navigation_timing::NavigationResponse {
                    url: url.to_string(),
                    status: 200,
                    headers: &response_headers,
                    fetch: timing.origin().0,
                    received,
                    details: None,
                    no_referrer: false,
                })
                .map_err(RuntimeError::Network)?;
            timing.finish(
                received,
                capture.lock().expect("buffered navigation capture").bytes as u64,
            );
            let mut stream = stream.with_navigation_timing(timing);
            stream.content_language = oryn_dom::language::content_language_header(
                response_headers
                    .iter()
                    .filter(|(name, _)| name.eq_ignore_ascii_case("content-language"))
                    .map(|(_, value)| value.as_str()),
            );
            self.load_document(
                url.to_string(),
                "",
                None,
                Some(stream),
                history,
                &BTreeMap::new(),
                Vec::new(),
            )?;
        }
        #[cfg(not(feature = "v8-host"))]
        {
            let mut parsed =
                oryn_dom::xml::parse_navigation(&source, content_type.as_deref(), url.as_str());
            if let Some(metadata) = parsed
                .dom
                .get_mut(parsed.document)
                .ok()
                .and_then(|node| node.document_metadata.as_mut())
            {
                metadata.encoding = encoding.into();
                metadata.content_language = oryn_dom::language::content_language_header(
                    response_headers
                        .iter()
                        .filter(|(name, _)| name.eq_ignore_ascii_case("content-language"))
                        .map(|(_, value)| value.as_str()),
                );
            }
            self.load_document(
                url.to_string(),
                &source,
                Some(parsed),
                None,
                history,
                &BTreeMap::new(),
            )?;
        }
        #[cfg(feature = "v8-host")]
        {
            self.pending_navigation_response = Some(PendingNavigationResponse {
                generation: self.document_generation,
                root_identity: self
                    .executable
                    .as_ref()
                    .and_then(|page| page.form_history_root_identity())
                    .map(|(realm, generation, epoch)| (u64::from(realm.0), generation, epoch)),
                current: url.clone(),
                status: 200,
                response_headers,
                essence: content_type
                    .as_deref()
                    .unwrap_or("text/html")
                    .split(';')
                    .next()
                    .unwrap_or("text/html")
                    .trim()
                    .to_ascii_lowercase(),
                capture,
            });
            self.finish_pending_navigation_response()?;
        }
        Ok(NavigationResult {
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
        })
    }

    fn controlled_navigation(
        &mut self,
        request: crate::navigation_control::NavigationRequest,
        emitter: crate::navigation_control::NavigationEmitter,
    ) -> Result<(), RuntimeError> {
        use crate::navigation_control::{NavigationCommand, NavigationEvent};
        #[cfg(feature = "v8-host")]
        self.cancel_navigation_transaction(RuntimeError::NavigationCancelled);
        emitter
            .publish(NavigationEvent::Started)
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        self.active_navigation = Some(emitter.clone());
        self.navigation_committed = false;
        let previous_generation = self.document_generation;
        #[cfg(feature = "v8-host")]
        let result = match request.command.clone() {
            NavigationCommand::Navigate { url } => {
                self.begin_browser_navigation(url, HistoryUpdate::Push, None, None, None)
            }
            NavigationCommand::Back => self.begin_history_navigation(-1),
            NavigationCommand::Forward => self.begin_history_navigation(1),
            NavigationCommand::Refresh => self.begin_refresh_navigation(),
        };
        #[cfg(feature = "v8-host")]
        let result = match result {
            Ok(NavigationProgress::Joint(receipt, result)) => {
                // The joint receipt owns readiness. A root among its destinations
                // must not complete this controller before the other routes commit.
                if self
                    .active_navigation
                    .as_ref()
                    .is_some_and(|active| active.same_job(&emitter))
                {
                    self.active_navigation = None;
                    self.navigation_committed = false;
                }
                self.pending_execution_replies.push(PendingNavigationReply {
                    joint_interactive_published: false,
                    joint_eager_reply: request.strategy
                        == crate::navigation_control::PageLoadStrategy::Eager,
                    joint_checkpoint: None,
                    joint_publication: None,
                    joint: Some(receipt),
                    generation: self.document_generation,
                    load_epoch: None,
                    identity: None,
                    deadline: Instant::now() + self.config.navigation_timeout,
                    result,
                    reply: NavigationReply::JointEvents(emitter.clone(), None),
                });
                return Ok(());
            }
            Ok(NavigationProgress::Pending(_)) => return Ok(()),
            Ok(NavigationProgress::Ready(result)) => Ok(result),
            Err(error) => Err(error),
        };
        #[cfg(not(feature = "v8-host"))]
        let result = match request.command {
            NavigationCommand::Navigate { url } => self.navigate(url, HistoryUpdate::Push),
            NavigationCommand::Back => self.traverse_history(-1),
            NavigationCommand::Forward => self.traverse_history(1),
            NavigationCommand::Refresh => self.refresh(),
        };
        let publication = (|| {
            match result {
                Ok(_) => {
                    // A new document publishes its actual commit/readiness in
                    // load_document. It may already have completed while parsing.
                    if self.active_navigation.is_none() {
                        return Ok(());
                    }
                    if !self.navigation_committed {
                        emitter.publish(NavigationEvent::Committed {
                            document_generation: self.document_generation,
                            url: self.url.clone(),
                        })?;
                        self.navigation_committed = true;
                    }
                    // Same-document fragment navigation has no new parser/load.
                    let same_document = self.document_generation == previous_generation;
                    if same_document || self.lifecycle.contains(&LifecycleState::DomContentLoaded) {
                        emitter.publish(NavigationEvent::Interactive {
                            document_generation: self.document_generation,
                        })?;
                    }
                    if same_document || self.lifecycle.contains(&LifecycleState::Load) {
                        emitter.publish(NavigationEvent::Complete {
                            document_generation: self.document_generation,
                        })?;
                        self.active_navigation = None;
                        self.navigation_committed = false;
                    }
                    Ok(())
                }
                Err(RuntimeError::HistoryBoundary | RuntimeError::NavigationCancelled) => {
                    emitter.publish(NavigationEvent::CompleteWithoutCommit)?;
                    self.active_navigation = None;
                    self.navigation_committed = false;
                    Ok(())
                }
                Err(error) => {
                    let reason = emitter
                        .checkpoint()
                        .err()
                        .unwrap_or_else(|| error.navigation_stop());
                    let _ = emitter.publish(NavigationEvent::Stopped(reason.clone()));
                    let cause = RuntimeError::NavigationStopped(reason);
                    Err(self.stop_failed_navigation_load(cause).navigation_stop())
                }
            }
        })();
        publication.map_err(RuntimeError::NavigationStopped)
    }
}
