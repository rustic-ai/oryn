// Included inside page_actor_turn::tests so the tests exercise the real actor
// mailbox, task pump, retained capture custody and destruction acknowledgement.
fn close_yield_control(root: &mut Root) -> Arc<oryn_js::prompts::PromptController> {
    let control = Arc::new(oryn_js::prompts::PromptController::new());
    assert!(root.state.page_task_terminal_control.bind(control.clone()));
    control
}

fn close_yield_script(root: &mut Root, source: &str) -> String {
    use crate::host::JavaScriptHost;
    let executable = root.state.executable.as_mut().unwrap();
    let mut host = executable.take_agent_host().unwrap();
    let result = host.evaluate_string(source).unwrap();
    assert!(matches!(executable.return_agent_host_after_turn(host), Ok(None)));
    result
}

fn close_yield_arm_frame(root: &mut Root) {
    close_yield_script(
        root,
        "globalThis.closeYieldFrames=0;requestAnimationFrame(()=>closeYieldFrames++);String(closeYieldFrames)",
    );
}

#[test]
fn terminal_close_after_drain_yields_without_consuming_capture_or_close_ack() {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let (mut root, sender) = timer_root(781);
    let control = close_yield_control(&mut root);
    // drain_tasks admits the first rendering step. Its newly scheduled callback
    // belongs to a later step and must remain pending when Close yields.
    close_yield_script(
        &mut root,
        "globalThis.closeYieldFirstFrame=0;globalThis.closeYieldNextFrame=0;requestAnimationFrame(()=>{closeYieldFirstFrame++;requestAnimationFrame(()=>closeYieldNextFrame++)});'armed'",
    );
    let before_layout = root.state.layout_revision;
    let (navigation_reply, mut navigation_received) = oneshot::channel();
    root.pending_navigations.push(PendingNavigationReply {
        joint_interactive_published: false,
        joint_eager_reply: false,
        joint_checkpoint: Some(crate::navigation_control::PendingNavigationAck::test_pending()),
        joint_publication: None,
        joint: None,
        generation: root.state.document_generation,
        load_epoch: None,
        identity: None,
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
        reply: NavigationReply::Controlled(navigation_reply),
    });
    let (capture, mut captured) = oneshot::channel();
    root.pending_captures.push_back(capture_readiness::Pending::new(
        &root.state,
        capture_readiness::Reply::Paint(capture),
    ));
    let (reply, mut closed) = oneshot::channel();
    let close_sender = sender.clone();
    root.state.after_page_task_drain = Some(Box::new(move || {
        control.retire();
        close_sender.try_send(PageCommand::Close { reply }).unwrap();
    }));
    assert!(root.dispatch(PageCommand::PollContextEvents, &context).is_none());
    assert!(root.state.after_page_task_drain.is_none(), "actual task drain reached");
    assert_eq!(timer_ticks(&mut root), "1", "already admitted work completed");
    assert_eq!(root.state.layout_revision, before_layout);
    assert_eq!(
        close_yield_script(&mut root, "String(closeYieldFirstFrame)+':'+String(closeYieldNextFrame)"),
        "1:0",
        "the admitted first callback ran; the next rendering step remains pending",
    );
    assert_eq!(root.pending_navigations.len(), 1);
    assert!(matches!(navigation_received.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
    assert_eq!(root.pending_captures.len(), 1);
    assert!(matches!(captured.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
    assert!(matches!(closed.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
    let command = root.next_command(&None).unwrap();
    assert!(matches!(&command, PageCommand::Close { .. }));
    let acknowledgement = root.dispatch(command, &context).unwrap();
    assert!(matches!(navigation_received.try_recv(), Ok(Err(RuntimeError::Closed))));
    assert!(matches!(closed.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
    drop(root);
    assert!(matches!(captured.try_recv(), Ok(Err(RuntimeError::Closed))));
    assert!(matches!(closed.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
    acknowledgement.send(()).unwrap();
    assert!(matches!(closed.try_recv(), Ok(())));
}

#[test]
fn live_page_control_keeps_normal_timer_and_animation_progress() {
    let (mut root, _sender) = timer_root(782);
    let control = close_yield_control(&mut root);
    close_yield_arm_frame(&mut root);
    root.state.poll_page_tasks().unwrap();
    root.state.refresh_layout().unwrap();
    assert_eq!(timer_ticks(&mut root), "1");
    assert_eq!(close_yield_script(&mut root, "String(closeYieldFrames)"), "1");
    assert!(control.inspect().is_ok());
    assert!(!root.state.terminal_close_requested());
}

#[test]
fn sibling_terminal_control_does_not_stop_this_pages_work() {
    let (mut root, _sender) = timer_root(783);
    let own = close_yield_control(&mut root);
    let sibling = Arc::new(oryn_js::prompts::PromptController::new());
    sibling.retire();
    root.state.poll_page_tasks().unwrap();
    assert_eq!(timer_ticks(&mut root), "1");
    assert!(own.inspect().is_ok());
    assert!(!root.state.terminal_close_requested());
}

#[test]
fn navigation_cancellation_does_not_retire_page_task_control() {
    use crate::navigation_control::{NavigationCommand, NavigationControl, NavigationRequest,
        NavigationStop, PageLoadStrategy};
    let (mut root, _sender) = timer_root(784);
    let control = close_yield_control(&mut root);
    let navigation = NavigationControl::default();
    let job = navigation.begin(NavigationRequest {
        command: NavigationCommand::Navigate { url: "https://cancel.test/".into() },
        strategy: PageLoadStrategy::Normal,
        timeout_ms: None,
    }).unwrap();
    job.stop(NavigationStop::Cancelled).unwrap();
    assert!(matches!(job.checkpoint(), Err(NavigationStop::Cancelled)));
    root.state.poll_page_tasks().unwrap();
    assert_eq!(timer_ticks(&mut root), "1");
    assert!(control.inspect().is_ok());
    assert!(!root.state.terminal_close_requested());
}

#[test]
fn terminal_presentation_preserves_pending_frame_without_entering_build() {
    let (mut root, _sender) = timer_root(785);
    let control = close_yield_control(&mut root);
    close_yield_arm_frame(&mut root);
    control.retire();
    let revision = root.state.layout_revision;
    root.state.publish_native_rendering_opportunity().unwrap();
    assert!(!root.state.presentation.building);
    assert_eq!(root.state.layout_revision, revision);
    assert_eq!(close_yield_script(&mut root, "String(closeYieldFrames)"), "0");
    // Retirement is only a yield signal, not destruction or successful Close.
    assert!(root.state.executable.is_some());
    assert!(root.state.terminal_close_requested());
}

#[test]
fn document_replacement_keeps_page_control_live() {
    let (mut root, _sender) = timer_root(786);
    let control = close_yield_control(&mut root);
    root.state
        .load_document(
            "https://scheduler.test/replacement".into(),
            "<body>replacement</body>",
            None,
            None,
            HistoryUpdate::Push,
            &BTreeMap::new(),
            Vec::new(),
        )
        .unwrap();
    assert!(Arc::ptr_eq(
        root.state.page_task_terminal_control.controller().unwrap(),
        &control
    ));
    assert!(!root.state.terminal_close_requested());
    close_yield_arm_frame(&mut root);
    root.state.refresh_layout().unwrap();
    assert_eq!(close_yield_script(&mut root, "String(closeYieldFrames)"), "1");
}

#[test]
fn scoped_frame_drivers_inherit_only_the_owning_page_terminal_control() {
    let (mut root, _sender) = root(787);
    let control = close_yield_control(&mut root);
    root.state.load_document(
        "https://scheduler.test/frames".into(),
        "<body><iframe srcdoc='<p>child</p>'></iframe></body>",
        None, None, HistoryUpdate::Push, &BTreeMap::new(), Vec::new(),
    ).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while root.state.frame_states.is_empty() {
        root.state.poll_page_tasks().unwrap();
        assert!(Instant::now() < deadline, "scoped child driver was not created");
    }
    for frame in root.state.frame_states.values() {
        assert!(Arc::ptr_eq(
            frame.page_task_terminal_control.controller().unwrap(),
            &control
        ));
        assert!(!frame.terminal_close_requested());
    }
    control.retire();
    assert!(root.state.terminal_close_requested());
    assert!(root.state.frame_states.values().all(|frame| frame.terminal_close_requested()));
    assert!(root.state.executable.is_some(), "scoped driver restored the root executable");
}

#[tokio::test(flavor = "current_thread")]
async fn actor_binds_exact_registered_page_control_and_does_not_replace_it() {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    let sibling = context.new_page();
    let expected = context.page_prompt_controller(page.page_id()).unwrap();
    let sibling_control = context.page_prompt_controller(sibling.page_id()).unwrap();
    let (mut root, _sender) = root(page.id());
    let (reply, _result) = oneshot::channel();
    root.dispatch(PageCommand::Capabilities { reply }, &context);
    assert!(Arc::ptr_eq(
        root.state.page_task_terminal_control.controller().unwrap(),
        &expected
    ));
    sibling_control.retire();
    assert!(!root.state.terminal_close_requested());
    expected.retire();
    let (reply, _result) = oneshot::channel();
    root.dispatch(PageCommand::Capabilities { reply }, &context);
    assert!(Arc::ptr_eq(
        root.state.page_task_terminal_control.controller().unwrap(),
        &expected
    ));
    assert!(root.state.terminal_close_requested());
    drop(root);
    page.close().await.unwrap();
    sibling.close().await.unwrap();
}

#[test]
fn failed_navigation_checkpoint_remains_an_error_before_task_drain() {
    use crate::navigation_control::{NavigationCommand, NavigationControl, NavigationEmitter,
        NavigationRequest, NavigationStop, PageLoadStrategy};
    let (mut root, _sender) = timer_root(788);
    let control = close_yield_control(&mut root);
    let navigation = NavigationControl::default();
    let job = navigation.begin(NavigationRequest {
        command: NavigationCommand::Navigate { url: "https://failed.test/".into() },
        strategy: PageLoadStrategy::Normal,
        timeout_ms: None,
    }).unwrap();
    job.stop(NavigationStop::Failed("preserved-before-yield".into())).unwrap();
    root.state.active_navigation = Some(NavigationEmitter::Local(job));
    let retire = control.clone();
    root.state.after_page_task_drain = Some(Box::new(move || retire.retire()));
    let error = root.state.poll_page_tasks().unwrap_err();
    assert!(error.to_string().contains("preserved-before-yield"), "{error}");
    assert!(root.state.after_page_task_drain.is_some());
    assert!(control.inspect().is_ok());
}

// Admit Close at a real instrumented operation boundary while the page actor is
// synchronously pumping work. The hook is test-only; the mailbox, control,
// callbacks, checkpoint and retained replies all use their production paths.
fn close_inside_page_task_operation(
    phase: &'static str,
    checkpoint_expected: bool,
    next_phase: &'static str,
) {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let (mut root, sender) = timer_root(791);
    let control = close_yield_control(&mut root);
    close_yield_script(
        &mut root,
        "globalThis.innerCloseOrder=[];requestAnimationFrame(()=>{innerCloseOrder.push('callback');Promise.resolve().then(()=>innerCloseOrder.push('microtask'));requestAnimationFrame(()=>innerCloseOrder.push('next-frame'))});'armed'",
    );
    let before_layout = root.state.layout_revision;
    let (navigation_reply, mut navigation_received) = oneshot::channel();
    root.pending_navigations.push(PendingNavigationReply {
        joint_interactive_published: false,
        joint_eager_reply: false,
        joint_checkpoint: Some(crate::navigation_control::PendingNavigationAck::test_pending()),
        joint_publication: None,
        joint: None,
        generation: root.state.document_generation,
        load_epoch: None,
        identity: None,
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
        reply: NavigationReply::Controlled(navigation_reply),
    });
    let (capture, mut captured) = oneshot::channel();
    root.pending_captures
        .push_back(capture_readiness::Pending::new(
            &root.state,
            capture_readiness::Reply::Paint(capture),
        ));
    let (reply, mut closed) = oneshot::channel();
    let mut reply = Some(reply);
    let entered = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = entered.clone();
    let (dispatch, labels) =
        crate::ipc_progress::close_admission::capture_test_phase_labels_with_hook(
            || root.dispatch(PageCommand::PollContextEvents, &context),
            move |label| {
                if label == phase && !observed.replace(true) {
                    control.retire();
                    sender
                        .try_send(PageCommand::Close {
                            reply: reply.take().unwrap(),
                        })
                        .unwrap();
                }
            },
        );
    assert!(entered.get(), "real operation reached: {phase}");
    assert!(dispatch.is_none());
    assert_eq!(root.state.layout_revision, before_layout);
    assert_eq!(timer_ticks(&mut root), "1");
    assert_eq!(
        close_yield_script(&mut root, "innerCloseOrder.join(',')"),
        "callback,microtask",
        "the admitted frame and microtask transaction completes in order",
    );
    assert_eq!(
        labels.contains(&"executable_checkpoint"),
        checkpoint_expected
    );
    assert!(
        !labels.contains(&next_phase),
        "no later whole checkpoint operation"
    );
    assert!(
        !labels.contains(&"checkpoint_document_snapshot"),
        "no late snapshot publication"
    );
    assert_eq!(
        labels
            .iter()
            .filter(|label| **label == "animation_publication")
            .count(),
        1,
        "only the admitted tree's internal publication, not post-checkpoint publication"
    );
    if checkpoint_expected {
        assert_eq!(
            labels.last(),
            Some(&phase),
            "yield after the admitted operation"
        );
    }
    assert_eq!(root.pending_navigations.len(), 1);
    assert_eq!(root.pending_captures.len(), 1);
    assert!(matches!(
        navigation_received.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(matches!(
        captured.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(matches!(
        closed.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    let command = root.next_command(&None).unwrap();
    assert!(matches!(&command, PageCommand::Close { .. }));
    let acknowledgement = root.dispatch(command, &context).unwrap();
    assert!(matches!(
        navigation_received.try_recv(),
        Ok(Err(RuntimeError::Closed))
    ));
    assert!(matches!(
        closed.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    drop(root);
    assert!(matches!(captured.try_recv(), Ok(Err(RuntimeError::Closed))));
    assert!(matches!(
        closed.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    acknowledgement.send(()).unwrap();
    assert!(matches!(closed.try_recv(), Ok(())));
}

#[test]
fn terminal_close_inside_checkpoint_yields_before_next_whole_operation() {
    close_inside_page_task_operation(
        "checkpoint_dynamic_frames",
        true,
        "checkpoint_dynamic_stylesheets",
    );
}

#[test]
fn terminal_close_inside_admitted_frame_preserves_callback_microtask_transaction() {
    close_inside_page_task_operation(
        "animation_document_callbacks",
        false,
        "executable_checkpoint",
    );
}

#[test]
fn unscoped_executable_drain_keeps_callback_and_checkpoint_behavior() {
    let mut page =
        crate::executable::RustExecutableDocument::load("<body>standalone</body>").unwrap();
    let unrelated = Arc::new(oryn_js::prompts::PromptController::new());
    unrelated.retire();
    page.execute("globalThis.standaloneOrder=[];requestAnimationFrame(()=>{standaloneOrder.push('callback');Promise.resolve().then(()=>standaloneOrder.push('microtask'))})").unwrap();
    let (result, labels) =
        crate::ipc_progress::close_admission::capture_test_phase_labels(|| page.drain_tasks());
    result.unwrap();
    assert_eq!(
        page.evaluate("standaloneOrder.join(',')").unwrap(),
        "callback,microtask"
    );
    assert!(labels.contains(&"checkpoint_dynamic_stylesheets"));
    assert!(labels.contains(&"checkpoint_document_snapshot"));
    assert_eq!(
        labels
            .iter()
            .filter(|label| **label == "animation_publication")
            .count(),
        2
    );
}

#[test]
fn terminal_close_at_formatted_checkpoint_boundaries_skips_following_operations() {
    for (phase, next_phase) in [
        (
            "checkpoint_closed_document_streams",
            "checkpoint_async_fetches_after",
        ),
        (
            "checkpoint_custom_element_reactions",
            "checkpoint_dom_observers",
        ),
    ] {
        close_inside_page_task_operation(phase, true, next_phase);
    }
}
