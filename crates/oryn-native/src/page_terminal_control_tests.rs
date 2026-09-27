// Included alongside the real actor tests. Phase hooks observe actual production
// boundaries; no sleeping, fabricated completion, or replacement task loop.
#[test]
fn page_owner_layout_inherits_terminal_control_and_keeps_admitted_microtasks() {
    let (mut root, _sender) = timer_root(810);
    let control = close_yield_control(&mut root);
    close_yield_script(
        &mut root,
        "globalThis.layoutOrder=[];requestAnimationFrame(()=>{layoutOrder.push('frame');Promise.resolve().then(()=>layoutOrder.push('microtask'));requestAnimationFrame(()=>layoutOrder.push('later'))});'armed'",
    );
    let before = root.state.layout_revision;
    let hit = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = hit.clone();
    let (result, labels) =
        crate::ipc_progress::close_admission::capture_test_phase_labels_with_hook(
            || root.state.refresh_page_task_layout(),
            move |label| {
                if label == "checkpoint_dynamic_frames" && !seen.replace(true) {
                    control.retire();
                }
            },
        );
    assert!(!result.unwrap());
    assert!(hit.get());
    assert_eq!(root.state.layout_revision, before);
    assert!(!labels.contains(&"checkpoint_dynamic_stylesheets"));
    assert_eq!(
        labels
            .iter()
            .filter(|label| **label == "animation_publication")
            .count(),
        1
    );
    assert_eq!(
        close_yield_script(&mut root, "layoutOrder.join(',')"),
        "frame,microtask"
    );
    assert!(root.state.executable.is_some());
}

#[test]
fn page_owner_constructor_yield_retains_document_without_false_readiness() {
    let (mut root, _sender) = root(811);
    let control = close_yield_control(&mut root);
    let hit = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = hit.clone();
    let (result, labels) =
        crate::ipc_progress::close_admission::capture_test_phase_labels_with_hook(
            || {
                root.state.load_document(
                    "https://owner.test/construction".into(),
                    "<body>retained construction document",
                    None,
                    None,
                    HistoryUpdate::Push,
                    &BTreeMap::new(),
                    Vec::new(),
                )
            },
            move |label| {
                if label == "checkpoint_dynamic_frames" && !seen.replace(true) {
                    control.retire();
                }
            },
        );
    result.unwrap();
    assert!(
        hit.get(),
        "construction reached the actual lifecycle checkpoint"
    );
    assert!(root.state.executable.is_some());
    assert!(root.state.history_index.is_some());
    assert!(root.state.terminal_close_requested());
    assert!(!labels.contains(&"checkpoint_dynamic_stylesheets"));
    assert!(!root.state.lifecycle.contains(&LifecycleState::Load));
    assert!(
        !root
            .state
            .trace
            .snapshot()
            .iter()
            .any(|entry| matches!(entry.kind, TraceEventKind::NavigationCommitted { .. }))
    );
}

