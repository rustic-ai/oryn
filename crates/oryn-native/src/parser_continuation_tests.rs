//! Continuation regressions exercise the retained production root/frame owner.
//! No test-only parser algorithm owns or consumes the input stream.
use super::*;
use std::cell::Cell;

fn install(page: &mut RustExecutableDocument, input: crate::host::DocumentStream) {
    let dom = page.agent.dom.clone();
    let parser = LiveParserState::new(input, &dom, &mut page.agent.host).unwrap();
    let realm = page.agent.host.active_realm();
    let load = PreparedDocumentLoad {
        realm,
        identity: page.agent.host.automation_realm_identity(realm).unwrap(),
    };
    let mut owner =
        FrameScriptPreparation::root_html(load, dom, &page.dynamic_scripts, &mut page.agent.host)
            .unwrap();
    owner.install_html_parser(parser);
    page.frame_script_preparations.push_back(owner);
}
fn tick(page: &mut RustExecutableDocument) -> Result<(), HostError> {
    for owner in &mut page.frame_script_preparations {
        owner.parser_retry = None;
    }
    page.poll_frame_script_preparations()
}
fn finish(page: &mut RustExecutableDocument) {
    for _ in 0..256 {
        tick(page).unwrap();
        if page.frame_script_preparations.is_empty() {
            return;
        }
    }
    panic!("retained parser did not finish ready input");
}
struct ControlledInput {
    ready: Rc<Cell<bool>>,
    chunks: VecDeque<String>,
    polls: Rc<Cell<usize>>,
    dropped: Rc<Cell<bool>>,
}
impl crate::host::DocumentInput for ControlledInput {
    fn poll_next(&mut self) -> std::task::Poll<Option<Result<String, HostError>>> {
        self.polls.set(self.polls.get() + 1);
        if !self.ready.get() {
            return std::task::Poll::Pending;
        }
        std::task::Poll::Ready(self.chunks.pop_front().map(Ok))
    }
}
impl Drop for ControlledInput {
    fn drop(&mut self) {
        self.dropped.set(true);
    }
}
type ControlledStreamFixture = (
    crate::host::DocumentStream,
    Rc<Cell<bool>>,
    Rc<Cell<usize>>,
    Rc<Cell<bool>>,
);

fn controlled(chunks: Vec<String>, ready: bool) -> ControlledStreamFixture {
    let ready = Rc::new(Cell::new(ready));
    let polls = Rc::new(Cell::new(0));
    let dropped = Rc::new(Cell::new(false));
    (
        crate::host::DocumentStream {
            content_language: None,
            chunks: Box::new(ControlledInput {
                ready: ready.clone(),
                chunks: chunks.into(),
                polls: polls.clone(),
                dropped: dropped.clone(),
            }),
            content_type: None,
            encoding: Default::default(),
        },
        ready,
        polls,
        dropped,
    )
}

