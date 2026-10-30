//! Selection WebIDL conversion, document membership and Range replacement.
use super::*;
fn error(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "Selection".into(),
        member: name.into(),
    }
}
pub(super) fn handles(member: &str) -> bool {
    matches!(
        member,
        "getComposedRanges"
            | "getRangeAt"
            | "addRange"
            | "removeRange"
            | "collapse"
            | "setPosition"
            | "selectAllChildren"
            | "extend"
            | "setBaseAndExtent"
            | "collapseToStart"
            | "collapseToEnd"
            | "containsNode"
    )
}
fn owner(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<Rc<V8BindingState>, V8BindingError> {
    if object.is_proxy() {
        return Err(error("TypeError"));
    }
    let state = binding_state_for_receiver(scope, object).ok_or_else(|| error("TypeError"))?;
    if !state
        .selection_object
        .borrow()
        .as_ref()
        .is_some_and(|actual| {
            actual
                .to_local(scope)
                .is_some_and(|actual| actual.strict_equals(object.into()))
        })
    {
        return Err(error("TypeError"));
    }
    Ok(state)
}
fn node(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<(Rc<V8BindingState>, NodeId), V8BindingError> {
    node_identity::argument(scope, value)?.ok_or_else(|| error("TypeError"))
}
fn offset(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Result<u32, V8BindingError> {
    value
        .uint32_value(scope)
        .ok_or(V8BindingError::JavaScriptException)
}
fn member(
    state: &V8BindingState,
    other: &V8BindingState,
    node: NodeId,
) -> Result<bool, V8BindingError> {
    Ok(Rc::ptr_eq(&state.dom, &other.dom)
        && shadow_including_tree_root(&other.dom.borrow(), node)? == state.document_root())
}
fn validate(
    state: &V8BindingState,
    node: NodeId,
    offset: u32,
    reject_doctype: bool,
) -> Result<(), V8BindingError> {
    let dom = state.dom.borrow();
    if reject_doctype && matches!(dom.arena().get(node)?.kind, NodeKind::DocumentType { .. }) {
        return Err(error("InvalidNodeTypeError"));
    }
    validate_boundary_offset(&dom, node, offset)
}
fn replace(
    state: &V8BindingState,
    anchor: BoundaryPoint,
    focus: BoundaryPoint,
) -> Result<(), V8BindingError> {
    // Admission allocates the replacement before changing the selection. The
    // original Range (including retained wrappers) remains independently live.
    update_selection_boundaries(state, anchor, focus, true)?;
    state.selection_modify_inline.set(None);
    state.selection_modify_caret.set(None);
    Ok(())
}
pub(super) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    method: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    owner(scope, object)?;
    if method == "getComposedRanges" {
        let state = owner(scope, object)?;
        return Ok(Some(selection_composed::get(scope, &state, args.get(0))?));
    }
    if method == "getRangeAt" {
        let index = offset(scope, args.get(0))?;
        let state = owner(scope, object)?;
        if index != 0 {
            return Err(error("IndexSizeError"));
        }
        let id = state
            .selection_range
            .get()
            .ok_or_else(|| error("IndexSizeError"))?;
        return Ok(Some(
            new_stateful_object(scope, &state, "Range", id)?.into(),
        ));
    }
    if matches!(method, "addRange" | "removeRange") {
        let range_object = args
            .get(0)
            .try_cast::<v8::Object>()
            .map_err(|_| error("TypeError"))?;
        let (range_owner, id) = range_boundaries::owner(scope, range_object)?;
        let state = owner(scope, object)?;
        if method == "removeRange" {
            if !Rc::ptr_eq(&state.dom, &range_owner.dom) || state.selection_range.get() != Some(id)
            {
                return Err(error("NotFoundError"));
            }
            clear_selection(&state);
            return Ok(None);
        }
        let range: RangeRecord = range_owner.dom.borrow().live_range(LiveRangeId(id))?.into();
        if state.selection_range.get().is_some()
            || !member(&state, &range_owner, range.start_container)?
            || !member(&state, &range_owner, range.end_container)?
        {
            return Ok(None);
        }
        let previous = (state.selection_anchor.get(), state.selection_focus.get());
        let anchor = BoundaryPoint {
            container: range.start_container,
            offset: range.start_offset,
        };
        let focus = BoundaryPoint {
            container: range.end_container,
            offset: range.end_offset,
        };
        state.selection_composed_range.set(None);
        state.selection_range_removal_revision.set(
            state
                .dom
                .borrow()
                .range_shadow_removal_revision(LiveRangeId(id)),
        );
        state.selected_text_control.borrow_mut().take();
        state.selection_range.set(Some(id));
        state.selection_backward.set(false);
        state.selection_anchor.set(Some(anchor));
        state.selection_focus.set(Some(focus));
        state.editing_overrides.borrow_mut().clear();
        if previous != (Some(anchor), Some(focus)) {
            state
                .selection_revision
                .set(state.selection_revision.get().saturating_add(1));
        }
        return Ok(None);
    }
    if matches!(method, "collapseToStart" | "collapseToEnd") {
        let state = owner(scope, object)?;
        let id = state
            .selection_range
            .get()
            .ok_or_else(|| error("InvalidStateError"))?;
        let range = state.dom.borrow().live_range(LiveRangeId(
            state.selection_composed_range.get().unwrap_or(id),
        ))?;
        let point = if method == "collapseToStart" {
            range.start
        } else {
            range.end
        };
        replace(&state, point, point)?;
        return Ok(None);
    }
    if method == "setBaseAndExtent" {
        node(scope, args.get(0))?;
        let anchor_offset = offset(scope, args.get(1))?;
        node(scope, args.get(2))?;
        let focus_offset = offset(scope, args.get(3))?;
        let (anchor_owner, anchor) = node(scope, args.get(0))?;
        let (focus_owner, focus) = node(scope, args.get(2))?;
        let state = owner(scope, object)?;
        validate(&anchor_owner, anchor, anchor_offset, false)?;
        validate(&focus_owner, focus, focus_offset, false)?;
        if !member(&state, &anchor_owner, anchor)? || !member(&state, &focus_owner, focus)? {
            return Ok(None);
        }
        replace(
            &state,
            BoundaryPoint {
                container: anchor,
                offset: anchor_offset,
            },
            BoundaryPoint {
                container: focus,
                offset: focus_offset,
            },
        )?;
        return Ok(None);
    }
    let nullable = matches!(method, "collapse" | "setPosition");
    if nullable && args.get(0).is_null_or_undefined() {
        offset(scope, args.get(1))?;
        let state = owner(scope, object)?;
        state.selected_text_control.borrow_mut().take();
        clear_selection(&state);
        return Ok(None);
    }
    node(scope, args.get(0))?;
    let converted = if matches!(method, "collapse" | "setPosition" | "extend") {
        offset(scope, args.get(1))?
    } else {
        0
    };
    let (target, node) = node(scope, args.get(0))?;
    let state = owner(scope, object)?;
    if method == "containsNode" {
        let Some(id) = state.selection_range.get() else {
            return Ok(Some(v8::Boolean::new(scope, false).into()));
        };
        if !Rc::ptr_eq(&state.dom, &target.dom) {
            return Ok(Some(v8::Boolean::new(scope, false).into()));
        }
        // Unlike Range.comparePoint, containsNode describes the visible
        // Selection. Unassigned light content and suppressed slot fallback
        // have no flat-tree path to this Document, but remain valid endpoints.
        {
            let dom = state.dom.borrow();
            let mut cursor = node;
            let mut seen = BTreeSet::new();
            loop {
                if cursor == state.document_root() {
                    break;
                }
                if !seen.insert(cursor) {
                    return Err(DomError::Cycle.into());
                }
                let Some(parent) = dom.flat_tree_parent(cursor)? else {
                    return Ok(Some(v8::Boolean::new(scope, false).into()));
                };
                cursor = parent;
            }
        }
        let range: RangeRecord = state.dom.borrow().live_range(LiveRangeId(id))?.into();
        let dom = state.dom.borrow();
        if tree_root(&dom, node)? != tree_root(&dom, range.start_container)? {
            return Ok(Some(v8::Boolean::new(scope, false).into()));
        }
        let before = relative_boundary(&dom, node, false).ok();
        let after = relative_boundary(&dom, node, true).ok();
        let contained = match (before, after) {
            (Some(before), Some(after)) => {
                if args.get(1).boolean_value(scope) {
                    compare_boundary_points(&dom, before, (range.end_container, range.end_offset))?
                        < 0
                        && compare_boundary_points(
                            &dom,
                            after,
                            (range.start_container, range.start_offset),
                        )? > 0
                } else {
                    compare_boundary_points(
                        &dom,
                        (range.start_container, range.start_offset),
                        before,
                    )? <= 0
                        && compare_boundary_points(
                            &dom,
                            after,
                            (range.end_container, range.end_offset),
                        )? <= 0
                }
            }
            _ => node == range.start_container || node == range.end_container,
        };
        return Ok(Some(v8::Boolean::new(scope, contained).into()));
    }
    if method == "extend" {
        // WebIDL conversion has already run; algorithm membership precedes the
        // empty-selection and boundary errors, as in pinned selection/extend.js.
        if !member(&state, &target, node)? {
            return Ok(None);
        }
        synchronize_selection_range(&state)?;
        let anchor = state
            .selection_anchor
            .get()
            .ok_or_else(|| error("InvalidStateError"))?;
        validate(&target, node, converted, true)?;
        let focus = BoundaryPoint {
            container: node,
            offset: converted,
        };
        // extend() collapses both public endpoints when the new focus has a
        // different tree root. Other composed-selection operations retain
        // their separate cross-root behavior.
        let anchor = if tree_root(&state.dom.borrow(), anchor.container)?
            != tree_root(&state.dom.borrow(), focus.container)?
        {
            focus
        } else {
            anchor
        };
        replace(&state, anchor, focus)?;
        return Ok(None);
    }
    let end = if method == "selectAllChildren" {
        validate(&target, node, 0, true)?;
        light_children(&target.dom.borrow(), node)?.len() as u32
    } else {
        validate(&target, node, converted, true)?;
        converted
    };
    if !member(&state, &target, node)? {
        return Ok(None);
    }
    let anchor = BoundaryPoint {
        container: node,
        offset: if method == "selectAllChildren" {
            0
        } else {
            end
        },
    };
    replace(
        &state,
        anchor,
        BoundaryPoint {
            container: node,
            offset: end,
        },
    )?;
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    #[test]
    fn replacement_identity_numeric_errors_and_node_membership_follow_pinned_selection_callers() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=a>abcd</div><div id=b>wxyz</div></body>")
                .into_runtime()
                .unwrap(),
        ));
        install_generated_bindings(scope, context, dom, RealmId(194), 1).unwrap();
        eval(
            scope,
            r#"(()=>{
          const check=(v,m)=>{if(!v)throw Error(m)},selection=getSelection(),a=document.getElementById('a').firstChild,b=document.getElementById('b').firstChild;
          const seed=()=>{selection.removeAllRanges();const range=document.createRange();range.setStart(a,1);range.setEnd(a,3);selection.addRange(range);return range};
          for(const method of ['collapse','setPosition','extend','setBaseAndExtent','collapseToStart','collapseToEnd','selectAllChildren']){
            const original=seed();
            if(method==='setBaseAndExtent')selection[method](b,3,b,1);else if(method.startsWith('collapseTo'))selection[method]();else selection[method](b,2);
            check(selection.getRangeAt(0)!==original&&original.startContainer===a&&original.startOffset===1&&original.endOffset===3,'replacement preserves original '+method);
            if(method==='selectAllChildren')check(selection.anchorNode===b&&selection.anchorOffset===0&&selection.focusOffset===0,'selectAllChildren uses child count for Text');
          }
          for(const method of ['collapse','setPosition','extend','getRangeAt']){
            const original=seed(),failure={method};let caught=false;const value={valueOf(){throw failure}};
            try{method==='getRangeAt'?selection[method](value):selection[method](b,value)}catch(error){caught=true;check(error===failure,'exact exception '+method)}
            check(caught&&selection.getRangeAt(0)===original,'failed conversion does not publish '+method);
          }
          let original=seed(),calls=0,failure={};try{selection.collapse(null,{valueOf(){calls++;throw failure}})}catch(error){check(error===failure,'null collapse conversion exception')}
          check(calls===1&&selection.getRangeAt(0)===original,'nullable argument still converts offset');selection.collapse(null,{valueOf(){calls++;return 0}});check(selection.rangeCount===0&&calls===2,'null collapse clears after conversion');
          seed();selection.collapse(b,4294967297);check(selection.focusOffset===1,'unsigned long modulo');check(selection.getRangeAt(4294967296)===selection.getRangeAt(0),'index modulo');
          let caught=false;try{selection.getRangeAt({valueOf(){selection.removeAllRanges();return 0}})}catch(error){caught=true;check(error.name==='IndexSizeError','post-conversion association')}check(caught,'empty range after conversion');
          const detached=document.createElement('div');detached.textContent='abc';original=seed();selection.collapse(detached,1);selection.selectAllChildren(detached);check(selection.getRangeAt(0)===original,'detached no-op');
          caught=false;try{selection.collapse(detached,99)}catch(error){caught=true;check(error.name==='IndexSizeError','offset before membership')}check(caught,'detached invalid offset');
          selection.removeAllRanges();selection.extend(detached,99);caught=false;try{selection.extend(b,0)}catch(error){caught=true;check(error.name==='InvalidStateError','empty selection extend')}check(caught,'empty local extend');
          const doctype=document.implementation.createDocumentType('html','','');for(const method of ['collapse','selectAllChildren']){caught=false;try{selection[method](doctype,0)}catch(error){caught=true;check(error.name==='InvalidNodeTypeError','doctype '+method)}check(caught,'doctype failure')}
          original=seed();selection.modify();selection.modify('move');check(selection.getRangeAt(0)===original&&selection.toString()==='bc','omitted modify arguments do not imply movement');
          let touched=false;try{selection.selectAllChildren(null)}catch(error){check(error instanceof TypeError,'nonnull selectAllChildren')}
          try{Selection.prototype.collapse.call({},a,{valueOf(){touched=true;return 0}})}catch(error){check(error instanceof TypeError,'receiver brand')}check(!touched,'brand precedes conversion');
          seed();for(const method of ['addRange','removeRange']){caught=false;try{selection[method]({})}catch(error){caught=true;check(error instanceof TypeError,'Range argument brand')}check(caught,'invalid Range rejected')}
          let disconnected=0;customElements.define('x-selection-delete',class extends HTMLElement{disconnectedCallback(){disconnected++}});const custom=document.createElement('x-selection-delete');document.body.append(custom);const removal=document.createRange();removal.selectNode(custom);selection.removeAllRanges();selection.addRange(removal);selection.deleteFromDocument();check(disconnected===1&&!custom.isConnected,'deleteFromDocument CEReactions before return');
        })()"#,
        );
    }
    #[test]
    fn same_arena_borrowed_range_association_preserves_one_wrapper_and_native_owner() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=target>text</div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let a = {
            let scope = &mut v8::ContextScope::new(scope, first);
            install_generated_bindings(scope, first, dom.clone(), RealmId(197), 1).unwrap()
        };
        let foreign_document = dom.borrow_mut().create(NodeKind::Document);
        let b = {
            let scope = &mut v8::ContextScope::new(scope, second);
            install_generated_bindings_with_root(
                scope,
                second,
                dom.clone(),
                RealmId(198),
                1,
                foreign_document,
            )
            .unwrap()
        };
        let range = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let target = dom
                .borrow()
                .get_element_by_id(a.document_root(), "target")
                .unwrap()
                .unwrap();
            let id = register_live_range(
                &b,
                RangeRecord {
                    start_container: target,
                    start_offset: 0,
                    end_container: target,
                    end_offset: 1,
                },
            )
            .unwrap();
            let object = new_stateful_object(scope, &b, "Range", id).unwrap();
            v8::Global::new(scope, object)
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let object = v8::Local::new(scope, &range);
        let id = stateful_id(scope, object).unwrap();
        let key = v8::String::new(scope, "borrowedRange").unwrap();
        first
            .global(scope)
            .set(scope, key.into(), object.into())
            .unwrap();
        eval(
            scope,
            r#"(()=>{const range=borrowedRange,prototype=Object.getPrototypeOf(range),selection=getSelection();range.marker=7;selection.addRange(range);if(selection.getRangeAt(0)!==range||range.marker!==7||Object.getPrototypeOf(range)!==prototype)throw Error('borrowed Range identity');selection.collapseToEnd();if(range.collapsed||range.toString()!=='text')throw Error('retained original Range changed');})()"#,
        );
        assert!(Rc::ptr_eq(&a.ranges, &b.ranges));
        assert!(Rc::ptr_eq(&a.range_objects, &b.range_objects));
        assert!(a.ranges.borrow().contains_key(&id));
        assert!(a.range_objects.borrow().contains_key(&id));
        collect_dead_ranges(&b, true).unwrap();
        assert!(dom.borrow().live_range(LiveRangeId(id)).is_ok());
    }
    #[test]
    fn adoption_during_ordered_conversions_refreshes_original_node_wrappers() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let a = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body><div id=anchor>abcd</div></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, first, dom, RealmId(195), 1).unwrap()
        };
        let b = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body><div id=target>wxyz</div></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, second, dom, RealmId(196), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let destination = wrap_node(scope, &b, b.document_root()).unwrap();
        let key = v8::String::new(scope, "destination").unwrap();
        first
            .global(scope)
            .set(scope, key.into(), destination.into())
            .unwrap();
        eval(
            scope,
            r#"(()=>{
          const check=(v,m)=>{if(!v)throw Error(m)},selection=getSelection(),anchor=document.getElementById('anchor').firstChild,target=destination.getElementById('target'),text=target.firstChild;
          selection.collapse(anchor,0);selection.collapse(text,{valueOf(){document.body.append(target);return 2}});check(selection.focusNode===text&&selection.focusOffset===2,'adopt incoming node');
          selection.collapse(anchor,1);const original=selection.getRangeAt(0);selection.extend(text,{valueOf(){destination.body.append(target);return 3}});check(selection.getRangeAt(0)===original,'adopt outgoing node no-op');
          let calls=[];selection.setBaseAndExtent(text,{valueOf(){calls.push('anchor');return 1}},text,{valueOf(){calls.push('focus');document.body.append(target);return 3}});
          check(calls.join(',')==='anchor,focus'&&selection.anchorNode===text&&selection.focusNode===text&&selection.toString()==='xy','both endpoints refreshed after final conversion');
          calls=[];let caught=false;try{selection.setBaseAndExtent(anchor,{valueOf(){calls.push('anchor');return 0}},null,{valueOf(){calls.push('focus');return 0}})}catch(error){caught=true;check(error instanceof TypeError,'focus Node brand')}
          check(caught&&calls.join(',')==='anchor','left-to-right conversion order');
          destination.body.append(target);check(!selection.containsNode(target,true),'foreign containsNode false');
          const foreign=destination.createRange();foreign.selectNodeContents(target);const before=selection.rangeCount?selection.getRangeAt(0):null;selection.addRange(foreign);check(selection.rangeCount===Number(before!==null),'foreign addRange ignored');
          caught=false;try{selection.removeRange(foreign)}catch(error){caught=true;check(error.name==='NotFoundError','foreign removeRange')}check(caught,'foreign removeRange rejected');
        })()"#,
        );
        let _ = a;
    }
}