#[test]
fn page_owner_nested_parser_checkpoint_inherits_control_before_installation() {
    struct Chunks(std::collections::VecDeque<String>);
    impl crate::host::DocumentInput for Chunks {
        fn poll_next(&mut self) -> std::task::Poll<Option<Result<String, crate::host::HostError>>> {
            std::task::Poll::Ready(self.0.pop_front().map(Ok))
        }
    }
    let owner = crate::page_terminal_control::PageTerminalControl::new(PageId(812));
    let control = Arc::new(oryn_js::prompts::PromptController::new());
    assert!(owner.bind(control.clone()));
    let stream = crate::host::DocumentStream {
        content_language: None,
        chunks: Box::new(Chunks(
            vec!["<body><script>globalThis.parserRan=1</script>done".into()].into(),
        )),
        content_type: Some("text/html".into()),
        encoding: Default::default(),
    };
    let mut page = crate::executable::RustExecutableDocument::load_for_page(
        crate::executable::PageDocumentLoadInput {
            parsed: None,
            source: Some(stream),
            html: "",
            resources: &BTreeMap::new(),
            broker: None,
            base_url: Some(Url::parse("https://owner.test/parser").unwrap()),
            max_heap_bytes: BrowserConfig::default().max_v8_heap_bytes,
            scheduled: true,
            #[cfg(unix)]
            context: None,
            recovery: None,
            inherited_origin: None,
            terminal_control: owner,
        },
    )
    .unwrap();
    let lifecycle = page.parser_lifecycle_probe_for_test();
    assert!(!lifecycle.get());
    assert!(
        page.parser_pending(),
        "scheduled construction retained actual parser work"
    );
    let hit = std::rc::Rc::new(std::cell::Cell::new(false));
    for _ in 0..64 {
        let active_lifecycle = lifecycle.clone();
        let seen = hit.clone();
        let retire = control.clone();
        let (result, labels) =
            crate::ipc_progress::close_admission::capture_test_phase_labels_with_hook(
                || page.poll_context_events(),
                move |label| {
                    // The retained HTML owner can enter lifecycle from the
                    // earlier frame-preparation checkpoint, before parser_resume.
                    // Require the exact executable's active lifecycle call,
                    // not a label remembered after an unrelated operation.
                    if active_lifecycle.get()
                        && label == "checkpoint_dynamic_frames"
                        && !seen.replace(true)
                    {
                        retire.retire();
                    }
                },
            );
        result.unwrap();
        if hit.get() {
            assert_eq!(labels.last(), Some(&"checkpoint_dynamic_frames"));
            break;
        }
    }
    assert!(
        hit.get(),
        "real parser lifecycle reentered the plain checkpoint"
    );
    assert!(
        !lifecycle.get(),
        "actual lifecycle scope restored after the terminal yield"
    );
    assert!(page.terminal_retired());
    assert!(matches!(
        page.layout_snapshot(),
        Err(crate::host::HostError::ScriptLoad(
            crate::script_load_failure::ScriptLoadFailure::Cancelled
        ))
    ));
    let (result, labels) =
        crate::ipc_progress::close_admission::capture_test_phase_labels(|| page.drain_tasks());
    result.unwrap();
    assert!(!labels.contains(&"timers_and_control_tasks"));
    let live_owner = crate::page_terminal_control::PageTerminalControl::new(PageId(1812));
    assert!(live_owner.bind(Arc::new(oryn_js::prompts::PromptController::new())));
    let mut live = crate::executable::RustExecutableDocument::load_for_page(
        crate::executable::PageDocumentLoadInput {
            parsed: None,
            source: Some(crate::host::DocumentStream::html(Box::new(
                std::iter::once(Ok(
                    "<body><script>globalThis.liveParser=1</script>complete".into()
                )),
            ))),
            html: "",
            resources: &BTreeMap::new(),
            broker: None,
            base_url: Some(Url::parse("https://owner.test/live-parser").unwrap()),
            max_heap_bytes: BrowserConfig::default().max_v8_heap_bytes,
            scheduled: false,
            #[cfg(unix)]
            context: None,
            recovery: None,
            inherited_origin: None,
            terminal_control: live_owner,
        },
    )
    .unwrap();
    assert!(!live.parser_lifecycle_probe_for_test().get());
    assert!(
        !std::rc::Rc::ptr_eq(&lifecycle, &live.parser_lifecycle_probe_for_test()),
        "a sibling executable has a distinct observation slot"
    );
    assert!(live.document_load_complete());
    assert_eq!(live.evaluate("String(liveParser)").unwrap(), "1");
}