#[test]
fn live_parser_distinguishes_pending_input_from_eof() {
    let mut page = RustExecutableDocument::load("").unwrap();
    let (input, ready, _, dropped) = controlled(vec!["<body><p id=late>late</p>".into()], false);
    install(&mut page, input);
    tick(&mut page).unwrap();
    assert!(
        page.frame_script_preparations
            .front()
            .unwrap()
            .waiting_input()
    );
    assert!(page.node_by_id("late").is_none());
    assert!(!dropped.get());
    page.agent
        .host
        .execute("globalThis.otherTaskRan=true")
        .unwrap();
    ready.set(true);
    finish(&mut page);
    assert!(page.node_by_id("late").is_some());
    assert!(dropped.get());
    assert_eq!(
        page.agent
            .host
            .evaluate_string("String(otherTaskRan)")
            .unwrap(),
        "true"
    );
}
#[test]
fn live_parser_continuation_yields_between_input_slices() {
    let mut page = RustExecutableDocument::load("").unwrap();
    let (input, _, polls, _) = controlled(
        vec![
            "<body><p id=first>first</p>".into(),
            "<p id=second>second</p>".into(),
        ],
        true,
    );
    install(&mut page, input);
    tick(&mut page).unwrap();
    assert_eq!(polls.get(), 1);
    assert!(page.node_by_id("first").is_some());
    assert!(page.node_by_id("second").is_none());
    page.agent
        .host
        .execute("globalThis.betweenParserSlices=42")
        .unwrap();
    tick(&mut page).unwrap();
    assert_eq!(polls.get(), 2);
    finish(&mut page);
    assert!(page.node_by_id("second").is_some());
    assert_eq!(
        page.agent
            .host
            .evaluate_string("String(betweenParserSlices)")
            .unwrap(),
        "42"
    );
}
#[test]
fn retiring_parser_realm_releases_input_without_polling_or_publishing_suffix() {
    for replace in [false, true] {
        let mut page = RustExecutableDocument::load("").unwrap();
        let (input, _, polls, dropped) =
            controlled(vec!["<p id=obsolete>obsolete</p>".into()], true);
        install(&mut page, input);
        if replace {
            page.agent.host.execute("document.open()").unwrap();
        } else {
            page.agent
                .host
                .stop_pending_document_load(page.top_realm)
                .unwrap();
        }
        tick(&mut page).unwrap();
        assert_eq!(polls.get(), 0);
        assert!(dropped.get());
        assert!(page.node_by_id("obsolete").is_none());
    }
}

