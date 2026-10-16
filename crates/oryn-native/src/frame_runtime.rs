// Included in runtime: independent semantic/layout state for each selected frame.
// The real executable remains a single page-owned isolate; it is lent only for
// the duration of a serialized command and restored on every ordinary error.
#[cfg(feature = "v8-host")]
struct ScopedActionNavigationUpdate {
    action_id: ActionId,
    url: Option<String>,
    root_replaced: bool,
}

impl PageState {
    fn scoped_worker_request(
        &mut self,
        scope: crate::host::webdriver_script::AutomationRealmSnapshot,
        request: crate::worker::WorkerRequest,
    ) -> Result<crate::worker::WorkerResponse, RuntimeError> {
        self.scoped_worker_request_with_paint_media(scope, request, false, None)
    }
    fn scoped_worker_request_with_paint_media(
        &mut self,
        scope: crate::host::webdriver_script::AutomationRealmSnapshot,
        request: crate::worker::WorkerRequest,
        print_media: bool,
        viewport_override: Option<ViewportState>,
    ) -> Result<crate::worker::WorkerResponse, RuntimeError> {
        self.scoped_worker_request_with_paint_mode(
            scope,
            request,
            print_media,
            viewport_override,
            false,
        )
    }
    fn scoped_worker_request_with_paint_mode(
        &mut self,
        scope: crate::host::webdriver_script::AutomationRealmSnapshot,
        request: crate::worker::WorkerRequest,
        print_media: bool,
        viewport_override: Option<ViewportState>,
        native_scene: bool,
    ) -> Result<crate::worker::WorkerResponse, RuntimeError> {
        #[cfg(not(feature = "v8-host"))]
        {
            let _ = (scope, request, print_media, viewport_override, native_scene);
            Err(RuntimeError::EvaluationUnavailable)
        }
        #[cfg(feature = "v8-host")]
        {
            use crate::worker::{WorkerRequest, WorkerResponse};
            if matches!(&request, WorkerRequest::Scoped { .. }) {
                return Err(RuntimeError::ProtocolViolation(
                    "nested frame envelope".into(),
                ));
            }
            if matches!(&request,WorkerRequest::WebdriverFrames{command} if !matches!(command,crate::host::webdriver_script::FrameCommand::SelectSemantic{..}))
            {
                let WorkerRequest::WebdriverFrames { command } = request else {
                    unreachable!()
                };
                return Ok(WorkerResponse::WebdriverFrames {
                    result: self.webdriver_frames(command),
                });
            }
            let read_only = matches!(
                &request,
                WorkerRequest::Observe { .. }
                    | WorkerRequest::ObserveWith { .. }
                    | WorkerRequest::ObserveSemantic { .. }
                    | WorkerRequest::ObserveSemanticV6 { .. }
                    | WorkerRequest::ResolveFocusedForm
                    | WorkerRequest::ResolveSelector { .. }
                    | WorkerRequest::ResolveSemantic { .. }
                    | WorkerRequest::ResolveTarget { .. }
                    | WorkerRequest::Extract { .. }
                    | WorkerRequest::ComputedStyle { .. }
                    | WorkerRequest::AccessibilitySnapshot
                    | WorkerRequest::BoxModel { .. }
                    | WorkerRequest::ObserveDelta { .. }
                    | WorkerRequest::ObserveSemanticDelta { .. }
                    | WorkerRequest::PaintSnapshot
                    | WorkerRequest::PrintSnapshot { .. }
                    | WorkerRequest::WaitFor { .. }
                    | WorkerRequest::WaitForUntil { .. }
            ) || matches!(
                &request,
                WorkerRequest::AccessibilityText {
                    command: oryn_js::accessibility::TextCommand::Read,
                    ..
                }
            );
            let (mut response, navigation) = self.with_scoped_frame_result(
                scope,
                print_media,
                viewport_override,
                native_scene,
                read_only,
                |frame| {
                    let response = if native_scene {
                        frame.publish_native_rendering_opportunity()?;
                        WorkerResponse::PaintSnapshot {
                            snapshot: Box::new(frame.presentation.scene.clone().ok_or_else(
                                || {
                                    RuntimeError::PaintUnavailable(
                                        "frame presentation was deferred".into(),
                                    )
                                },
                            )?),
                        }
                    } else if (print_media || viewport_override.is_some())
                        && matches!(&request, WorkerRequest::PaintSnapshot)
                    {
                        WorkerResponse::PaintSnapshot {
                            snapshot: Box::new(complete_paint_snapshot(
                                frame.prepare_capture_plan_until_ready(print_media, None)?,
                            )?),
                        }
                    } else {
                        frame.dispatch_scoped_worker(request)?
                    };
                    // Print-media geometry must not replace the live screen inventory.
                    if !print_media && let WorkerResponse::PaintSnapshot { snapshot } = &response {
                        frame
                            .executable
                            .as_ref()
                            .ok_or(RuntimeError::EvaluationUnavailable)?
                            .install_selected_paint_capture_inventory(snapshot)
                            .map_err(map_host_error)?;
                    }
                    Ok(response)
                },
            )?;
            if let (Some(navigation), WorkerResponse::Action { result }) =
                (navigation, &mut response)
                && result.action_id == navigation.action_id
            {
                if let Some(url) = navigation.url {
                    result.effects.push(Effect::Navigation { url });
                }
                // A selected-frame delta cannot describe a replaced root DOM.
                if navigation.root_replaced {
                    result.delta = None;
                }
            }
            if let WorkerResponse::WebdriverActions { feedback, .. } = &mut response {
                feedback.page_generation = self.document_generation;
            }
            Ok(response)
        }
    }