#[test]
fn page_owner_navigation_publication_yield_keeps_pending_transaction() {
    let (mut root, _sender) = timer_root(813);
    let control = close_yield_control(&mut root);
    assert!(matches!(
        root.state
            .begin_navigation_resource(
                root.state.url.clone(),
                HistoryUpdate::Reload,
                None,
                None,
                None,
                None
            )
            .unwrap(),
        NavigationProgress::Pending(_)
    ));
    let mut transaction = root.state.pending_navigation_transaction.take().unwrap();
    let executable = root.state.executable.as_ref().unwrap();
    transaction.phase = NavigationTransactionPhase::Publication {
        restored: false,
        identity: executable.form_history_root_identity(),
        epoch: executable.native_navigation_load_epoch(),
        retry: None,
        milestone: 0,
        result: NavigationResult {
            page_id: root.state.page_id,
            requested_url: root.state.url.clone(),
            final_url: root.state.url.clone(),
            title: String::new(),
            status: 200,
            document_generation: root.state.document_generation,
            revision: Revision(root.state.revision),
            lifecycle: vec![LifecycleState::Commit],
            challenge: root.state.last_challenge.clone(),
        },
    };
    let (reply, mut received) = oneshot::channel();
    transaction.reply = Some(NavigationReply::Controlled(reply));
    let before = root.state.layout_revision;
    let hit = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = hit.clone();
    root.state.pending_navigation_transaction = Some(transaction);
    let (result, labels) =
        crate::ipc_progress::close_admission::capture_test_phase_labels_with_hook(
            || root.state.poll_page_tasks_inner(),
            move |label| {
                if label == "checkpoint_dynamic_frames" && !seen.replace(true) {
                    control.retire();
                }
            },
        );
    assert!(hit.get());
    result.unwrap();
    assert!(
        !labels.contains(&"executable_drain"),
        "no new task drain after the early publication yielded"
    );
    assert_eq!(root.state.layout_revision, before);
    let transaction = root.state.pending_navigation_transaction.as_ref().unwrap();
    assert!(matches!(
        transaction.phase,
        NavigationTransactionPhase::Publication { milestone: 0, .. }
    ));
    assert!(matches!(
        received.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(root.state.executable.is_some());
}

#[test]
fn page_owner_cached_document_and_host_restoration_keep_exact_slot() {
    use crate::host::JavaScriptHost;
    let (mut root, _sender) = timer_root(814);
    let control = close_yield_control(&mut root);
    let owner = root.state.page_task_terminal_control.clone();
    root.state
        .load_document(
            "https://owner.test/second".into(),
            "<body>second",
            None,
            None,
            HistoryUpdate::Push,
            &BTreeMap::new(),
            Vec::new(),
        )
        .unwrap();
    assert!(
        root.state
            .restore_cached_document(0, "https://scheduler.test/".into())
            .unwrap()
            .is_some()
    );
    assert!(Arc::ptr_eq(&owner, &root.state.page_task_terminal_control));
    let executable = root.state.executable.as_mut().unwrap();
    let mut host = executable.take_agent_host().unwrap();
    assert!(
        host.execute("throw new Error('retained-host-error')")
            .is_err()
    );
    assert!(matches!(
        executable.return_agent_host_after_turn(host),
        Ok(None)
    ));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::ipc_progress::close_admission::capture_test_phase_labels_with_hook(
            || executable.drain_tasks(),
            |label| {
                if label == "checkpoint_dynamic_frames" {
                    panic!("injected checkpoint unwind");
                }
            },
        )
    }));
    assert!(outcome.is_err());
    control.retire();
    assert!(executable.terminal_retired());
    let (result, labels) = crate::ipc_progress::close_admission::capture_test_phase_labels(|| {
        executable.drain_tasks()
    });
    result.unwrap();
    assert!(!labels.contains(&"timers_and_control_tasks"));
}

#[test]
fn page_owner_frame_error_restores_executable_scope_and_owner() {
    let (mut root, _sender) = root(815);
    let control = close_yield_control(&mut root);
    root.state
        .load_document(
            "https://owner.test/frame".into(),
            "<body><iframe srcdoc='<p>child</p>'></iframe>",
            None,
            None,
            HistoryUpdate::Push,
            &BTreeMap::new(),
            Vec::new(),
        )
        .unwrap();
    let scope = root
        .state
        .executable
        .as_ref()
        .unwrap()
        .automation_realms()
        .into_iter()
        .find(|entry| entry.frame.is_some())
        .expect("real child realm");
    let previous = root
        .state
        .executable
        .as_ref()
        .unwrap()
        .current_automation_scope();
    let own = control.clone();
    let error = root
        .state
        .with_scoped_frame(
            scope.clone(),
            false,
            None,
            true,
            true,
            move |frame| -> Result<(), RuntimeError> {
                assert!(Arc::ptr_eq(
                    frame.page_task_terminal_control.controller().unwrap(),
                    &own
                ));
                Err(RuntimeError::Script("retained-frame-error".into()))
            },
        )
        .unwrap_err();
    assert!(error.to_string().contains("retained-frame-error"));
    let executable = root.state.executable.as_ref().unwrap();
    assert_eq!(
        executable
            .current_automation_scope()
            .map(|entry| entry.realm),
        previous.map(|entry| entry.realm)
    );
    control.retire();
    assert!(executable.terminal_retired());
    let called = std::cell::Cell::new(false);
    assert!(matches!(
        root.state
            .with_scoped_frame(scope, false, None, true, true, |_| {
                called.set(true);
                Ok(())
            }),
        Err(RuntimeError::PageTaskYield)
    ));
    assert!(!called.get());
    assert!(root.state.executable.is_some());
}