#[cfg(unix)]
fn inject(
    page: &mut RustExecutableDocument,
    node: NodeId,
    source: &str,
) -> std::sync::Weak<std::sync::atomic::AtomicUsize> {
    let dom = page.agent.dom.clone();
    let mut record = capture_parser_script_node(&page.agent.host, &dom, node).unwrap();
    record.1 = Some("https://example.test/ready.js".into());
    record.2 = None;
    record.4 = false;
    let identity = record.7.unwrap();
    let owner = page.frame_script_preparations.back_mut().unwrap();
    let (sender, budget, ticket) = owner.inject_async_source(record.clone(), &page.dynamic_scripts);
    let result = retain_parser_script_source(
        Ok((
            "https://example.test/ready.js".into(),
            source.into(),
            false,
            "no-referrer".into(),
        )),
        &budget,
    );
    sender
        .send(ParserResourceCompletion::Script {
            realm: identity.realm,
            stream_epoch: identity.stream_epoch,
            document_generation: identity.document_generation,
            document: identity.document,
            order: 0,
            record: ticket,
            asynchronous: true,
            result,
        })
        .unwrap();
    Arc::downgrade(&budget)
}
#[cfg(unix)]
#[test]
fn ready_async_sources_run_while_html_input_is_pending() {
    for action in ["run", "adopt", "replace"] {
        let mut page =
            RustExecutableDocument::load("<script id=source type=application/json></script>")
                .unwrap();
        let node = page.node_by_id("source").unwrap();
        let (input, _, polls, dropped) = controlled(vec![], false);
        install(&mut page, input);
        inject(
            &mut page,
            node,
            if action == "replace" {
                "document.open();globalThis.asyncRan=true"
            } else {
                "globalThis.asyncRan=true"
            },
        );
        if action == "adopt" {
            page.agent.host.execute("document.implementation.createHTMLDocument('foreign').adoptNode(document.getElementById('source'))").unwrap();
        }
        for _ in 0..8 {
            tick(&mut page).unwrap();
            if action == "replace" && dropped.get() {
                break;
            }
        }
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(globalThis.asyncRan===true)")
                .unwrap(),
            if action == "adopt" { "false" } else { "true" }
        );
        if action == "replace" {
            assert!(dropped.get());
            let before = polls.get();
            tick(&mut page).unwrap();
            assert_eq!(
                polls.get(),
                before,
                "replacement never polls obsolete input again"
            );
        } else {
            assert!(!dropped.get());
            assert!(
                page.frame_script_preparations
                    .front()
                    .unwrap()
                    .waiting_input()
            );
        }
    }
}
#[cfg(unix)]
#[test]
fn ready_source_bursts_keep_unconsumed_responses_between_page_slices() {
    let mut page =
        RustExecutableDocument::load(&"<script type=application/json></script>".repeat(130))
            .unwrap();
    let dom = page.agent.dom.clone();
    let nodes = dom
        .borrow()
        .query_selector_all(dom.borrow().document(), "script")
        .unwrap();
    page.agent
        .host
        .execute("globalThis.completedSources=0")
        .unwrap();
    let (input, _, _, _) = controlled(vec![], false);
    install(&mut page, input);
    for node in nodes {
        inject(&mut page, node, "completedSources++");
    }
    let mut previous = 0;
    for _ in 0..128 {
        tick(&mut page).unwrap();
        let completed = page
            .agent
            .host
            .evaluate_string("String(completedSources)")
            .unwrap()
            .parse::<usize>()
            .unwrap();
        assert!(
            completed >= previous && completed - previous <= 8,
            "production async slice remains bounded"
        );
        previous = completed;
        if completed == 130 {
            break;
        }
    }
    assert_eq!(previous, 130);
    assert!(
        page.frame_script_preparations
            .front()
            .unwrap()
            .asynchronous
            .is_empty()
    );
}
#[cfg(unix)]
#[test]
fn live_parser_pending_resource_returns_owned_wait_state() {
    let mut page =
        RustExecutableDocument::load("<script id=source type=application/json></script>").unwrap();
    let node = page.node_by_id("source").unwrap();
    let (input, _, _, dropped) = controlled(vec![], false);
    install(&mut page, input);
    let dom = page.agent.dom.clone();
    let mut record = capture_parser_script_node(&page.agent.host, &dom, node).unwrap();
    record.1 = Some("https://example.test/pending.js".into());
    record.2 = None;
    record.4 = false;
    let owner = page.frame_script_preparations.back_mut().unwrap();
    let (_sender, _budget, _ticket) = owner.inject_async_source(record, &page.dynamic_scripts);
    let lifetime = Arc::downgrade(
        &owner
            .asynchronous
            .back()
            .unwrap()
            .source
            .as_ref()
            .unwrap()
            .lifetime,
    );
    tick(&mut page).unwrap();
    assert!(lifetime.upgrade().is_some());
    assert!(!dropped.get());
    page.agent
        .host
        .execute("globalThis.whileParserWaits=true")
        .unwrap();
    page.frame_script_preparations.clear();
    assert!(lifetime.upgrade().is_none());
    assert!(dropped.get());
    assert_eq!(
        page.agent
            .host
            .evaluate_string("String(whileParserWaits)")
            .unwrap(),
        "true"
    );
}

