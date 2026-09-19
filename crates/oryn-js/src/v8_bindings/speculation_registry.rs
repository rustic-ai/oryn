//! Original-document ownership for parsed speculation rules.
use super::*;

type Identity = (NodeId, u64, u64, u64);
fn identity(state: &V8BindingState) -> Identity {
    (
        state.document_root(),
        state.document_generation(),
        state.document_stream_epoch(),
        state.document_authority_epoch(),
    )
}

#[derive(Default)]
pub(super) struct Registry {
    next: u64,
    sources: BTreeMap<u64, Source>,
    hover: BTreeMap<i32, Hover>,
    eligible: BTreeSet<(u64, usize)>,
}
struct Hover {
    document: Identity,
    node: oryn_dom::TrackedNodeLease,
    url: String,
    since: std::time::Instant,
}
const HOVER_DELAY: std::time::Duration = std::time::Duration::from_millis(200);
struct Source {
    document: Identity,
    script: oryn_dom::TrackedNodeLease,
    candidates: Vec<crate::speculation_rules::Candidate>,
}

/// The host reconciles this complete, non-consuming snapshot against its live
/// candidate owners. Failed load admission cannot silently lose a candidate;
/// removal is represented by absence from the next authoritative snapshot.
#[derive(Clone, Debug)]
pub struct NativePrerenderCandidate {
    pub rule_source: u64,
    pub ordinal: usize,
    pub candidate: crate::speculation_rules::Candidate,
    pub eligible: bool,
    pub eligibility_delay: Option<std::time::Duration>,
}

fn prune(state: &V8BindingState) {
    let current = identity(state);
    state
        .speculation_rules
        .borrow_mut()
        .sources
        .retain(|_, source| {
            let (dom, node) = source.script.current();
            source.document == current
                && !state.execution_retired()
                && Rc::ptr_eq(&dom, &state.dom)
                && dom.borrow().is_connected(node).unwrap_or(false)
                && dom.borrow().node_document(node).ok().flatten() == Some(current.0)
        });
}

pub(super) fn register(
    state: &Rc<V8BindingState>,
    script: NodeId,
    source: &str,
) -> Result<(), V8BindingError> {
    prune(state);
    {
        let dom = state.dom.borrow();
        if state.execution_retired()
            || !dom.is_connected(script)?
            || dom.node_document(script)? != Some(state.document_root())
        {
            return Ok(());
        }
    }
    if state
        .speculation_rules
        .borrow()
        .sources
        .values()
        .any(|source| {
            let (dom, node) = source.script.current();
            Rc::ptr_eq(&dom, &state.dom) && node == script
        })
    {
        return Ok(());
    }
    let base = state.native_document_base_url(true)?;
    let parsed = match crate::speculation_rules::parse(source, &base, &base) {
        Ok(parsed) => parsed,
        Err(crate::speculation_rules::ParseError::Capacity) => {
            return Err(V8BindingError::Allocation("speculation rule capacity"));
        }
        Err(_) => {
            // Native error publication retains its callback suffix and does
            // not hold a DOM borrow while an error handler changes <base>.
            state.queue_script_error(script)?;
            state.dom.borrow_mut().mark_script_started(script)?;
            return Ok(());
        }
    };
    let target = oryn_dom::TrackedNodeLease::new(&state.dom, script)?;
    let mut registry = state.speculation_rules.borrow_mut();
    if registry.sources.len() >= 64
        || registry
            .sources
            .values()
            .map(|source| source.candidates.len())
            .sum::<usize>()
            .saturating_add(parsed.prerenders.len())
            > crate::speculation_rules::MAX_CANDIDATES
    {
        return Err(V8BindingError::Allocation(
            "document speculation candidate capacity",
        ));
    }
    let id = registry
        .next
        .checked_add(1)
        .ok_or(V8BindingError::Allocation("speculation source identity"))?;
    // Marking is fallible and precedes publication. No parser/author callback
    // executes between this native mutation and the registry insertion.
    state.dom.borrow_mut().mark_script_started(script)?;
    registry.sources.insert(
        id,
        Source {
            document: identity(state),
            script: target,
            candidates: parsed.prerenders,
        },
    );
    registry.next = id;
    Ok(())
}