#[test]
fn page_owner_slot_rejects_foreign_binding_and_blank_root_isolation() {
    let owner = crate::page_terminal_control::PageTerminalControl::new(PageId(816));
    assert!(!owner.retired());
    let own = Arc::new(oryn_js::prompts::PromptController::new());
    let foreign = Arc::new(oryn_js::prompts::PromptController::new());
    assert!(owner.bind(own.clone()));
    assert!(!owner.bind(foreign.clone()));
    assert!(Arc::ptr_eq(owner.controller().unwrap(), &own));
    let same = owner.for_page(PageId(816));
    let blank = owner.for_page(PageId(817));
    assert!(Arc::ptr_eq(&owner, &same));
    assert!(!Arc::ptr_eq(&owner, &blank));
    assert!(blank.bind(foreign.clone()));
    own.retire();
    assert!(same.retired());
    assert!(
        !blank.retired(),
        "accepted blank root does not inherit source retirement"
    );
    foreign.retire();
    assert!(blank.retired());
}

#[test]
fn page_task_yield_json_remains_closed_and_remote_input_cannot_create_yield() {
    let yielded = RuntimeError::PageTaskYield;
    let closed = RuntimeError::Closed;
    assert_ne!(yielded, closed, "local capture distinguishes provenance");
    assert_eq!(yielded.to_string(), closed.to_string());
    let wire = serde_json::to_string(&yielded).unwrap();
    assert_eq!(wire, serde_json::to_string(&closed).unwrap());
    assert_eq!(serde_json::from_str::<RuntimeError>(&wire).unwrap(), closed);
    assert!(serde_json::from_str::<RuntimeError>("\"PageTaskYield\"").is_err());
    let wrapped = RuntimeError::NavigationCleanupFailed {
        cause: Box::new(RuntimeError::PageTaskYield),
        cleanup: Box::new(RuntimeError::Script("real-cleanup-failure".into())),
    };
    let encoded = serde_json::to_string(&wrapped).unwrap();
    let decoded = serde_json::from_str::<RuntimeError>(&encoded).unwrap();
    assert!(
        matches!(decoded, RuntimeError::NavigationCleanupFailed { cause, cleanup }
        if *cause == RuntimeError::Closed && *cleanup == RuntimeError::Script("real-cleanup-failure".into()))
    );
}

#[test]
fn frame_terminal_yield_retains_deferred_action_without_new_navigation() {
    let (mut root, _sender) = root(818);
    let control = close_yield_control(&mut root);
    root.state
        .load_document(
            "https://owner.test/deferred-frame".into(),
            "<body><iframe srcdoc='<p>child</p>'></iframe>",
            None,
            None,
            HistoryUpdate::Push,
            &BTreeMap::new(),
            Vec::new(),
        )
        .unwrap();
    let scope = root
        .state
        .executable
        .as_ref()
        .unwrap()
        .automation_realms()
        .into_iter()
        .find(|entry| entry.frame.is_some())
        .unwrap();
    root.state
        .with_scoped_frame(scope, false, None, true, false, move |frame| {
            frame.action_root_navigation = Some(action_navigation::ScopedAction {
                action_id: ActionId(818),
                deadline: None,
                #[cfg(unix)]
                ipc_identity: None,
            });
            control.retire();
            Ok(())
        })
        .unwrap();
    assert!(root.state.executable.is_some());
    assert!(root.state.pending_navigation_transaction.is_none());
    assert!(root.state.frame_states.values().any(|frame| {
        frame
            .action_root_navigation
            .as_ref()
            .is_some_and(|pending| pending.action_id == ActionId(818))
    }));
}

#[tokio::test(flavor = "current_thread")]
async fn page_owner_missing_source_registration_retries_exact_destination_context() {
    let source = Browser::in_process_with_config(Default::default()).new_context();
    let destination = Browser::in_process_with_config(Default::default()).new_context();
    let page_id = PageId(819);
    let (mut root, _sender) = root(page_id.0);
    let slot = root.state.page_task_terminal_control.clone();
    let (reply, _received) = oneshot::channel();
    root.dispatch(PageCommand::Capabilities { reply }, &source);
    assert!(
        slot.controller().is_none(),
        "source context has no destination record"
    );
    assert!(
        !slot.retired(),
        "missing binding is not successful retirement"
    );
    let page = destination.new_page_with_identity(page_id, None);
    let exact = destination.page_prompt_controller(page_id).unwrap();
    let (reply, _received) = oneshot::channel();
    root.dispatch(PageCommand::Capabilities { reply }, &destination);
    assert!(Arc::ptr_eq(&slot, &root.state.page_task_terminal_control));
    assert!(Arc::ptr_eq(slot.controller().unwrap(), &exact));
    exact.retire();
    assert!(root.state.terminal_close_requested());
    drop(root);
    page.close().await.unwrap();
}

