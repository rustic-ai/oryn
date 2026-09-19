//! Live traversal of the canonical light tree, with callback state kept out of
//! DOM borrows. Algorithms: https://dom.spec.whatwg.org/#interface-treewalker
use super::*;

const ACCEPT: u16 = 1;
const REJECT: u16 = 2;
const SKIP: u16 = 3;

pub(super) struct Record {
    what_to_show: u32,
    active: Rc<Cell<bool>>,
    _wrapper: v8::Weak<v8::Object>,
}

fn invalid() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "TreeWalker".into(),
        member: "incompatible receiver".into(),
    }
}

fn key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let name =
        v8::String::new(scope, name).ok_or(V8BindingError::Allocation("TreeWalker private key"))?;
    Ok(v8::Private::for_api(scope, Some(name)))
}

fn record(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
) -> Result<Rc<Record>, V8BindingError> {
    if object.is_proxy() {
        return Err(invalid());
    }
    let key = key(scope, "oryn.treewalker.id")?;
    let id = object
        .get_private(scope, key)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map(|value| value.u64_value().0)
        .ok_or_else(invalid)?;
    state
        .tree_walkers
        .borrow()
        .get(&id)
        .cloned()
        .ok_or_else(invalid)
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    root_owner: &Rc<V8BindingState>,
    root: NodeId,
    what_to_show: u32,
    filter: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if !filter.is_null_or_undefined() && !filter.is_object() {
        return Err(V8BindingError::Unsupported {
            interface: "NodeFilter".into(),
            member: "filter must be an object".into(),
        });
    }
    let id = state.next_traversal_id.get();
    let next = id
        .checked_add(1)
        .filter(|next| *next <= 1 << 53)
        .ok_or(V8BindingError::Allocation("TreeWalker identifiers"))?;
    let object = new_stateful_object(scope, state, "TreeWalker", id)?;
    let id_key = key(scope, "oryn.treewalker.id")?;
    let id_value = v8::BigInt::new_from_u64(scope, id);
    object
        .set_private(scope, id_key, id_value.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    let root_object = wrap_node(scope, root_owner, root)?;
    for name in ["oryn.treewalker.root", "oryn.treewalker.current"] {
        let key = key(scope, name)?;
        object
            .set_private(scope, key, root_object.into())
            .ok_or(V8BindingError::JavaScriptException)?;
    }
    let filter_key = key(scope, "oryn.treewalker.filter")?;
    let filter = if filter.is_null_or_undefined() {
        v8::null(scope).into()
    } else {
        filter
    };
    object
        .set_private(scope, filter_key, filter)
        .ok_or(V8BindingError::JavaScriptException)?;
    let weak_state = Rc::downgrade(state);
    let wrapper = v8::Weak::with_finalizer(
        scope,
        object,
        Box::new(move |_| {
            if let Some(state) = weak_state.upgrade() {
                state.tree_walkers.borrow_mut().remove(&id);
            }
        }),
    );
    state.tree_walkers.borrow_mut().insert(
        id,
        Rc::new(Record {
            what_to_show,
            active: Rc::new(Cell::new(false)),
            _wrapper: wrapper,
        }),
    );
    state.next_traversal_id.set(next);
    Ok(object)
}

pub(super) fn attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let record = record(scope, state, object)?;
    let name = match member {
        "root" => "oryn.treewalker.root",
        "currentNode" => "oryn.treewalker.current",
        "filter" => "oryn.treewalker.filter",
        "whatToShow" => {
            return Ok(v8::Integer::new_from_unsigned(scope, record.what_to_show).into());
        }
        _ => return Err(invalid()),
    };
    let key = key(scope, name)?;
    object
        .get_private(scope, key)
        .ok_or(V8BindingError::JavaScriptException)
}

pub(super) fn set_current(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    record(scope, state, object)?;
    node_identity::argument(scope, value)?.ok_or_else(invalid)?;
    let key = key(scope, "oryn.treewalker.current")?;
    object
        .set_private(scope, key, value)
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok(())
}