    // Every operation borrows the selected frame's executable for one turn.
    // Restore the source scope and host before a related root may be scheduled.
    #[cfg(feature = "v8-host")]
    fn with_scoped_frame<T>(
        &mut self,
        scope: crate::host::webdriver_script::AutomationRealmSnapshot,
        print_media: bool,
        viewport_override: Option<ViewportState>,
        native_scene: bool,
        read_only: bool,
        operation: impl FnOnce(&mut PageState) -> Result<T, RuntimeError>,
    ) -> Result<T, RuntimeError> {
        self.with_scoped_frame_result(
            scope,
            print_media,
            viewport_override,
            native_scene,
            read_only,
            operation,
        )
        .map(|(value, _navigation)| value)
    }

    // Wait, WebDriver and recursive paint callers own different result types.
    // Navigation ownership is independent of optional worker-response metadata.
    #[cfg(feature = "v8-host")]
    fn with_scoped_frame_result<T>(
        &mut self,
        scope: crate::host::webdriver_script::AutomationRealmSnapshot,
        _print_media: bool,
        viewport_override: Option<ViewportState>,
        native_scene: bool,
        read_only: bool,
        operation: impl FnOnce(&mut PageState) -> Result<T, RuntimeError>,
    ) -> Result<(T, Option<ScopedActionNavigationUpdate>), RuntimeError> {
        if !native_scene {
            self.poll_page_tasks()?;
        }
        if self.terminal_close_requested() {
            return Err(RuntimeError::PageTaskYield);
        }
        let key = (
            self.document_generation,
            scope.realm.0,
            scope.document_generation,
            scope.stream_epoch,
        );
        let inventory = self
            .executable
            .as_ref()
            .ok_or(RuntimeError::EvaluationUnavailable)?
            .automation_realms();
        self.frame_states.retain(|key, state| {
            key.0 == self.document_generation
                && state.frame_scope.as_ref().is_some_and(|saved| {
                    inventory.iter().any(|entry| entry.same_document_as(saved))
                })
        });
        let frame_viewport = scope
            .frame
            .and_then(|node| {
                let packed = (u64::from(node.generation) << 32) | u64::from(node.slot);
                let layout = if self
                    .frame_scope
                    .as_ref()
                    .is_some_and(|entry| Some(entry.realm) == scope.parent_realm)
                    || inventory.iter().any(|entry| {
                        entry.parent_realm.is_none() && Some(entry.realm) == scope.parent_realm
                    }) {
                    self.layout.as_ref()
                } else {
                    self.frame_states
                        .values()
                        .find(|state| {
                            state
                                .frame_scope
                                .as_ref()
                                .is_some_and(|entry| Some(entry.realm) == scope.parent_realm)
                        })
                        .and_then(|state| state.layout.as_ref())
                };
                layout
                    .and_then(|layout| {
                        layout
                            .paint_projections
                            .get(&packed)
                            .map(|projection| &projection.box_model)
                            .or_else(|| {
                                layout
                                    .geometry
                                    .get(&packed)
                                    .and_then(|geometry| geometry.box_model.as_ref())
                            })
                    })
                    .map(|model| ViewportState {
                        width: model.content_box.width.max(1.),
                        height: model.content_box.height.max(1.),
                        device_pixel_ratio: self.viewport.device_pixel_ratio,
                    })
            })
            .unwrap_or(ViewportState {
                width: 300.,
                height: 150.,
                device_pixel_ratio: self.viewport.device_pixel_ratio,
            });
        let frame_viewport = viewport_override.unwrap_or(frame_viewport);
        let executable = self
            .executable
            .as_mut()
            .ok_or(RuntimeError::EvaluationUnavailable)?;
        let previous_scope = executable.current_automation_scope();
        executable
            .set_automation_scope(Some(scope.clone()))
            .map_err(map_host_error)?;
        let native_stamp = match executable.selected_accessibility_stamp() {
            Some(stamp) => stamp,
            None => {
                let _ = executable.set_automation_scope(previous_scope.clone());
                return Err(RuntimeError::ProtocolViolation(
                    "selected frame native checkpoint unavailable".into(),
                ));
            }
        };
        let snapshot = match executable.semantic_snapshot() {
            Ok(value) => value,
            Err(error) => {
                let _ = executable.set_automation_scope(previous_scope.clone());
                return Err(map_host_error(error));
            }
        };
        let selected_broker = executable.selected_network_broker();
        let mut frame = if let Some(frame) = self.frame_states.remove(&key) {
            frame
        } else {
            if self.frame_states.len() >= 256 {
                let _ = executable.set_automation_scope(previous_scope.clone());
                return Err(RuntimeError::ProtocolViolation(
                    "frame state capacity exhausted".into(),
                ));
            }
            let Some(next) = self.last_issued_document_generation.checked_add(1) else {
                let _ = executable.set_automation_scope(previous_scope.clone());
                return Err(RuntimeError::ProtocolViolation(
                    "semantic generation exhausted".into(),
                ));
            };
            self.last_issued_document_generation = next;
            let mut config = self.config.clone();
            config.document_generation_base = self.last_issued_document_generation;
            let options = ContextOptions {
                viewport: frame_viewport,
                color_scheme: self.color_scheme,
                reduced_motion: self.reduced_motion,
                ..Default::default()
            };
            let mut frame = Box::new(PageState::new(
                self.page_id,
                self.opener_page_id,
                config,
                options,
                self.broker.clone(),
                #[cfg(unix)]
                None,
            ));
            frame.frame_scope = Some(scope.clone());
            frame.lifecycle = vec![LifecycleState::DomContentLoaded, LifecycleState::Load];
            frame.network_quiet_since = Some(Instant::now());
            frame
        };
        // Cached, fresh, nested and replacement frame drivers all allocate in
        // the owning page namespace, never in a document-local counter.
        frame.action_ids = self.action_ids.clone();
        frame.page_task_terminal_control = self.page_task_terminal_control.clone();
        frame.broker = selected_broker;
        if frame.viewport != frame_viewport {
            frame.viewport = frame_viewport;
            frame.layout = None;
            frame.layout_input_cache = None;
        }
        frame.url = inventory
            .iter()
            .find(|entry| entry.realm == scope.realm)
            .map(|entry| entry.url.clone())
            .unwrap_or_else(|| scope.url.clone());
        frame.document = Some(snapshot);
        let checkpoint = (
            native_stamp.0,
            native_stamp.1,
            native_stamp.2,
            frame_viewport,
        );
        let changed = frame.frame_checkpoint != Some(checkpoint);
        if changed {
            frame.semantic_graph.reset(
                &frame.document.as_ref().unwrap().dom,
                frame.document_generation,
            );
        }
        frame.web_fonts = self.web_fonts.clone();
        let trace_boundary = frame.trace.snapshot().len();
        frame.executable = self.executable.take();
        let result = (|| {
            if frame.terminal_close_requested() {
                return Err(RuntimeError::PageTaskYield);
            }
            frame.sync_executable_semantics()?;
            if !native_scene {
                frame.refresh_layout()?;
            }
            // Do not fabricate a generic operation result after a layout yield.
            // The restoration block below still returns the borrowed executable,
            // automation scope and any existing action-navigation custody.
            if frame.terminal_close_requested() {
                return Err(RuntimeError::PageTaskYield);
            }
            if changed {
                frame.revision = frame.revision.checked_add(1).ok_or_else(|| {
                    RuntimeError::ProtocolViolation("frame revision exhausted".into())
                })?;
                frame.record_snapshot()?;
            }
            frame.frame_checkpoint = Some(checkpoint);
            let response = operation(&mut frame)?;
            Ok(response)
        })();
        self.executable = frame.executable.take();
        // The owner must pump this action's embedded navigation after the frame
        // returns its borrowed executable. Its semantic state cannot drive a root
        // navigation or silently lose a receipt when the old frame is retired.
        let action_navigation = frame.action_navigation.take();
        let mut action_root_navigation = frame.action_root_navigation.take();
        if frame.terminal_close_requested() {
            frame.action_root_navigation = action_root_navigation.take();
        }
        // The lent executable owns a page-wide diagnostic cursor. A scoped
        // projection/command may consume it into the frame trace, so retain
        // newly published records in the owner trace before fallible restoration.
        // The prefix boundary prevents replay on a subsequent scoped command.
        for event in frame.trace.snapshot().into_iter().skip(trace_boundary) {
            if matches!(
                &event.kind,
                TraceEventKind::ScriptDiagnostic { .. } | TraceEventKind::Console { .. }
            ) {
                self.trace.push_causal(
                    Revision(self.revision),
                    event.action_id,
                    event.causality,
                    event.kind,
                );
            }
        }
        let restoration = self
            .executable
            .as_mut()
            .ok_or(RuntimeError::EvaluationUnavailable)?
            .set_automation_scope(previous_scope.clone())
            .map_err(map_host_error);
        // Invalidation cannot turn a stale frame reference into another DOM.
        let live = self.executable.as_ref().is_some_and(|executable| {
            executable
                .automation_realms()
                .iter()
                .any(|entry| entry.same_document_as(&scope))
        });
        if !live && let Some(deferred) = frame.action_root_navigation.take() {
            // An invalidated frame cannot keep deferred custody in the cache.
            // Transfer it to the root's existing failure barrier before dropping
            // the projection, preserving an independently returned operation error.
            self.retain_failed_action_navigation(
                deferred.action_id,
                result
                    .as_ref()
                    .err()
                    .cloned()
                    .unwrap_or(RuntimeError::Closed),
            );
        }
        if live {
            self.frame_states.insert(key, frame);
        }
        let action_id = action_navigation
            .as_ref()
            .map(|barrier| barrier.action_id)
            .or_else(|| {
                action_root_navigation
                    .as_ref()
                    .map(|deferred| deferred.action_id)
            });
        if let Some(barrier) = action_navigation {
            self.adopt_action_navigation(barrier)?;
        }
        if let Err(error) = restoration {
            if let Some(action_id) = action_id {
                self.retain_failed_action_navigation(action_id, error.clone());
                return match result {
                    Err(original) => Err(original),
                    Ok(_) => Err(RuntimeError::ActionExecutionFailed {
                        action_id,
                        cause: Box::new(error),
                    }),
                };
            }
            return Err(error);
        }
        let mut navigation_update = None;
        if let Some(deferred) = action_root_navigation {
            if self.terminal_close_requested() {
                self.retain_failed_action_navigation(
                    deferred.action_id,
                    result
                        .as_ref()
                        .err()
                        .cloned()
                        .unwrap_or(RuntimeError::Closed),
                );
                return result.map(|value| (value, None));
            }
            #[cfg(unix)]
            let previous_ipc_identity = std::mem::replace(
                &mut self.navigation_progress_identity,
                deferred.ipc_identity,
            );
            let navigation = (|| {
                if self.frame_scope.is_some() {
                    return Err(RuntimeError::ProtocolViolation(
                        "scoped action returned to a non-root PageState".into(),
                    ));
                }
                let (location, history) = self
                    .executable
                    .as_mut()
                    .ok_or(RuntimeError::EvaluationUnavailable)?
                    .take_root_action_navigation()
                    .map_err(map_host_error)?;
                let generation = self.document_generation;
                let (progress, url) = if let Some(location) = location {
                    let update = if location.reload {
                        HistoryUpdate::Reload
                    } else if location.replace {
                        HistoryUpdate::Replace
                    } else {
                        HistoryUpdate::Push
                    };
                    let url = location.url.clone();
                    (
                        Some(self.begin_script_navigation(location, update)?),
                        Some(url),
                    )
                } else if let Some(history) = history {
                    (self.begin_action_history_navigation(history)?, None)
                } else {
                    (None, None)
                };
                if let Some(progress) = progress {
                    self.retain_action_navigation(
                        deferred.action_id,
                        generation,
                        deferred.deadline,
                        progress,
                    )?;
                    navigation_update = Some(ScopedActionNavigationUpdate {
                        action_id: deferred.action_id,
                        url,
                        root_replaced: self.document_generation != generation,
                    });
                }
                Ok(())
            })();
            #[cfg(unix)]
            {
                self.navigation_progress_identity = previous_ipc_identity;
            }
            if let Err(error) = navigation {
                self.retain_failed_action_navigation(deferred.action_id, error.clone());
                return match result {
                    Err(original) => Err(original),
                    Ok(_) => Err(RuntimeError::ActionExecutionFailed {
                        action_id: deferred.action_id,
                        cause: Box::new(error),
                    }),
                };
            }
        }
        let checkpoint = if native_scene {
            Ok(())
        } else {
            self.poll_page_tasks()
        };
        match result {
            Err(error) => Err(error),
            Ok(value) => {
                if let Err(error) = checkpoint {
                    if let Some(action_id) = action_id {
                        self.retain_failed_action_navigation(action_id, error.clone());
                        return Err(RuntimeError::ActionExecutionFailed {
                            action_id,
                            cause: Box::new(error),
                        });
                    }
                    return Err(error);
                }
                if read_only
                    && !self.executable.as_ref().is_some_and(|executable| {
                        executable
                            .automation_realms()
                            .iter()
                            .any(|entry| entry.same_document_as(&scope))
                    })
                {
                    return Err(RuntimeError::StaleSemanticRef);
                }
                Ok((value, navigation_update))
            }
        }
    }