#[test]
fn page_owner_early_reply_handoff_retains_later_replies_and_skips_task_drain() {
    for retire in [false, true] {
        let (mut root, _sender) = timer_root(if retire { 820 } else { 821 });
        let control = close_yield_control(&mut root);
        let mut receivers = Vec::new();
        for _ in 0..2 {
            let (reply, received) = oneshot::channel();
            receivers.push(received);
            let page = root.state.executable.as_ref().unwrap();
            root.state
                .pending_execution_replies
                .push(PendingNavigationReply {
                    joint_interactive_published: false,
                    joint_eager_reply: false,
                    joint_checkpoint: None,
                    joint_publication: None,
                    joint: None,
                    generation: root.state.document_generation,
                    load_epoch: page.native_navigation_load_epoch(),
                    identity: page.form_history_root_identity(),
                    deadline: Instant::now() + Duration::from_secs(30),
                    result: NavigationResult {
                        page_id: root.state.page_id,
                        requested_url: root.state.url.clone(),
                        final_url: root.state.url.clone(),
                        title: String::new(),
                        status: 200,
                        document_generation: root.state.document_generation,
                        revision: Revision(root.state.revision),
                        lifecycle: root.state.lifecycle.clone(),
                        challenge: root.state.last_challenge.clone(),
                    },
                    reply: NavigationReply::Controlled(reply),
                });
        }
        assert!(root.state.lifecycle.contains(&LifecycleState::Load));
        let (result, labels) =
            crate::ipc_progress::close_admission::capture_test_phase_labels_with_hook(
                || root.state.poll_page_tasks_inner(),
                move |label| {
                    if retire && label == "page_poll_execution_reply" {
                        control.retire();
                    }
                },
            );
        result.unwrap();
        // The first operation was already admitted and completes its real result.
        assert!(matches!(receivers[0].try_recv(), Ok(Ok(_))));
        if retire {
            assert_eq!(
                labels
                    .iter()
                    .filter(|label| **label == "page_poll_execution_reply")
                    .count(),
                1
            );
            assert!(!labels.contains(&"executable_drain"));
            assert_eq!(root.state.pending_execution_replies.len(), 1);
            assert!(matches!(
                receivers[1].try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ));
        } else {
            assert!(labels.contains(&"executable_drain"));
            assert!(root.state.pending_execution_replies.is_empty());
            assert!(matches!(receivers[1].try_recv(), Ok(Ok(_))));
        }
    }
}

#[test]
fn invalidated_frame_terminal_handoff_retains_action_failure_and_original_error() {
    for fail in [false, true] {
        let (mut root, _sender) = root(if fail { 822 } else { 823 });
        let control = close_yield_control(&mut root);
        root.state
            .load_document(
                "https://owner.test/invalidated-frame".into(),
                "<body><iframe srcdoc='<p>child</p>'></iframe>",
                None,
                None,
                HistoryUpdate::Push,
                &BTreeMap::new(),
                Vec::new(),
            )
            .unwrap();
        let scope = root
            .state
            .executable
            .as_ref()
            .unwrap()
            .automation_realms()
            .into_iter()
            .find(|entry| entry.frame.is_some())
            .unwrap();
        let stale_scope = scope.clone();
        let action_id = ActionId(822);
        let original = RuntimeError::Script("frame operation failed after invalidation".into());
        let failure = original.clone();
        let result = root
            .state
            .with_scoped_frame(scope, false, None, true, false, move |frame| {
                frame.action_root_navigation = Some(action_navigation::ScopedAction {
                    action_id,
                    deadline: None,
                    #[cfg(unix)]
                    ipc_identity: None,
                });
                let page = frame.executable.as_mut().unwrap();
                // Real DOM removal and its native lifecycle checkpoint invalidate
                // the selected frame before the loan is restored to the root.
                page.set_automation_scope(None).unwrap();
                assert_eq!(
                    page.evaluate("document.querySelector('iframe').remove();'removed'")
                        .unwrap(),
                    "removed"
                );
                assert!(
                    !page
                        .automation_realms()
                        .iter()
                        .any(|entry| entry.same_document_as(&stale_scope))
                );
                control.retire();
                if fail { Err(failure) } else { Ok(()) }
            });
        if fail {
            assert_eq!(result.unwrap_err(), original);
        } else {
            result.unwrap();
        }
        assert!(root.state.frame_states.is_empty());
        assert!(root.state.executable.is_some());
        assert!(root.state.pending_navigation_transaction.is_none());
        let expected = if fail { original } else { RuntimeError::Closed };
        assert!(matches!(
            root.state.poll_action_navigation(),
            std::task::Poll::Ready(Err(RuntimeError::ActionExecutionFailed { action_id: id, cause }))
                if id == action_id && *cause == expected
        ));
        assert!(
            root.state.action_navigation.is_some(),
            "failure remains owned until settlement/destruction"
        );
    }
}