pub(super) fn install_window(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let function = v8::Function::builder(window_get_selection)
        .length(0)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("Window.getSelection"))?;
    let name = v8::String::new(scope, "getSelection")
        .ok_or(V8BindingError::Allocation("Window.getSelection name"))?;
    function.set_name(name);
    context
        .global(scope)
        .define_own_property(
            scope,
            name.into(),
            function.into(),
            v8::PropertyAttribute::NONE,
        )
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok(())
}

pub(super) fn window_get_selection(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(state) = binding_state_for_receiver(scope, args.this()) else {
        throw_type_error(scope, "Window.getSelection requires a Window receiver");
        return;
    };
    if state.binding_global != BindingGlobal::Window
        || !global_events::is_global(scope, &state, args.this())
    {
        throw_type_error(scope, "Window.getSelection requires a Window receiver");
        return;
    }
    if !global_events::check_window_access(scope, &state, args.this()) {
        return;
    }
    if state.execution_retired() {
        rv.set(v8::null(scope).into());
        return;
    }
    let result = (|| {
        let global = state
            .global_object
            .to_local(scope)
            .ok_or(V8BindingError::Allocation("selection Window realm"))?;
        let context = global
            .get_creation_context(scope)
            .ok_or(V8BindingError::Allocation("selection Window context"))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        if let Some(selection) = state.selection_object.borrow().as_ref() {
            return selection
                .to_local(scope)
                .ok_or(V8BindingError::Allocation("selection context"));
        }
        let selection = new_stateful_object(scope, &state, "Selection", 1)?;
        let retained =
            context_intrinsics::retain_object_for(scope, &state, "selection", selection)?;
        state.selection_object.borrow_mut().replace(retained);
        Ok::<_, V8BindingError>(selection)
    })();
    match result {
        Ok(selection) => rv.set(selection.into()),
        Err(error) => throw_binding_error(scope, &error),
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;
    #[test]
    fn window_selection_is_document_selection_and_rejects_unrelated_receivers() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>selection</body>")
                .into_runtime()
                .unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(super::super::test_eval::eval(scope, r#"(()=>{
            const selection=getSelection();
            const detached=getSelection;
            if(detached.call(null)!==selection||detached.call(undefined)!==selection
                ||(function(){'use strict';return detached()})()!==selection)
                throw Error('Window receiver substitution');
            const descriptor=Object.getOwnPropertyDescriptor(window,'getSelection');
            if(!descriptor||Object.hasOwn(Window.prototype,'getSelection'))throw Error('global placement');
            if(selection!==window.getSelection()||selection!==document.getSelection()||selection!==descriptor.value.call(window))throw Error('selection identity');
            for(const receiver of [{},document,selection]){
                let rejected=false;try{getSelection.call(receiver)}catch(e){rejected=e instanceof TypeError}
                if(!rejected)throw Error('foreign receiver accepted');
            }
            return getSelection.length===0 && getSelection.name==='getSelection';
        })()"#).is_true());
    }
    #[test]
    fn borrowed_window_selection_uses_receiver_realm_and_checks_access_and_retirement() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let caller = v8::Context::new(scope, Default::default());
        let owner = v8::Context::new(scope, Default::default());
        owner.set_security_token(caller.get_security_token(scope));
        let mut states = Vec::new();
        for (index, context) in [caller, owner].into_iter().enumerate() {
            let scope = &mut v8::ContextScope::new(scope, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>selection</body>")
                    .into_runtime()
                    .unwrap(),
            ));
            states.push(
                install_generated_bindings(scope, context, dom, RealmId(index as u32), 1).unwrap(),
            );
        }
        let foreign = owner.global(scope);
        {
            let scope = &mut v8::ContextScope::new(scope, caller);
            let key = v8::String::new(scope, "foreignWindow").unwrap();
            caller.global(scope).set(scope, key.into(), foreign.into());
            assert!(super::super::test_eval::eval(scope, r#"(()=>{
                const selected=getSelection.call(foreignWindow);
                if(selected!==foreignWindow.document.getSelection()||selected===getSelection())throw Error('receiver selection');
                if(Object.getPrototypeOf(selected)!==foreignWindow.Selection.prototype)throw Error('receiver prototype');
                globalThis.borrowedSelection=foreignWindow.getSelection;
                return borrowedSelection.call(window)===getSelection();
            })()"#).is_true());
        }
        let cross_origin = v8::String::new(scope, "other-origin").unwrap();
        owner.set_security_token(cross_origin.into());
        {
            let scope = &mut v8::ContextScope::new(scope, caller);
            assert!(super::super::test_eval::eval(scope, r#"(()=>{
                try{getSelection.call(foreignWindow)}catch(error){return error.name==='SecurityError'}
                return false;
            })()"#).is_true());
        }
        owner.set_security_token(caller.get_security_token(scope));
        states[1].retire_execution();
        {
            let scope = &mut v8::ContextScope::new(scope, caller);
            assert!(
                super::super::test_eval::eval(scope, "getSelection.call(foreignWindow)===null")
                    .is_true()
            );
        }
    }
}