    #[cfg(feature = "v8-host")]
    fn dispatch_scoped_worker(
        &mut self,
        request: crate::worker::WorkerRequest,
    ) -> Result<crate::worker::WorkerResponse, RuntimeError> {
        use crate::worker::{WorkerRequest as Q, WorkerResponse as R};
        Ok(match request {
            Q::WebdriverFrames { command } => R::WebdriverFrames {
                result: self.webdriver_frames(command),
            },
            Q::Observe { profile } => R::Observation {
                observation: self.observation(profile.unwrap_or(ProjectionProfile::Compact)),
            },
            Q::ObserveWith { options } => R::ObservationV4 {
                observation: self.observation_v4(options)?,
            },
            Q::ObserveSemantic { profile } => R::SemanticObservation {
                observation: self.semantic_observation(profile),
            },
            Q::ObserveSemanticV6 { options } => R::SemanticObservationV6 {
                observation: self.semantic_observation_v6(options)?,
            },
            Q::ResolveFocusedForm => R::SemanticRef {
                semantic_ref: self.resolve_focused_form()?,
            },
            Q::ResolveSelector { selector, action } => R::SemanticRef {
                semantic_ref: self.resolve_selector(&selector, action)?,
            },
            Q::ResolveTarget { target, action } => R::Resolution {
                result: self.resolve_target(&target, action)?,
            },
            Q::ResolveSemantic { target, action } => R::Resolution {
                result: self.resolve_semantic(&target, action)?,
            },
            Q::Extract { kind, selector } => R::Structured {
                value: self.extract(kind, selector.as_deref())?,
            },
            Q::ComputedStyle { target } => R::ComputedStyle {
                style: self.computed_style(target)?,
            },
            Q::AccessibilitySnapshot => R::AccessibilitySnapshot {
                snapshot: Box::new(self.accessibility_snapshot()?),
            },
            Q::AccessibilityText { target, command } => R::AccessibilityText {
                snapshot: self.accessibility_text(target, command)?,
            },
            Q::BoxModel { target } => R::BoxModel {
                box_model: self.box_model(target)?,
            },
            Q::SetViewport { viewport } => R::Viewport {
                viewport: self.set_viewport(viewport)?,
            },
            Q::SetMedia { media } => R::Media {
                media: self.set_media(media)?,
            },
            Q::Scroll { request } => R::Scroll {
                scroll: self.scroll(request)?,
            },
            Q::ScrollIntoView { target } => R::Scroll {
                scroll: self.scroll_into_view(target)?,
            },
            Q::ObserveDelta { from } => R::Delta {
                delta: self.delta_from(from)?,
            },
            Q::ObserveSemanticDelta { from } => R::SemanticDelta {
                delta: self.semantic_delta_from(from)?,
            },
            Q::Evaluate { source } => R::Value {
                value: self.evaluate(&source)?,
            },
            Q::Execute { action } => R::Action {
                result: self.dispatch_scoped_action(action)?,
            },
            Q::ExecuteBatch { .. } => {
                return Err(RuntimeError::ProtocolViolation(
                    "scoped batches require per-action actor admission".into(),
                ));
            }
            Q::SetFiles { target, files } => R::Action {
                result: self.set_files(target, files)?,
            },
            Q::WaitFor { predicate } => {
                self.wait_for(&predicate)?;
                R::Satisfied
            }
            Q::WaitForUntil {
                predicate,
                timeout_ms,
            } => {
                self.wait_for_timeout(&predicate, timeout_ms.map(Duration::from_millis))?;
                R::Satisfied
            }
            Q::Capabilities => R::Capabilities {
                capabilities: self.capabilities(),
            },
            Q::ChallengeAssessment => R::ChallengeAssessment {
                assessment: self.last_challenge.clone(),
            },
            Q::Trace => R::Trace {
                events: self.trace.snapshot(),
            },
            Q::PaintSnapshot => R::PaintSnapshot {
                snapshot: Box::new(complete_paint_snapshot(
                    self.prepare_capture_plan_until_ready(false, None)?,
                )?),
            },
            Q::PrintSnapshot { options } => {
                let (plan, pages) = self.print_snapshot_plan(&options)?;
                R::PrintSnapshot {
                    snapshot: Box::new(complete_print_snapshot(
                        complete_paint_snapshot(plan)?,
                        pages,
                        self.config.max_layout_nodes,
                    )?),
                }
            }
            Q::WebdriverScript {
                request,
                mut context,
            } => {
                context.selected = self.frame_scope.clone();
                R::WebdriverScript {
                    result: self.execute_webdriver_script(request, context, None),
                }
            }
            Q::WebdriverActions {
                command,
                mut context,
            } => {
                context.selected = self.frame_scope.clone();
                let result = self.execute_webdriver_actions(command, context, None);
                R::WebdriverActions {
                    result,
                    receipts: self.webdriver_input.take_os_receipts(),
                    feedback: self.take_drag_feedback(),
                }
            }
            _ => {
                return Err(RuntimeError::ProtocolViolation(
                    "command is not a selected-document operation".into(),
                ));
            }
        })
    }
    #[cfg(feature = "v8-host")]
    fn dispatch_scoped_action(
        &mut self,
        action: crate::worker::NativeActionRequest,
    ) -> Result<ActionResult, RuntimeError> {
        let native = NativeAction {
            target: action.target,
            action: action.action,
            value: action.value,
        };
        if action.sensitive {
            self.execute_sensitive(native)
        } else if action.force {
            self.execute_forced(native)
        } else {
            self.execute(native)
        }
    }
}