// Keep the original wrapper across author callbacks. Adoption rewrites its
// native identity; no arena-local ID may survive an author-code boundary.
fn edge<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    node: v8::Local<'s, v8::Object>,
    direction: &str,
    reverse: bool,
) -> Result<Option<v8::Local<'s, v8::Object>>, V8BindingError> {
    let (owner, id) = node_identity::argument(scope, node.into())?.ok_or_else(invalid)?;
    let next = {
        let dom = owner.dom.borrow();
        let parent = if matches!(
            dom.arena().get(id)?.kind,
            NodeKind::Document | NodeKind::ShadowRoot { .. }
        ) {
            None
        } else {
            dom.parent(id)?
        };
        match direction {
            "parent" => parent,
            "child" => {
                let nodes = light_children(&dom, id)?;
                if reverse { nodes.last() } else { nodes.first() }.copied()
            }
            "sibling" => {
                if let Some(parent) = parent {
                    let nodes = light_children(&dom, parent)?;
                    nodes
                        .iter()
                        .position(|candidate| *candidate == id)
                        .and_then(|index| {
                            if reverse {
                                index.checked_sub(1)
                            } else {
                                index.checked_add(1)
                            }
                        })
                        .and_then(|index| nodes.get(index).copied())
                } else {
                    None
                }
            }
            _ => return Err(invalid()),
        }
    };
    next.map(|next| wrap_node(scope, &owner, next)).transpose()
}

fn filter(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    record: &Record,
    node: v8::Local<v8::Object>,
) -> Result<u16, V8BindingError> {
    if record.active.get() {
        return Err(V8BindingError::Unsupported {
            interface: "TreeWalker".into(),
            member: "InvalidStateError".into(),
        });
    }
    let (owner, id) = node_identity::argument(scope, node.into())?.ok_or_else(invalid)?;
    let node_type = traversal_node_type(&owner.dom.borrow(), id)?;
    if record.what_to_show & (1 << (node_type - 1)) == 0 {
        return Ok(SKIP);
    }
    let filter_key = key(scope, "oryn.treewalker.filter")?;
    let callback = object
        .get_private(scope, filter_key)
        .ok_or(V8BindingError::JavaScriptException)?;
    if callback.is_null() {
        return Ok(ACCEPT);
    }
    record.active.set(true);
    let result = (|| {
        let callback_object = v8::Local::<v8::Object>::try_from(callback).map_err(|_| invalid())?;
        let argument = node;
        let context = callback_object
            .get_creation_context(scope)
            .ok_or(V8BindingError::Allocation("NodeFilter callback realm"))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let (function, receiver) = if let Ok(function) =
            v8::Local::<v8::Function>::try_from(callback)
        {
            (function, v8::undefined(scope).into())
        } else {
            let receiver = v8::Local::<v8::Object>::try_from(callback).map_err(|_| invalid())?;
            let name = v8::String::new(scope, "acceptNode")
                .ok_or(V8BindingError::Allocation("NodeFilter operation"))?;
            let value = receiver
                .get(scope, name.into())
                .ok_or(V8BindingError::JavaScriptException)?;
            let function = v8::Local::<v8::Function>::try_from(value).map_err(|_| {
                throw_type_error(scope, "NodeFilter.acceptNode must be callable");
                V8BindingError::JavaScriptException
            })?;
            (function, receiver.into())
        };
        let value = function
            .call(scope, receiver, &[argument.into()])
            .ok_or(V8BindingError::JavaScriptException)?;
        Ok(value
            .uint32_value(scope)
            .ok_or(V8BindingError::JavaScriptException)? as u16)
    })();
    record.active.set(false);
    result
}

pub(super) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<'s, v8::Object>,
    member: &str,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let record = record(scope, state, object)?;
    let current_key = key(scope, "oryn.treewalker.current")?;
    let current = object
        .get_private(scope, current_key)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(invalid)?;
    let root_key = key(scope, "oryn.treewalker.root")?;
    let root = object
        .get_private(scope, root_key)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(invalid)?;
    let Some(value) = walk(scope, object, &record, root, current, member)? else {
        return Ok(Some(v8::null(scope).into()));
    };
    object
        .set_private(scope, current_key, value.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok(Some(value.into()))
}