#[cfg(unix)]
#[test]
fn pending_parser_script_resumes_original_insertion_point_after_success_or_failure() {
    struct Script {
        success: bool,
        requests: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl crate::network::NetworkBroker for Script {
        fn send(
            &self,
            request: crate::network::NetworkRequest,
        ) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
            self.requests
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(crate::network::NetworkResponse{response_policy: None,timing:None,request_id:request.request_id,status:if self.success{200}else{404},headers:vec![("content-type".into(),"text/javascript".into())],body:b"globalThis.suffixHidden=!document.getElementById('following');document.write('<b id=inserted>inserted</b>');globalThis.blockingRuns=(globalThis.blockingRuns||0)+1".to_vec()})
        }
    }
    for success in [false, true] {
        let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut page = RustExecutableDocument::load_with_environment(
            "",
            &BTreeMap::new(),
            Some(SharedNetworkBroker::new(Script {
                success,
                requests: requests.clone(),
            })),
            Some(Url::parse("https://example.test/root").unwrap()),
        )
        .unwrap();
        let source = "<script src='blocking.js'></script><p id=following>following</p>";
        install(
            &mut page,
            crate::host::DocumentStream::html(Box::new(std::iter::once(Ok(source.into())))),
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !page.frame_script_preparations.is_empty() {
            tick(&mut page).unwrap();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(page.node_by_id("following").is_some());
        assert_eq!(page.node_by_id("inserted").is_some(), success);
        assert_eq!(
            page.agent
                .host
                .evaluate_string(
                    "String(globalThis.blockingRuns===1&&globalThis.suffixHidden===true)"
                )
                .unwrap(),
            if success { "true" } else { "false" }
        );
    }
}
#[cfg(unix)]
#[test]
fn parser_inline_script_waits_for_style_and_resumes_after_native_applicability_changes() {
    for release in ["completion", "failure", "media", "disabled", "removed"] {
        let mut page =
            RustExecutableDocument::load("<style id=gate>body{color:black}</style>").unwrap();
        let dom = page.agent.dom.clone();
        let sheet = page.node_by_id("gate").unwrap();
        let revision = dom.borrow().arena().stylesheet_revision(sheet).unwrap();
        let source = "<script>globalThis.styleRuns=(globalThis.styleRuns||0)+1;globalThis.suffixHidden=!document.getElementById('suffix');document.write('<b id=written>write</b>')</script><p id=suffix>suffix</p>";
        install(
            &mut page,
            crate::host::DocumentStream::html(Box::new(std::iter::once(Ok(source.into())))),
        );
        let owner = page.frame_script_preparations.back_mut().unwrap();
        owner.style_nodes.clear();
        let mut styles = ParserResourceScheduler::with_page_limits(&page.dynamic_scripts);
        styles.reserve_stylesheet(sheet, revision).unwrap();
        owner.styles = Some(styles);
        tick(&mut page).unwrap();
        tick(&mut page).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("typeof styleRuns").unwrap(),
            "undefined"
        );
        assert!(page.node_by_id("suffix").is_none());
        match release {
            "media" => page
                .agent
                .host
                .execute("document.getElementById('gate').media='print'")
                .unwrap(),
            "disabled" => page
                .agent
                .host
                .execute("document.getElementById('gate').sheet.disabled=true")
                .unwrap(),
            "removed" => page
                .agent
                .host
                .execute("document.getElementById('gate').remove()")
                .unwrap(),
            _ => {
                let owner = page.frame_script_preparations.back().unwrap();
                let identity = owner.live_parser.as_ref().unwrap().identity;
                let result = if release == "completion" {
                    Ok((
                        "https://example.test/gate.css".into(),
                        "body{color:green}".into(),
                        Vec::new(),
                    ))
                } else {
                    Err("fixture stylesheet failure".into())
                };
                owner
                    .styles
                    .as_ref()
                    .unwrap()
                    .sender
                    .send(ParserResourceCompletion::Stylesheet {
                        identity,
                        node: sheet,
                        revision,
                        result,
                    })
                    .unwrap();
            }
        }
        for _ in 0..32 {
            tick(&mut page).unwrap();
            if page.node_by_id("suffix").is_some() {
                break;
            }
        }
        assert!(page.node_by_id("suffix").is_some());
        assert!(page.node_by_id("written").is_some());
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(styleRuns===1&&suffixHidden)")
                .unwrap(),
            "true"
        );
        // Unchanged pending fetch custody survives applicability-only release.
        if !matches!(release, "completion" | "failure") {
            assert!(page.frame_script_preparations.iter().any(|owner| {
                owner
                    .styles
                    .as_ref()
                    .is_some_and(|styles| styles.pending == 1)
            }));
        }
    }
}

