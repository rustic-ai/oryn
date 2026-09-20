// Included in runtime.rs; typed accessibility commands share the native actor.
#[cfg(feature = "v8-host")]
enum AccessibilityReply {
    Text(oneshot::Sender<Result<oryn_js::accessibility::TextSnapshot, RuntimeError>>),
    Worker(oneshot::Sender<Result<crate::worker::WorkerResponse, RuntimeError>>),
}
#[cfg(feature = "v8-host")]
impl AccessibilityReply {
    fn closed(&self) -> bool {
        match self {
            Self::Text(reply) => reply.is_closed(),
            Self::Worker(reply) => reply.is_closed(),
        }
    }
    fn send(self, result: Result<oryn_js::accessibility::TextSnapshot, RuntimeError>) {
        match self {
            Self::Text(reply) => {
                let _ = reply.send(result);
            }
            Self::Worker(reply) => {
                let _ =
                    reply.send(result.map(|snapshot| {
                        crate::worker::WorkerResponse::AccessibilityText { snapshot }
                    }));
            }
        }
    }
}
#[cfg(feature = "v8-host")]
struct PendingAccessibility {
    operation: crate::executable::NativeAccessibilityOperation,
    reply: AccessibilityReply,
    deadline: Instant,
    retry: Instant,
    error: Option<RuntimeError>,
}
#[cfg(feature = "v8-host")]
impl PageState {
    fn enqueue_accessibility(
        &mut self,
        scope: Option<crate::host::webdriver_script::AutomationRealmSnapshot>,
        target: SemanticRef,
        command: oryn_js::accessibility::TextCommand,
        reply: AccessibilityReply,
    ) {
        let prepared = (|| {
            if self.pending_accessibility.len() >= 32 {
                return Err(RuntimeError::Script(
                    "accessibility operation queue is full".into(),
                ));
            }
            self.pending_accessibility.try_reserve(1).map_err(|_| {
                RuntimeError::Script("accessibility operation queue allocation".into())
            })?;
            if let Some(scope) = &scope {
                self.scoped_worker_request(
                    scope.clone(),
                    crate::worker::WorkerRequest::AccessibilityText {
                        target,
                        command: oryn_js::accessibility::TextCommand::Read,
                    },
                )?;
            } else {
                self.validate_target(target)?;
            }
            let operation = self
                .executable
                .as_ref()
                .ok_or(RuntimeError::StaleSemanticRef)?
                .begin_accessibility_operation(
                    crate::semantic::unpack_node_ref(target.node),
                    scope.as_ref(),
                    &command,
                )
                .map_err(map_host_error)?;
            let retained = self
                .pending_accessibility
                .iter()
                .fold(operation.receipt.payload_bytes(), |sum, pending| {
                    sum.saturating_add(pending.operation.receipt.payload_bytes())
                });
            if retained > 64 * 1024 * 1024 {
                return Err(RuntimeError::Script(
                    "accessibility retained payload budget exceeded".into(),
                ));
            }
            Ok(operation)
        })();
        match prepared {
            Ok(operation) => {
                let now = Instant::now();
                self.pending_accessibility.push_back(PendingAccessibility {
                    operation,
                    reply,
                    deadline: now + self.config.navigation_timeout,
                    retry: now,
                    error: None,
                });
            }
            Err(error) => reply.send(Err(error)),
        }
    }
    fn accessibility_operation_delay(&self) -> Option<Duration> {
        let mut active = false;
        self.pending_accessibility
            .iter()
            .map(|pending| {
                if pending.reply.closed() || !pending.operation.receipt.retry_safe() {
                    return Duration::ZERO;
                }
                let mut wake = pending.deadline;
                if !pending.operation.receipt.owner().execution_suspended() && !active {
                    active = true;
                    wake = wake.min(pending.retry);
                }
                wake.saturating_duration_since(Instant::now())
            })
            .min()
    }
    fn poll_accessibility_operations(&mut self) {
        let count = self.pending_accessibility.len();
        let mut active = false;
        for _ in 0..count {
            let Some(mut pending) = self.pending_accessibility.pop_front() else {
                break;
            };
            if pending.reply.closed() {
                pending.operation.receipt.cancel();
                continue;
            }
            let now = Instant::now();
            if now >= pending.deadline {
                pending.operation.receipt.cancel();
                pending
                    .reply
                    .send(Err(pending.error.take().unwrap_or_else(|| {
                        RuntimeError::Script("accessibility operation deadline exceeded".into())
                    })));
                continue;
            }
            if !pending.operation.receipt.retry_safe() {
                pending.operation.receipt.cancel();
                pending.reply.send(Err(RuntimeError::StaleSemanticRef));
                continue;
            }
            if pending.operation.receipt.owner().execution_suspended() {
                self.pending_accessibility.push_back(pending);
                continue;
            }
            if active || now < pending.retry {
                active = true;
                self.pending_accessibility.push_back(pending);
                continue;
            }
            active = true;
            let Some(executable) = self.executable.as_mut() else {
                pending.operation.receipt.cancel();
                pending.reply.send(Err(RuntimeError::StaleSemanticRef));
                continue;
            };
            if !executable
                .automation_realms()
                .iter()
                .any(|scope| scope.same_document_as(&pending.operation.selected) && scope.live)
            {
                pending.operation.receipt.cancel();
                pending.reply.send(Err(RuntimeError::StaleSemanticRef));
                continue;
            }
            match executable.poll_accessibility_operation(&mut pending.operation) {
                Ok(Some(snapshot)) => pending.reply.send(Ok(snapshot)),
                result => {
                    if let Err(error) = result {
                        if matches!(error, crate::host::HostError::LimitExceeded { .. })
                            || !pending.operation.receipt.retry_safe()
                        {
                            pending.operation.receipt.cancel();
                            pending.reply.send(Err(map_host_error(error)));
                            continue;
                        }
                        if pending.error.is_none() {
                            pending.error = Some(map_host_error(error));
                        }
                    }
                    pending.retry = Instant::now() + Duration::from_millis(10);
                    self.pending_accessibility.push_back(pending);
                }
            }
        }
    }
}
impl PageHandle {
    pub async fn accessibility_text(
        &self,
        target: SemanticRef,
        command: oryn_js::accessibility::TextCommand,
    ) -> Result<oryn_js::accessibility::TextSnapshot, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::AccessibilityText {
                target,
                command: command.clone(),
            })
            .await
        {
            return match response? {
                crate::worker::WorkerResponse::AccessibilityText { snapshot } => Ok(snapshot),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::AccessibilityText {
                target,
                command,
                reply,
            })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }
}
impl PageState {
    fn accessibility_text(
        &mut self,
        target: SemanticRef,
        command: oryn_js::accessibility::TextCommand,
    ) -> Result<oryn_js::accessibility::TextSnapshot, RuntimeError> {
        if !matches!(&command, oryn_js::accessibility::TextCommand::Read) {
            return Err(RuntimeError::Script(
                "mutating accessibility commands require retained actor admission".into(),
            ));
        }
        self.validate_target(target)?;
        #[cfg(feature = "v8-host")]
        {
            let executable = self
                .executable
                .as_mut()
                .ok_or(RuntimeError::StaleSemanticRef)?;
            executable
                .accessibility_text(crate::semantic::unpack_node_ref(target.node), &command)
                .map_err(|e| RuntimeError::Script(e.to_string()))
        }
        #[cfg(not(feature = "v8-host"))]
        {
            let _ = command;
            Err(RuntimeError::Script(
                "native accessibility text requires a page worker".into(),
            ))
        }
    }
}
impl PageHandle {
    pub async fn accessibility_snapshot(
        &self,
    ) -> Result<crate::accessibility_platform::NativeSnapshot, RuntimeError> {
        #[cfg(unix)]
        if let Some(response) = self
            .worker_request(crate::worker::WorkerRequest::AccessibilitySnapshot)
            .await
        {
            return match response? {
                crate::worker::WorkerResponse::AccessibilitySnapshot { snapshot } => Ok(*snapshot),
                response => Err(unexpected_worker_response(response)),
            };
        }
        let (reply, response) = oneshot::channel();
        self.local_sender()?
            .send(PageCommand::AccessibilitySnapshot { reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }
}
impl PageState {
    fn accessibility_snapshot(
        &mut self,
    ) -> Result<crate::accessibility_platform::NativeSnapshot, RuntimeError> {
        #[cfg(not(feature = "v8-host"))]
        {
            Err(RuntimeError::EvaluationUnavailable)
        }
        #[cfg(feature = "v8-host")]
        {
            self.poll_page_tasks()?;
            let plan = self.prepare_capture_plan_until_ready(false, None)?;
            let mut snapshot = self.accessibility_snapshot_prepared(plan.snapshot.clone())?;
            snapshot.paint = complete_paint_snapshot(plan)?;
            Ok(snapshot)
        }
    }
    #[cfg(feature = "v8-host")]
    fn accessibility_snapshot_prepared(
        &mut self,
        paint: PaintSnapshot,
    ) -> Result<crate::accessibility_platform::NativeSnapshot, RuntimeError> {
        let stamp = self
            .executable
            .as_ref()
            .and_then(|page| page.selected_accessibility_stamp())
            .ok_or(RuntimeError::StaleSemanticRef)?;
        let semantic = self.semantic_observation(ProjectionProfile::Full);
        if semantic.observation.revision != paint.revision
            || semantic.observation.page != paint.page
        {
            return Err(RuntimeError::Semantic(
                "accessibility capture checkpoint changed".into(),
            ));
        }
        let editable = semantic
            .observation
            .nodes
            .iter()
            .filter(|node| node.actions.contains(&SemanticAction::Type))
            .map(|node| node.semantic_ref)
            .collect::<BTreeSet<_>>();
        let targets: Vec<_> = semantic
            .accessibility
            .iter()
            .filter(|node| {
                !node.excluded
                    && (matches!(
                        node.role.as_str(),
                        "textbox" | "searchbox" | "document" | "text" | "statictext"
                    ) || editable.contains(&node.semantic_ref))
            })
            .map(|node| node.semantic_ref)
            .collect();
        if targets.len() > 65_536 {
            return Err(RuntimeError::Semantic(
                "accessibility text object budget exceeded".into(),
            ));
        }
        let mut text = Vec::new();
        let mut units = 0usize;
        let mut retained = 0usize;
        for target in targets {
            let value = self
                .accessibility_text(target, oryn_js::accessibility::TextCommand::Read)
                .map_err(|error| {
                    RuntimeError::Semantic(format!("native accessibility text capture: {error}"))
                })?;
            units = units
                .checked_add(value.value.len())
                .ok_or_else(|| RuntimeError::Semantic("accessibility text size overflow".into()))?;
            if units > 2 * 1024 * 1024 {
                return Err(RuntimeError::Semantic(
                    "accessibility text snapshot budget exceeded".into(),
                ));
            }
            retained = retained
                .checked_add(value.retained_bytes().ok_or_else(|| {
                    RuntimeError::Semantic("accessibility text storage overflow".into())
                })?)
                .ok_or_else(|| {
                    RuntimeError::Semantic("accessibility text storage overflow".into())
                })?;
            if retained > 128 * 1024 * 1024 {
                return Err(RuntimeError::Semantic(
                    "accessibility text and geometry storage budget exceeded".into(),
                ));
            }
            text.push((target, value));
        }
        if self
            .executable
            .as_ref()
            .and_then(|page| page.selected_accessibility_stamp())
            != Some(stamp)
        {
            return Err(RuntimeError::Semantic(
                "accessibility native lifetime or paint state changed".into(),
            ));
        }
        let pickers = self
            .executable
            .as_ref()
            .ok_or(RuntimeError::StaleSemanticRef)?
            .accessibility_pickers()
            .map_err(|error| RuntimeError::Semantic(error.to_string()))?
            .into_iter()
            .map(|(node, picker)| {
                (
                    SemanticRef {
                        page_id: semantic.observation.page.page_id,
                        document_generation: semantic.observation.page.document_generation,
                        node: (u64::from(node.generation) << 32) | u64::from(node.slot),
                    },
                    picker,
                )
            })
            .collect();
        let permission_prompts = self
            .executable
            .as_ref()
            .ok_or(RuntimeError::StaleSemanticRef)?
            .accessibility_permission_prompts()
            .map_err(|error| RuntimeError::Semantic(error.to_string()))?
            .into_iter()
            .map(|(node, prompt)| {
                (
                    SemanticRef {
                        page_id: semantic.observation.page.page_id,
                        document_generation: semantic.observation.page.document_generation,
                        node: (u64::from(node.generation) << 32) | u64::from(node.slot),
                    },
                    prompt,
                )
            })
            .collect();
        let file_pickers = self
            .executable
            .as_ref()
            .ok_or(RuntimeError::StaleSemanticRef)?
            .accessibility_file_pickers()
            .map_err(|error| RuntimeError::Semantic(error.to_string()))?
            .into_iter()
            .map(|(node, picker)| {
                (
                    SemanticRef {
                        page_id: semantic.observation.page.page_id,
                        document_generation: semantic.observation.page.document_generation,
                        node: (u64::from(node.generation) << 32) | u64::from(node.slot),
                    },
                    picker,
                )
            })
            .collect();
        let boundary_journal = self
            .executable
            .as_ref()
            .ok_or(RuntimeError::StaleSemanticRef)?
            .accessibility_boundary_snapshot()
            .map_err(|error| RuntimeError::Semantic(error.to_string()))?;
        if text
            .iter()
            .any(|(_, text)| text.boundary_revision != boundary_journal.current)
        {
            return Err(RuntimeError::Semantic(
                "accessibility boundary checkpoint changed".into(),
            ));
        }
        Ok(crate::accessibility_platform::NativeSnapshot {
            file_pickers,
            permission_prompts,
            pickers,
            boundary_journal,
            semantic,
            paint,
            text,
        })
    }
}

#[cfg(all(test, feature = "v8-host", feature = "in-process-probe"))]
mod accessibility_runtime_tests {
    use super::*;
    #[tokio::test]
    async fn unchanged_child_queries_keep_revision_and_native_text_identity() {
        let browser = Browser::in_process_probe();
        let context = browser.new_context();
        let page = context.new_page();
        page.load_html(
            "https://ax.test/",
            "<iframe srcdoc=\"<textarea aria-label='Message'>child</textarea>\"></iframe>",
        )
        .await
        .unwrap();
        let crate::host::webdriver_script::FrameResponse::Inventory(inventory) = page
            .webdriver_frames(crate::host::webdriver_script::FrameCommand::Inventory)
            .await
            .unwrap()
        else {
            panic!("inventory")
        };
        let child = inventory
            .into_iter()
            .find(|scope| scope.live && scope.parent_realm.is_some())
            .unwrap();
        let scoped = page.with_frame_scope(Some(child));
        // Initial layout/environment installation may establish a new checkpoint.
        let _ = scoped.accessibility_snapshot().await;
        let first = scoped.accessibility_snapshot().await.unwrap();
        let second = scoped.accessibility_snapshot().await.unwrap();
        assert_eq!(
            first.semantic.observation.revision,
            second.semantic.observation.revision
        );
        assert_eq!(first.text.len(), 1);
        assert_eq!(
            first.text[0].1.value,
            "child".encode_utf16().collect::<Vec<_>>()
        );
        let encoded = serde_json::to_vec(&second).unwrap();
        let decoded: crate::accessibility_platform::NativeSnapshot =
            serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.text[0].0, first.text[0].0);
        let target = first.text[0].0;
        let value = &first.text[0].1;
        let selection = scoped
            .accessibility_text(
                target,
                oryn_js::accessibility::TextCommand::Select {
                    start: 1,
                    end: 3,
                    expected_value: value.value.clone(),
                    expected_selection_revision: value.selection_revision,
                },
            )
            .await
            .unwrap();
        assert_eq!((selection.start, selection.end), (1, 3));
        page.close().await.unwrap();
        assert!(
            scoped
                .accessibility_text(target, oryn_js::accessibility::TextCommand::Read)
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn native_modal_exclusion_is_reversible_without_author_attribute_mutation() {
        let browser = Browser::in_process_probe();
        let context = browser.new_context();
        let page = context.new_page();
        page.load_html(
            "https://ax.test/",
            "<button id=outside>Outside</button><dialog id=modal><button>Inside</button></dialog>",
        )
        .await
        .unwrap();
        page.evaluate("modal.showModal()").await.unwrap();
        let snapshot = page
            .observe_semantics(ProjectionProfile::Full)
            .await
            .unwrap();
        let outside = snapshot
            .accessibility
            .iter()
            .find(|node| node.name == "Outside")
            .unwrap();
        assert!(outside.excluded);
        assert_eq!(outside.exclusion_reason.as_deref(), Some("native_inert"));
        page.evaluate("modal.close()").await.unwrap();
        let snapshot = page
            .observe_semantics(ProjectionProfile::Full)
            .await
            .unwrap();
        assert!(
            !snapshot
                .accessibility
                .iter()
                .find(|node| node.name == "Outside")
                .unwrap()
                .excluded
        );
        page.close().await.unwrap();
    }
}

impl PageHandle {
    pub(crate) async fn complete_native_file_picker(
        &self,
        target: SemanticRef,
        expected: oryn_dom::FilePicker,
        paths: Option<Vec<std::path::PathBuf>>,
    ) -> Result<(), RuntimeError> {
        let files = if let Some(paths) = paths {
            let context = BrowserContext {
                inner: self.context.upgrade().ok_or(RuntimeError::Closed)?,
            };
            Some(
                context
                    .webdriver_upload_files(self.page_id(), &paths)?
                    .into_iter()
                    .map(|file| oryn_js::accessibility::PickerFile {
                        name: file.file.filename,
                        media_type: file.file.media_type,
                        bytes: file.file.bytes,
                        last_modified: file.last_modified,
                    })
                    .collect(),
            )
        } else {
            None
        };
        self.accessibility_text(
            target,
            oryn_js::accessibility::TextCommand::FilePicker { expected, files },
        )
        .await?;
        Ok(())
    }
}

#[cfg(all(test, feature = "v8-host"))]
mod retained_accessibility_actor_tests {
    use super::*;
    fn page() -> PageState {
        let mut page = PageState::new(
            PageId(752),
            None,
            BrowserConfig::default(),
            ContextOptions::default(),
            None,
            #[cfg(unix)]
            None,
        );
        page.load_document(
            "https://ax.test/".into(),
            "<textarea id=t>abc</textarea>",
            None,
            None,
            HistoryUpdate::Push,
            &BTreeMap::new(),
            Vec::new(),
        )
        .unwrap();
        page
    }
    #[test]
    fn pending_reply_deadline_preserves_original_error_without_retry_spin() {
        let mut page = page();
        let target = page.resolve_selector("#t", SemanticAction::Type).unwrap();
        let (reply, mut receiver) = oneshot::channel();
        page.enqueue_accessibility(
            None,
            target,
            oryn_js::accessibility::TextCommand::Read,
            AccessibilityReply::Text(reply),
        );
        assert_eq!(page.pending_accessibility.len(), 1);
        let future = Instant::now() + Duration::from_secs(60);
        {
            let pending = page.pending_accessibility.front_mut().unwrap();
            pending.retry = future;
            pending.deadline = future;
        }
        assert!(page.accessibility_operation_delay().unwrap() > Duration::ZERO);
        page.poll_accessibility_operations();
        assert!(receiver.try_recv().is_err());
        assert_eq!(page.pending_accessibility.len(), 1);
        {
            let pending = page.pending_accessibility.front_mut().unwrap();
            pending.deadline = Instant::now();
            pending.error = Some(RuntimeError::Script("original allocation failure".into()));
        }
        page.poll_accessibility_operations();
        assert!(page.pending_accessibility.is_empty());
        assert!(page.accessibility_operation_delay().is_none());
        assert!(
            matches!(receiver.try_recv().unwrap(),Err(RuntimeError::Script(message))if message=="original allocation failure")
        );
    }
    #[test]
    fn closed_reply_releases_original_operation_without_running_callbacks() {
        let mut page = page();
        let target = page.resolve_selector("#t", SemanticAction::Type).unwrap();
        let (reply, receiver) = oneshot::channel();
        page.enqueue_accessibility(
            None,
            target,
            oryn_js::accessibility::TextCommand::Read,
            AccessibilityReply::Text(reply),
        );
        drop(receiver);
        page.poll_accessibility_operations();
        assert!(page.pending_accessibility.is_empty());
        assert!(page.accessibility_operation_delay().is_none());
    }
    #[test]
    fn suspended_owner_has_only_request_deadline_wakeup_and_restores_due_work() {
        let mut page = page();
        let target = page.resolve_selector("#t", SemanticAction::Type).unwrap();
        let (reply, mut receiver) = oneshot::channel();
        page.enqueue_accessibility(
            None,
            target,
            oryn_js::accessibility::TextCommand::Read,
            AccessibilityReply::Text(reply),
        );
        let owner = page
            .pending_accessibility
            .front()
            .unwrap()
            .operation
            .receipt
            .owner();
        owner.set_execution_suspended(true);
        page.pending_accessibility.front_mut().unwrap().deadline =
            Instant::now() + Duration::from_secs(60);
        assert!(page.accessibility_operation_delay().unwrap() > Duration::from_secs(59));
        page.poll_accessibility_operations();
        assert_eq!(page.pending_accessibility.len(), 1);
        assert!(receiver.try_recv().is_err());
        assert!(page.accessibility_operation_delay().unwrap() > Duration::from_secs(59));
        owner.set_execution_suspended(false);
        assert!(page.accessibility_operation_delay().unwrap().is_zero());
        page.poll_accessibility_operations();
        assert!(page.pending_accessibility.is_empty());
        assert!(receiver.try_recv().unwrap().is_ok());
    }
    #[test]
    fn cached_original_document_retains_ax_request_without_running_in_replacement() {
        let mut page = page();
        // Raw load_document commits before the actor's asynchronous load barrier.
        let deadline = Instant::now() + Duration::from_secs(3);
        while !page.executable.as_ref().unwrap().document_load_complete() {
            assert!(
                Instant::now() < deadline,
                "source load must complete before BFCache admission"
            );
            page.poll_page_tasks().unwrap();
            std::thread::yield_now();
        }
        assert!(page.executable.as_ref().unwrap().document_load_complete());
        let target = page.resolve_selector("#t", SemanticAction::Type).unwrap();
        let (reply, mut receiver) = oneshot::channel();
        page.enqueue_accessibility(
            None,
            target,
            oryn_js::accessibility::TextCommand::Read,
            AccessibilityReply::Text(reply),
        );
        let owner = page
            .pending_accessibility
            .front()
            .unwrap()
            .operation
            .receipt
            .owner();
        let future = Instant::now() + Duration::from_secs(60);
        {
            let pending = page.pending_accessibility.front_mut().unwrap();
            pending.retry = future;
            pending.deadline = future + Duration::from_secs(60);
        }
        page.load_document(
            "https://ax.test/next".into(),
            "<textarea id=t>replacement</textarea>",
            None,
            None,
            HistoryUpdate::Push,
            &BTreeMap::new(),
            Vec::new(),
        )
        .unwrap();
        assert!(owner.execution_suspended());
        assert!(page.back_forward_cache[&0].reusable);
        page.pending_accessibility.front_mut().unwrap().retry = Instant::now();
        assert!(page.accessibility_operation_delay().unwrap() > Duration::from_secs(60));
        page.poll_accessibility_operations();
        assert!(receiver.try_recv().is_err());
        assert_eq!(page.pending_accessibility.len(), 1);
        page.restore_cached_document(0, "https://ax.test/".into())
            .unwrap()
            .unwrap();
        assert!(!owner.execution_suspended());
        page.poll_accessibility_operations();
        let result = receiver.try_recv().unwrap().unwrap();
        assert_eq!(result.value, "abc".encode_utf16().collect::<Vec<_>>());
        assert!(page.pending_accessibility.is_empty());
    }
    #[test]
    fn replaced_document_cannot_receive_queued_accessibility_operation() {
        let mut page = page();
        let target = page.resolve_selector("#t", SemanticAction::Type).unwrap();
        let (reply, mut receiver) = oneshot::channel();
        page.enqueue_accessibility(
            None,
            target,
            oryn_js::accessibility::TextCommand::Read,
            AccessibilityReply::Text(reply),
        );
        page.executable.as_mut().unwrap().evaluate("document.open();document.write('<textarea id=t>replacement</textarea>');document.close()").unwrap();
        page.poll_accessibility_operations();
        assert!(page.pending_accessibility.is_empty());
        assert!(receiver.try_recv().unwrap().is_err());
    }
}