#[cfg(feature = "v8-host")]
impl PageState {
    fn capture_child_frame_paint(
        &mut self,
        plan: &mut PaintSnapshotPlan,
        print_media: bool,
    ) -> Result<(), RuntimeError> {
        self.capture_child_frame_paint_mode(plan, print_media, false)
    }
    fn capture_child_frame_presentation(
        &mut self,
        plan: &mut PaintSnapshotPlan,
    ) -> Result<(), RuntimeError> {
        self.capture_child_frame_paint_mode(plan, false, true)
    }
    fn capture_child_frame_paint_mode(
        &mut self,
        plan: &mut PaintSnapshotPlan,
        print_media: bool,
        native_scene: bool,
    ) -> Result<(), RuntimeError> {
        let Some(executable) = self.executable.as_ref() else {
            return Ok(());
        };
        let inventory = executable.automation_realms();
        let current = self
            .frame_scope
            .as_ref()
            .map(|scope| scope.realm)
            .or_else(|| {
                inventory
                    .iter()
                    .find(|entry| entry.live && entry.parent_realm.is_none())
                    .map(|entry| entry.realm)
            });
        let Some(current) = current else {
            return Ok(());
        };
        let mut ancestor = Some(current);
        let mut seen = BTreeSet::new();
        while let Some(realm) = ancestor {
            if !seen.insert(realm) || seen.len() > 32 {
                return Err(RuntimeError::PaintUnavailable(
                    "frame paint depth exceeds 32 or cyclic realm ancestry".into(),
                ));
            }
            ancestor = inventory
                .iter()
                .find(|entry| entry.realm == realm)
                .and_then(|entry| entry.parent_realm);
        }
        let children = inventory
            .into_iter()
            .filter(|entry| entry.live && entry.parent_realm == Some(current))
            .collect::<Vec<_>>();
        if children.len() > 256 {
            return Err(RuntimeError::PaintUnavailable(
                "frame paint exceeds 256 direct children".into(),
            ));
        }
        let mut count = 0usize;
        let mut nodes = plan.snapshot.nodes.len().saturating_add(
            plan.snapshot
                .svg_foreign_objects
                .iter()
                .map(|object| object.nodes.len())
                .sum::<usize>(),
        );
        let mut bytes = plan
            .snapshot
            .images
            .iter()
            .map(|image| image.bytes.len())
            .sum::<usize>();
        for scope in children {
            if native_scene
                && !self
                    .executable
                    .as_ref()
                    .is_some_and(|page| page.native_realm_rendering_allowed(scope.realm))
            {
                continue;
            }
            let Some(owner) = scope.frame.map(pack_node_id) else {
                continue;
            };
            let Some(node) = plan
                .snapshot
                .nodes
                .iter()
                .chain(
                    plan.snapshot
                        .svg_foreign_objects
                        .iter()
                        .flat_map(|object| object.nodes.iter()),
                )
                .find(|node| node.node_id == owner)
            else {
                continue;
            };
            let Some(model) = node
                .projection
                .as_ref()
                .map(|projection| &projection.box_model)
                .or(node.geometry.box_model.as_ref())
            else {
                continue;
            };
            if model.content_box.width <= 0.
                || model.content_box.height <= 0.
                || node.geometry.availability != oryn_common::v4::GeometryAvailability::Available
            {
                continue;
            }
            if !print_media
                && !node.geometry.in_viewport
                && !node.projection.as_ref().is_some_and(|projection| {
                    projection.paint_intersects_viewport(plan.snapshot.viewport)
                })
            {
                continue;
            }
            let viewport = ViewportState {
                width: model.content_box.width,
                height: model.content_box.height,
                device_pixel_ratio: plan.snapshot.viewport.device_pixel_ratio,
            };
            let result = self.scoped_worker_request_with_paint_mode(
                scope.clone(),
                crate::worker::WorkerRequest::PaintSnapshot,
                print_media,
                Some(viewport),
                native_scene,
            );
            // Nested capture must restore its actual parent, not always the top realm.
            let restored = self
                .executable
                .as_mut()
                .ok_or(RuntimeError::EvaluationUnavailable)?
                .set_automation_scope(self.frame_scope.clone())
                .map_err(map_host_error);
            restored?;
            let child = match result? {
                crate::worker::WorkerResponse::PaintSnapshot { snapshot } => snapshot,
                _ => {
                    return Err(RuntimeError::ProtocolViolation(
                        "frame capture returned wrong response".into(),
                    ));
                }
            };
            let live = self.executable.as_ref().is_some_and(|executable| {
                executable.automation_realms().iter().any(|entry| {
                    entry.live
                        && entry.realm == scope.realm
                        && entry.frame == scope.frame
                        && entry.frame_generation == scope.frame_generation
                        && entry.document_root == scope.document_root
                        && entry.document_generation == scope.document_generation
                        && entry.stream_epoch == scope.stream_epoch
                })
            });
            if !live {
                return Err(RuntimeError::PaintUnavailable(
                    "frame lifetime changed during paint capture".into(),
                ));
            }
            let mut pending = vec![child.as_ref()];
            while let Some(snapshot) = pending.pop() {
                count = count.saturating_add(1);
                nodes = nodes.saturating_add(snapshot.nodes.len()).saturating_add(
                    snapshot
                        .svg_foreign_objects
                        .iter()
                        .map(|object| object.nodes.len())
                        .sum::<usize>(),
                );
                bytes = bytes
                    .saturating_add(
                        snapshot
                            .images
                            .iter()
                            .map(|image| image.bytes.len())
                            .sum::<usize>(),
                    )
                    .saturating_add(
                        snapshot
                            .fonts
                            .values()
                            .map(|font| font.bytes.len())
                            .sum::<usize>(),
                    );
                if count > 256
                    || nodes > self.config.max_layout_nodes
                    || bytes
                        > self
                            .config
                            .max_response_bytes
                            .saturating_add(self.config.max_page_font_bytes)
                {
                    return Err(RuntimeError::PaintUnavailable(
                        "composed frame paint resource budget exceeded".into(),
                    ));
                }
                pending.extend(snapshot.frames.iter().map(|frame| frame.snapshot.as_ref()));
            }
            plan.snapshot
                .frames
                .push(oryn_common::v4::PaintFrameSnapshot {
                    owner_node: owner,
                    snapshot: child,
                });
        }
        Ok(())
    }
}