#[test]
fn construction_publication_failure_retains_consumed_prefix_and_constructor_identity() {
    let mut page=RustExecutableDocument::load("<script>globalThis.constructors=0;customElements.define('x-retained',class extends HTMLElement{constructor(){super();constructors++;globalThis.constructed=this}})</script>").unwrap();
    let(input,_,polls,_)=controlled(vec!["<x-retained id=retained></x-retained><script>globalThis.afterConstructor=(globalThis.afterConstructor||0)+1</script><p id=tail>tail</p>".into()],true);
    install(&mut page, input);
    XML_PUBLICATION_FAILURE
        .with(|failure| failure.set(Some(XmlPublicationFailure::AfterHtmlTokenization)));
    assert!(tick(&mut page).is_err());
    assert_eq!(polls.get(), 1);
    assert!(!page.frame_script_preparations.is_empty());
    assert_eq!(
        page.agent
            .host
            .evaluate_string("String(constructors)")
            .unwrap(),
        "1"
    );
    finish(&mut page);
    assert_eq!(page.agent.host.evaluate_string("String(constructors===1&&constructed===document.getElementById('retained')&&afterConstructor===1)").unwrap(),"true");
    assert!(page.node_by_id("tail").is_some());
}
#[test]
fn eof_preparation_failure_keeps_terminal_input_without_repolling_or_losing_parser() {
    let mut page = RustExecutableDocument::load("").unwrap();
    let (input, _, polls, dropped) = controlled(vec!["<body><p id=kept>kept</p>".into()], true);
    install(&mut page, input);
    tick(&mut page).unwrap();
    assert_eq!(polls.get(), 1);
    XML_PUBLICATION_FAILURE.with(|failure| failure.set(Some(XmlPublicationFailure::AfterHtmlEof)));
    assert!(tick(&mut page).is_err());
    assert_eq!(polls.get(), 2);
    assert!(!dropped.get());
    assert!(page.frame_script_preparations.front().unwrap().html_eof);
    finish(&mut page);
    assert_eq!(polls.get(), 2);
    assert!(dropped.get());
    assert!(page.node_by_id("kept").is_some());
}

#[test]
fn nested_page_checkpoints_share_one_parser_input_slice_and_release_budget() {
    let mut page = RustExecutableDocument::load("").unwrap();
    let (input, _, polls, _) = controlled(
        vec![
            "<p id=budgetOne>one</p>".into(),
            "<p id=budgetTwo>two</p>".into(),
        ],
        true,
    );
    install(&mut page, input);
    {
        let _turn = page.parser_turn.enter();
        page.checkpoint().unwrap();
        page.checkpoint().unwrap();
        page.resume_pending_parser().unwrap();
        assert_eq!(
            polls.get(),
            1,
            "nested fixed points and trailing resume share a task budget"
        );
        assert!(page.node_by_id("budgetOne").is_some());
        assert!(page.node_by_id("budgetTwo").is_none());
        page.agent
            .host
            .execute("globalThis.budgetTaskRan=true")
            .unwrap();
    }
    page.poll_context_events().unwrap();
    assert_eq!(polls.get(), 2, "the next page task receives its own slice");
    assert!(page.node_by_id("budgetTwo").is_some());
    assert_eq!(
        page.agent
            .host
            .evaluate_string("String(budgetTaskRan)")
            .unwrap(),
        "true"
    );
}

#[test]
fn newly_discovered_child_gets_a_parser_slice_before_parent_input_resumes() {
    let mut page = RustExecutableDocument::load("<body></body>").unwrap();
    page.agent.host.execute("globalThis.childTurn=0").unwrap();
    let (input, _, polls, _) = controlled(
        vec![r#"<iframe srcdoc="<script>parent.childTurn++</script>"></iframe>"#.into()],
        true,
    );
    install(&mut page, input);
    {
        let _turn = page.parser_turn.enter();
        page.checkpoint().unwrap();
        page.checkpoint().unwrap();
        assert_eq!(polls.get(), 1, "the parent still has one input slice");
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(childTurn)")
                .unwrap(),
            "1",
            "a new child must not inherit its parent's exhausted parser budget"
        );
    }
    finish(&mut page);
    assert_eq!(page.evaluate("String(childTurn)").unwrap(), "1");
}