#[test]
fn terminal_postdrain_handoff_retains_extracted_navigation_and_diagnostic_batch() {
    let (mut root, _sender) = timer_root(824);
    let control = close_yield_control(&mut root);
    close_yield_script(
        &mut root,
        "console.log('retained-terminal-signal');location.href='https://scheduler.test/held';'queued'",
    );
    let hit = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = hit.clone();
    let (result, labels) =
        crate::ipc_progress::close_admission::capture_test_phase_labels_with_hook(
            || root.state.poll_page_tasks_inner(),
            move |label| {
                if label == "semantic_projection" && !seen.replace(true) {
                    control.retire();
                }
            },
        );
    result.unwrap();
    assert!(hit.get());
    let retained = root
        .state
        .terminal_page_task_signals
        .as_ref()
        .expect("extracted batch remains page-owned");
    assert_eq!(
        retained.location.as_ref().unwrap().0.url,
        "https://scheduler.test/held"
    );
    assert!(
        retained
            .console_entries
            .iter()
            .any(|entry| entry.message.contains("retained-terminal-signal"))
    );
    assert!(root.state.pending_navigation_transaction.is_none());
    assert!(!labels.contains(&"navigation_response_completion"));
    assert_eq!(
        labels
            .iter()
            .filter(|label| **label == "semantic_projection")
            .count(),
        1
    );
    let (result, labels) = crate::ipc_progress::close_admission::capture_test_phase_labels(|| {
        root.state.poll_page_tasks_inner()
    });
    result.unwrap();
    assert!(!labels.contains(&"executable_drain"));
    assert!(
        root.state
            .terminal_page_task_signals
            .as_ref()
            .unwrap()
            .location
            .is_some()
    );
}

#[test]
fn terminal_history_publication_keeps_next_stage_and_later_job_after_completed_callback() {
    for retire in [false, true] {
        let (mut root, _sender) = timer_root(if retire { 825 } else { 826 });
        let control = close_yield_control(&mut root);
        close_yield_script(
            &mut root,
            "globalThis.historyOrder=[];addEventListener('pageshow',()=>{historyOrder.push('pageshow');Promise.resolve().then(()=>historyOrder.push('microtask'))});'armed'",
        );
        root.state.queue_rollback_pageshow(false).unwrap();
        root.state.queue_rollback_pageshow(false).unwrap();
        let (result, labels) =
            crate::ipc_progress::close_admission::capture_test_phase_labels_with_hook(
                || root.state.poll_page_tasks_inner(),
                move |label| {
                    if retire && label == "history_publication_stage" {
                        control.retire();
                    }
                },
            );
        result.unwrap();
        if retire {
            assert!(!labels.contains(&"executable_drain"));
            assert_eq!(root.state.history_publications.len(), 2);
            assert_eq!(
                root.state.history_publications[0].stage, 4,
                "completed pageshow advances exactly its actual stage"
            );
            assert_eq!(
                root.state.history_publications[1].stage, 6,
                "later job was not admitted"
            );
            assert_eq!(
                close_yield_script(&mut root, "historyOrder.join(',')"),
                "pageshow,microtask"
            );
        } else {
            assert!(root.state.history_publications.is_empty());
            assert!(labels.contains(&"executable_drain"));
            assert_eq!(
                close_yield_script(&mut root, "historyOrder.join(',')"),
                "pageshow,microtask,pageshow,microtask"
            );
        }
    }
}