fn walk<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    record: &Record,
    root: v8::Local<'s, v8::Object>,
    start: v8::Local<'s, v8::Object>,
    member: &str,
) -> Result<Option<v8::Local<'s, v8::Object>>, V8BindingError> {
    let mut node = start;
    match member {
        "parentNode" => {
            while !node.strict_equals(root.into()) {
                let Some(next) = edge(scope, node, "parent", false)? else {
                    break;
                };
                node = next;
                if filter(scope, object, record, node)? == ACCEPT {
                    return Ok(Some(node));
                }
            }
        }
        "firstChild" | "lastChild" => {
            let reverse = member == "lastChild";
            let Some(first) = edge(scope, node, "child", reverse)? else {
                return Ok(None);
            };
            node = first;
            loop {
                match filter(scope, object, record, node)? {
                    ACCEPT => return Ok(Some(node)),
                    SKIP => {
                        if let Some(next) = edge(scope, node, "child", reverse)? {
                            node = next;
                            continue;
                        }
                    }
                    _ => {}
                }
                loop {
                    if let Some(next) = edge(scope, node, "sibling", reverse)? {
                        node = next;
                        break;
                    }
                    let Some(next) = edge(scope, node, "parent", false)? else {
                        return Ok(None);
                    };
                    if next.strict_equals(root.into()) || next.strict_equals(start.into()) {
                        return Ok(None);
                    }
                    node = next;
                }
            }
        }
        "nextSibling" | "previousSibling" => {
            if node.strict_equals(root.into()) {
                return Ok(None);
            }
            let reverse = member == "previousSibling";
            loop {
                let mut next = edge(scope, node, "sibling", reverse)?;
                while let Some(candidate) = next {
                    node = candidate;
                    let result = filter(scope, object, record, node)?;
                    if result == ACCEPT {
                        return Ok(Some(node));
                    }
                    next = edge(scope, node, "child", reverse)?;
                    if result == REJECT || next.is_none() {
                        next = edge(scope, node, "sibling", reverse)?;
                    }
                }
                let Some(next) = edge(scope, node, "parent", false)? else {
                    return Ok(None);
                };
                node = next;
                if node.strict_equals(root.into()) || filter(scope, object, record, node)? == ACCEPT
                {
                    return Ok(None);
                }
            }
        }
        "previousNode" => {
            while !node.strict_equals(root.into()) {
                let mut next = edge(scope, node, "sibling", true)?;
                while let Some(candidate) = next {
                    node = candidate;
                    let mut result = filter(scope, object, record, node)?;
                    while result != REJECT {
                        let Some(next) = edge(scope, node, "child", true)? else {
                            break;
                        };
                        node = next;
                        result = filter(scope, object, record, node)?;
                    }
                    if result == ACCEPT {
                        return Ok(Some(node));
                    }
                    next = edge(scope, node, "sibling", true)?;
                }
                if node.strict_equals(root.into()) {
                    return Ok(None);
                }
                let Some(next) = edge(scope, node, "parent", false)? else {
                    return Ok(None);
                };
                node = next;
                if filter(scope, object, record, node)? == ACCEPT {
                    return Ok(Some(node));
                }
            }
        }
        "nextNode" => {
            let mut result = ACCEPT;
            loop {
                while result != REJECT {
                    let Some(next) = edge(scope, node, "child", false)? else {
                        break;
                    };
                    node = next;
                    result = filter(scope, object, record, node)?;
                    if result == ACCEPT {
                        return Ok(Some(node));
                    }
                }
                let mut ancestor = node;
                node = loop {
                    if ancestor.strict_equals(root.into()) {
                        return Ok(None);
                    }
                    if let Some(next) = edge(scope, ancestor, "sibling", false)? {
                        break next;
                    }
                    let Some(next) = edge(scope, ancestor, "parent", false)? else {
                        return Ok(None);
                    };
                    ancestor = next;
                };
                result = filter(scope, object, record, node)?;
                if result == ACCEPT {
                    return Ok(Some(node));
                }
            }
        }
        _ => return Err(invalid()),
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_exhaustion_preserves_existing_walker_and_counter() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(DomRuntime::new()));
        let state = install_generated_bindings(scope, context, dom, RealmId(93), 1).unwrap();
        let root = state.document_root();
        state.next_traversal_id.set((1 << 53) - 1);
        let filter = v8::null(scope).into();
        let walker = create(scope, &state, &state, root, u32::MAX, filter).unwrap();
        assert_eq!(state.next_traversal_id.get(), 1 << 53);
        for exhausted in [1 << 53, u64::MAX] {
            state.next_traversal_id.set(exhausted);
            let filter = v8::null(scope).into();
            assert!(matches!(
                create(scope, &state, &state, root, 0, filter),
                Err(V8BindingError::Allocation("TreeWalker identifiers"))
            ));
            assert_eq!(state.next_traversal_id.get(), exhausted);
            assert_eq!(state.tree_walkers.borrow().len(), 1);
            assert_eq!(
                record(scope, &state, walker).unwrap().what_to_show,
                u32::MAX
            );
        }
    }

    #[test]
    fn callback_migration_preserves_every_traversal_cursor_and_original_wrapper() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let source_dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let target_dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div><p></p></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _source = {
            let scope = &mut v8::ContextScope::new(scope, first);
            install_generated_bindings(scope, first, source_dom, RealmId(91), 1).unwrap()
        };
        let target = {
            let scope = &mut v8::ContextScope::new(scope, second);
            install_generated_bindings(scope, second, target_dom, RealmId(92), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let foreign = wrap_node(scope, &target, target.document.get()).unwrap();
        let name = v8::String::new(scope, "foreignDocument").unwrap();
        first
            .global(scope)
            .set(scope, name.into(), foreign.into())
            .unwrap();
        let script = v8::String::new(scope, r#"
          function fixture() {
            const root=document.createElement('div');
            root.innerHTML='<section id="a"><i id="aa"></i></section><section id="b"><i id="bb"></i></section><section id="c"></section>';
            return {root,a:root.children[0],aa:root.children[0].firstChild,b:root.children[1],bb:root.children[1].firstChild,c:root.children[2]};
          }
          for (const phase of ['getter','callback','conversion']) {
            for (const [method,start,expected] of [
              ['parentNode','aa','a'], ['firstChild','root','a'], ['lastChild','root','c'],
              ['nextSibling','a','b'], ['previousSibling','c','b'],
              ['nextNode','a','aa'], ['previousNode','b','aa']
            ]) {
              const f=fixture(); let moves=0;
              function move() { if (!moves++) foreignDocument.adoptNode(f.root); }
              const filter={get acceptNode() {
                if(phase==='getter')move();
                return function(node) {
                  if(phase==='callback')move();
                  return {valueOf(){if(phase==='conversion')move();return 1}};
                };
              }};
              const walker=document.createTreeWalker(f.root,1,filter),proto=Object.getPrototypeOf(walker);
              walker.currentNode=f[start];
              if(walker[method]()!==f[expected]||walker.currentNode!==f[expected])throw Error(phase+' '+method+' identity');
              if(walker.root!==f.root||walker.filter!==filter||Object.getPrototypeOf(walker)!==proto)throw Error('walker identity');
              document.adoptNode(f.root);
              if(walker.currentNode.ownerDocument!==document)throw Error('return migration');
            }
          }
          // SKIP must follow the adopted candidate's new descendants even when
          // root and original current node remain in a different arena.
          for(const method of ['firstChild','nextNode','nextSibling']) {
            const f=fixture(); let once=false;
            const walker=document.createTreeWalker(f.root,1,node=>{
              if(!once){once=true;foreignDocument.adoptNode(node);return 3}return 1;
            });
            if(method==='nextSibling')walker.currentNode=f.a;
            const expected=method==='nextSibling'?f.bb:f.aa;
            if(walker[method]()!==expected||walker.root!==f.root)throw Error('split arena '+method);
          }
          // REJECT must continue at the destination sibling, not a reused
          // source ID; both the root boundary and cursor can move repeatedly.
          {
            const f=fixture();let calls=0;
            const walker=document.createTreeWalker(f.root,1,node=>{
              if(!calls++){foreignDocument.adoptNode(f.root);return 2}
              document.adoptNode(f.root);return 1;
            });
            if(walker.nextNode()!==f.b||walker.currentNode!==f.b)throw Error('reject repeated migration');
          }
          // A failed callback does not publish its temporary candidate. The
          // active flag also covers getter and return-value conversion.
          for(const phase of ['getter','conversion']) {
            const f=fixture(),sentinel={};let walker,fail=true;
            function interrupt(){
              foreignDocument.adoptNode(f.root);
              try{walker.nextNode();throw Error('reentry accepted')}catch(e){if(e.name!=='InvalidStateError')throw e}
              throw sentinel;
            }
            walker=document.createTreeWalker(f.root,1,{get acceptNode(){
              if(fail&&phase==='getter')interrupt();
              return ()=>({valueOf(){if(fail&&phase==='conversion')interrupt();return 1}});
            }});
            try{walker.firstChild();throw Error('exception swallowed')}catch(e){if(e!==sentinel)throw e}
            if(walker.currentNode!==f.root)throw Error('failed cursor publication');
            fail=false;if(walker.firstChild()!==f.a)throw Error('active flag not cleared');
          }
        "#).unwrap();
        v8::Script::compile(scope, script, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
}