#[test]
fn parser_turn_budget_bounds_new_documents_and_resets_for_the_next_task() {
    let budget = Rc::new(ParserTurnBudget::default());
    {
        let _turn = budget.enter();
        assert!(budget.claim());
        for realm in 1..64 {
            assert!(budget.claim_frame(RealmId(realm)));
            assert!(!budget.claim_frame(RealmId(realm)));
        }
        let _nested = budget.enter();
        assert!(!budget.claim_frame(RealmId(64)));
        assert!(!budget.claim());
    }
    let _next = budget.enter();
    assert!(budget.claim_frame(RealmId(64)));
    assert!(budget.claim());
}

#[test]
fn parser_turn_budget_resets_after_early_error_and_nested_unwind() {
    let budget = Rc::new(ParserTurnBudget::default());
    let fail = || -> Result<(), HostError> {
        let _outer = budget.enter();
        assert!(budget.claim());
        {
            let _inner = budget.enter();
            assert!(!budget.claim());
        }
        Err(HostError::Execution("injected".into()))
    };
    assert!(fail().is_err());
    assert_eq!(budget.depth.get(), 0);
    {
        let _next = budget.enter();
        assert!(budget.claim());
    }
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _outer = budget.enter();
        assert!(budget.claim());
        let _inner = budget.enter();
        panic!("nested parser task");
    }));
    assert!(unwound.is_err());
    assert_eq!(budget.depth.get(), 0);
    let _next = budget.enter();
    assert!(budget.claim());
}

#[test]
fn buffered_input_observation_shares_the_nested_page_task_budget() {
    let mut page = RustExecutableDocument::load("").unwrap();
    page.agent.host.execute("document.open()").unwrap();
    let (input, _, polls, dropped) = controlled(vec![], false);
    let identity = capture_live_parser_identity(&page.agent.host, &page.agent.dom).unwrap();
    page.pending_buffered = Some((
        BufferedNavigationInput::new(input, "https://example.test/buffered".into(), 1024),
        identity,
    ));
    {
        let _turn = page.parser_turn.enter();
        assert!(!page.resume_pending_parser().unwrap());
        assert_eq!(polls.get(), 1);
        assert!(!page.resume_pending_parser().unwrap());
        assert_eq!(polls.get(), 1);
    }
    assert!(!page.resume_pending_parser().unwrap());
    assert_eq!(polls.get(), 2);
    assert!(!dropped.get());
    page.stop_pending_document_load().unwrap();
    assert!(dropped.get());
}

#[test]
fn data_block_batch_keeps_each_microtask_boundary_and_bounds_one_source_slice() {
    let mut page = RustExecutableDocument::load("<body></body>").unwrap();
    page.agent.host.execute("globalThis.dataChecks=0;globalThis.dataOrder=true;new MutationObserver(records=>{for(const record of records){for(const node of record.addedNodes){if(node.nodeType===1&&/^d[0-9]+$/.test(node.id)){dataChecks++;dataOrder=dataOrder&&!document.getElementById('d'+(Number(node.id.slice(1))+1));}}}}).observe(document,{subtree:true,childList:true})").unwrap();
    let mut source = (0..70)
        .map(|index| format!("<script id=d{index} type=application/json>{{}}</script>"))
        .collect::<String>();
    source.push_str("<script>globalThis.realBoundary=!document.getElementById('followingData')</script><p id=followingData>later</p>");
    let (input, _, polls, _) = controlled(vec![source], true);
    install(&mut page, input);
    tick(&mut page).unwrap();
    assert_eq!(polls.get(), 1);
    assert!(page.node_by_id("d63").is_some());
    assert!(page.node_by_id("d64").is_none());
    assert_eq!(
        page.agent
            .host
            .evaluate_string("dataChecks+'|'+dataOrder")
            .unwrap(),
        "64|true"
    );
    tick(&mut page).unwrap();
    assert_eq!(
        polls.get(),
        1,
        "batch continuation owns the suffix rather than another source read"
    );
    assert_eq!(
        page.agent
            .host
            .evaluate_string("dataChecks+'|'+dataOrder+'|'+realBoundary")
            .unwrap(),
        "70|true|true"
    );
    finish(&mut page);
    assert!(page.node_by_id("followingData").is_some());
}