/// Native hit testing is the only interest producer. Script-dispatched pointer
/// events and click() cannot manufacture speculative loading eligibility.
pub(super) fn observe_native_pointer(
    state: &Rc<V8BindingState>,
    target: Option<NodeId>,
    input: &NativePointerInput,
    action: NativePointerAction,
) -> Result<(), V8BindingError> {
    observe_at(state, target, input, action, std::time::Instant::now())
}
fn link_url(
    state: &V8BindingState,
    target: Option<NodeId>,
) -> Result<Option<(NodeId, String)>, V8BindingError> {
    let base = Url::parse(&state.native_document_base_url(true)?)
        .map_err(|_| V8BindingError::Allocation("speculation base URL"))?;
    let encoding = hyperlinks::document_encoding(state, state.document_root())?;
    let dom = state.dom.borrow();
    let mut cursor = target;
    let mut seen = BTreeSet::new();
    while let Some(node) = cursor {
        if !seen.insert(node)
            || dom.node_document(node)? != Some(state.document_root())
            || !dom.is_connected(node)?
        {
            return Ok(None);
        }
        let record = dom.arena().get(node)?;
        if matches!(&record.kind,NodeKind::Element{name}if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&matches!(name.local.as_ref(),"a"|"area"))
        {
            if let Some(href) = record.attributes.get_null("href") {
                let value = String::from_utf16_lossy(&oryn_dom::dom_string::units(href));
                return Ok(hyperlinks::parse(&value, Some(&base), encoding)
                    .ok()
                    .filter(|url| matches!(url.scheme(), "http" | "https"))
                    .map(|url| (node, url.to_string())));
            }
        }
        cursor = dom.parent(node)?;
    }
    Ok(None)
}
fn observe_at(
    state: &Rc<V8BindingState>,
    target: Option<NodeId>,
    input: &NativePointerInput,
    action: NativePointerAction,
    now: std::time::Instant,
) -> Result<(), V8BindingError> {
    prune(state);
    if state.execution_retired() || state.execution_suspended() || state.document_prerendering() {
        return Ok(());
    }
    if action == NativePointerAction::Cancel {
        state.speculation_rules.borrow_mut().hover.remove(&input.id);
        return Ok(());
    }
    let Some((node, url)) = link_url(state, target)? else {
        state.speculation_rules.borrow_mut().hover.remove(&input.id);
        return Ok(());
    };
    if action == NativePointerAction::Down && input.primary && matches!(input.button, 0 | 1) {
        let mut registry = state.speculation_rules.borrow_mut();
        let mut selected = Vec::new();
        for (source, record) in &registry.sources {
            for (ordinal, candidate) in record.candidates.iter().enumerate() {
                if candidate.url == url {
                    selected.push((*source, ordinal));
                }
            }
        }
        registry.eligible.extend(selected);
        registry.hover.remove(&input.id);
        return Ok(());
    }
    if input.buttons != 0 {
        state.speculation_rules.borrow_mut().hover.remove(&input.id);
        return Ok(());
    }
    if action == NativePointerAction::Move
        && input.buttons == 0
        && input.device != NativePointerDevice::Touch
    {
        let same = state
            .speculation_rules
            .borrow()
            .hover
            .get(&input.id)
            .is_some_and(|hover| {
                let (owner, current) = hover.node.current();
                hover.document == identity(state)
                    && Rc::ptr_eq(&owner, &state.dom)
                    && current == node
                    && hover.url == url
            });
        if !same {
            let lease = oryn_dom::TrackedNodeLease::new(&state.dom, node)?;
            state.speculation_rules.borrow_mut().hover.insert(
                input.id,
                Hover {
                    document: identity(state),
                    node: lease,
                    url,
                    since: now,
                },
            );
        }
    }
    Ok(())
}
fn candidates_at(state: &V8BindingState, now: std::time::Instant) -> Vec<NativePrerenderCandidate> {
    prune(state);
    // Recheck live URL/connection and document identity before a delayed hover
    // can activate a rule. A moved or retargeted anchor cannot spend old intent.
    let hover = state
        .speculation_rules
        .borrow()
        .hover
        .iter()
        .map(|(id, hover)| {
            (
                *id,
                hover.node.current(),
                hover.document,
                hover.url.clone(),
                hover.since,
            )
        })
        .collect::<Vec<_>>();
    let mut active = Vec::new();
    let mut invalid = Vec::new();
    for (id, (owner, node), document, url, since) in hover {
        if document != identity(state)
            || !Rc::ptr_eq(&owner, &state.dom)
            || link_url(state, Some(node))
                .ok()
                .flatten()
                .is_none_or(|(current, current_url)| current != node || current_url != url)
        {
            invalid.push(id);
            continue;
        }
        active.push((
            url,
            HOVER_DELAY.saturating_sub(now.saturating_duration_since(since)),
        ));
    }
    let mut registry = state.speculation_rules.borrow_mut();
    for id in invalid {
        registry.hover.remove(&id);
    }
    let live_sources = registry.sources.keys().copied().collect::<BTreeSet<_>>();
    registry
        .eligible
        .retain(|(source, _)| live_sources.contains(source));
    let mut admitted = Vec::new();
    let mut result = Vec::new();
    for (source, record) in &registry.sources {
        for (ordinal, candidate) in record.candidates.iter().enumerate() {
            let delay = if candidate.eagerness == crate::speculation_rules::Eagerness::Moderate {
                active
                    .iter()
                    .filter(|(url, _)| url == &candidate.url)
                    .map(|(_, delay)| *delay)
                    .min()
            } else {
                None
            };
            let eligible = matches!(
                candidate.eagerness,
                crate::speculation_rules::Eagerness::Immediate
                    | crate::speculation_rules::Eagerness::Eager
            ) || registry.eligible.contains(&(*source, ordinal))
                || delay == Some(std::time::Duration::ZERO);
            if eligible {
                admitted.push((*source, ordinal));
            }
            result.push(NativePrerenderCandidate {
                rule_source: *source,
                ordinal,
                candidate: candidate.clone(),
                eligible,
                eligibility_delay: (!eligible).then_some(delay).flatten(),
            });
        }
    }
    registry.eligible.extend(admitted);
    result
}
pub(super) fn candidates(state: &V8BindingState) -> Vec<NativePrerenderCandidate> {
    candidates_at(state, std::time::Instant::now())
}
pub(super) fn clear_interest(state: &V8BindingState) {
    state.speculation_rules.borrow_mut().hover.clear();
}
pub(super) fn clear(state: &V8BindingState) {
    let mut registry = state.speculation_rules.borrow_mut();
    registry.sources.clear();
    registry.hover.clear();
    registry.eligible.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    fn eligibility_fixture(scope: &mut v8::PinScope) -> (Rc<V8BindingState>, NodeId) {
        let context = scope.get_current_context();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><a id=link href='/next'><span id=child>next</span></a><script id=rules type=speculationrules></script>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(74), 1).unwrap();
        state.commit_location("https://rules.test/source").unwrap();
        let script = state.dom.borrow().find_by_id("rules").unwrap();
        let child = state.dom.borrow().find_by_id("child").unwrap();
        register(&state,script,r#"{"prerender":[{"source":"list","urls":["/next"],"eagerness":"moderate"},{"source":"list","urls":["/next"],"eagerness":"conservative","target_hint":"_blank"}]}"#).unwrap();
        (state, child)
    }
    #[test]
    fn hover_waits_then_admits_only_moderate_and_pointerdown_admits_conservative() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let (state, child) = eligibility_fixture(scope);
        let now = std::time::Instant::now();
        let mut input = NativePointerInput::default();
        assert!(
            candidates_at(&state, now)
                .iter()
                .all(|candidate| !candidate.eligible)
        );
        observe_at(&state, Some(child), &input, NativePointerAction::Move, now).unwrap();
        let before = candidates_at(
            &state,
            now + HOVER_DELAY - std::time::Duration::from_millis(1),
        );
        assert!(!before[0].eligible);
        assert!(before[0].eligibility_delay.is_some());
        assert!(!before[1].eligible);
        let after = candidates_at(&state, now + HOVER_DELAY);
        assert!(after[0].eligible);
        assert!(!after[1].eligible);
        input.button = 0;
        input.buttons = 1;
        observe_at(
            &state,
            Some(child),
            &input,
            NativePointerAction::Down,
            now + HOVER_DELAY,
        )
        .unwrap();
        assert!(
            candidates_at(&state, now + HOVER_DELAY)
                .iter()
                .all(|candidate| candidate.eligible)
        );
    }
    #[test]
    fn delayed_interest_revalidates_href_removal_and_source_identity() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let (state, child) = eligibility_fixture(scope);
        let now = std::time::Instant::now();
        let input = NativePointerInput::default();
        observe_at(&state, Some(child), &input, NativePointerAction::Move, now).unwrap();
        eval(scope, "document.getElementById('link').href='/other'");
        assert!(
            candidates_at(&state, now + HOVER_DELAY)
                .iter()
                .all(|candidate| !candidate.eligible)
        );
        eval(scope, "document.getElementById('link').href='/next'");
        observe_at(&state, Some(child), &input, NativePointerAction::Move, now).unwrap();
        observe_at(&state, None, &input, NativePointerAction::Cancel, now).unwrap();
        assert!(
            candidates_at(&state, now + HOVER_DELAY)
                .iter()
                .all(|candidate| !candidate.eligible)
        );
        observe_at(&state, Some(child), &input, NativePointerAction::Move, now).unwrap();
        reset_native_pointer_sources(&state);
        assert!(
            candidates_at(&state, now + HOVER_DELAY)
                .iter()
                .all(|candidate| !candidate.eligible)
        );
        observe_at(&state, Some(child), &input, NativePointerAction::Move, now).unwrap();
        eval(scope, "document.getElementById('link').remove()");
        assert!(
            candidates_at(&state, now + HOVER_DELAY)
                .iter()
                .all(|candidate| !candidate.eligible)
        );
        eval(scope, "document.getElementById('rules').remove()");
        assert!(candidates_at(&state, now + HOVER_DELAY).is_empty());
    }
    #[test]
    fn authored_pointer_events_do_not_admit_rules_but_native_hit_testing_does() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let (state, child) = eligibility_fixture(scope);
        eval(
            scope,
            "document.getElementById('child').dispatchEvent(new PointerEvent('pointerdown',{bubbles:true}));document.getElementById('child').dispatchEvent(new PointerEvent('pointerover',{bubbles:true}))",
        );
        assert!(
            candidates(&state)
                .iter()
                .all(|candidate| !candidate.eligible)
        );
        state.set_layout(BindingLayout {
            viewport: Some((100., 100.)),
            geometry: BTreeMap::from([(
                node_key(child),
                BindingGeometry::Available(BindingRect {
                    x: 0.,
                    y: 0.,
                    width: 100.,
                    height: 100.,
                }),
            )]),
            ..Default::default()
        });
        let input = NativePointerInput {
            x: 10.,
            y: 10.,
            button: 0,
            buttons: 1,
            ..Default::default()
        };
        dispatch_native_pointer(scope, &state, &input, NativePointerAction::Down).unwrap();
        assert!(
            candidates(&state)
                .iter()
                .all(|candidate| candidate.eligible)
        );
    }
    #[test]
    fn feature_detection_reaches_native_speculation_registration() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://rules.test/source").unwrap();
        assert!(eval(scope,r#"
            let conversions=0;
            if(HTMLScriptElement.supports({toString(){conversions++;return 'speculationrules'}})){
                let rules=document.createElement('script');rules.type='speculationrules';
                rules.textContent=JSON.stringify({prerender:[{source:'list',urls:['/next']}]});
                document.body.append(rules);
            }
            conversions===1&&!HTMLScriptElement.supports('Speculationrules')&&!HTMLScriptElement.supports(' speculationrules')
        "#).is_true());
        let entries = candidates(&state);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].candidate.url, "https://rules.test/next");
    }
    #[test]
    fn dynamic_speculation_preparation_preserves_native_inline_kind() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://rules.test/source").unwrap();
        let kinds = Rc::new(RefCell::new(Vec::new()));
        let observed = kinds.clone();
        state.set_parser_inline_policy(Rc::new(move |_, _, _, _, _, rules| {
            observed.borrow_mut().push(rules);
            Ok(rules)
        }));
        eval(
            scope,
            r#"let rules=document.createElement('script');rules.type='speculationrules';rules.textContent=JSON.stringify({prerender:[{source:'list',urls:['/next']}]});document.body.append(rules);let ordinary=document.createElement('script');ordinary.textContent='globalThis.disallowedRan=true';document.body.append(ordinary)"#,
        );
        assert_eq!(candidates(&state).len(), 1);
        assert_eq!(&*kinds.borrow(), &[true, false]);
        assert!(eval(scope, "globalThis.disallowedRan===undefined").is_true());
    }
    #[test]
    fn candidate_snapshot_retains_identity_until_source_removal_or_document_replacement() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><script id=rules type=speculationrules></script>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://rules.test/source").unwrap();
        let value = eval(scope, "document.getElementById('rules')");
        let script = argument_node(scope, &state, value).unwrap();
        register(
            &state,
            script,
            r#"{"prerender":[{"source":"list","urls":["/next"]}]}"#,
        )
        .unwrap();
        let first = candidates(&state);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].candidate.url, "https://rules.test/next");
        assert_eq!(candidates(&state)[0].rule_source, first[0].rule_source);
        register(
            &state,
            script,
            r#"{"prerender":[{"source":"list","urls":["/different"]}]}"#,
        )
        .unwrap();
        assert_eq!(candidates(&state)[0].candidate.url, first[0].candidate.url);
        eval(scope, "document.getElementById('rules').remove()");
        assert!(candidates(&state).is_empty());
        eval(
            scope,
            "document.open();document.write('<body></body>');document.close()",
        );
        assert!(candidates(&state).is_empty());
    }
}
