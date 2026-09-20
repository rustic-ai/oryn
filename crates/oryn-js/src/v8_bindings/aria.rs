//! ARIA reflection and ElementInternals defaults use separate native stores.
type RelationArrays = BTreeMap<(NodeId, String, String), (Vec<NodeId>, v8::Weak<v8::Array>)>;

use super::*;
#[derive(Default)]
pub(super) struct Registry {
    pub(super) migrated: RefCell<Vec<v8::Weak<v8::Object>>>,
    arrays: RefCell<RelationArrays>,
}
pub(super) fn collect(state: &V8BindingState) {
    state
        .aria
        .arrays
        .borrow_mut()
        .retain(|(node, _, _), _| state.dom.borrow().arena().get(*node).is_ok());
}
pub(super) fn attribute(member: &str) -> Option<&'static str> {
    Some(match member {
        "role" => "role",
        "ariaAtomic" => "aria-atomic",
        "ariaAutoComplete" => "aria-autocomplete",
        "ariaBrailleLabel" => "aria-braillelabel",
        "ariaBrailleRoleDescription" => "aria-brailleroledescription",
        "ariaBusy" => "aria-busy",
        "ariaChecked" => "aria-checked",
        "ariaColCount" => "aria-colcount",
        "ariaColIndex" => "aria-colindex",
        "ariaColIndexText" => "aria-colindextext",
        "ariaColSpan" => "aria-colspan",
        "ariaCurrent" => "aria-current",
        "ariaDescription" => "aria-description",
        "ariaDisabled" => "aria-disabled",
        "ariaExpanded" => "aria-expanded",
        "ariaHasPopup" => "aria-haspopup",
        "ariaHidden" => "aria-hidden",
        "ariaInvalid" => "aria-invalid",
        "ariaKeyShortcuts" => "aria-keyshortcuts",
        "ariaLabel" => "aria-label",
        "ariaLevel" => "aria-level",
        "ariaLive" => "aria-live",
        "ariaModal" => "aria-modal",
        "ariaMultiLine" => "aria-multiline",
        "ariaMultiSelectable" => "aria-multiselectable",
        "ariaOrientation" => "aria-orientation",
        "ariaPlaceholder" => "aria-placeholder",
        "ariaPosInSet" => "aria-posinset",
        "ariaPressed" => "aria-pressed",
        "ariaReadOnly" => "aria-readonly",
        "ariaRelevant" => "aria-relevant",
        "ariaRequired" => "aria-required",
        "ariaRoleDescription" => "aria-roledescription",
        "ariaRowCount" => "aria-rowcount",
        "ariaRowIndex" => "aria-rowindex",
        "ariaRowIndexText" => "aria-rowindextext",
        "ariaRowSpan" => "aria-rowspan",
        "ariaSelected" => "aria-selected",
        "ariaSetSize" => "aria-setsize",
        "ariaSort" => "aria-sort",
        "ariaValueMax" => "aria-valuemax",
        "ariaValueMin" => "aria-valuemin",
        "ariaValueNow" => "aria-valuenow",
        "ariaValueText" => "aria-valuetext",
        _ => return None,
    })
}
pub(super) fn handles(interface: &str, member: &str) -> bool {
    matches!(interface, "Element" | "ElementInternals")
        && (attribute(member).is_some() || relation(member).is_some())
}
pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if relation(member).is_some() {
        return get_relation(scope, state, node, interface, member);
    }
    let name = attribute(member).unwrap();
    let dom = state.dom.borrow();
    let node = dom.arena().get(node)?;
    let value = if interface == "ElementInternals" {
        node.default_accessibility_attribute(name)
    } else {
        node.attributes.get_null(name)
    };
    match value {
        Some(value) => v8_string(scope, value),
        None => Ok(v8::null(scope).into()),
    }
}
pub(super) fn set(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    if relation(member).is_some() {
        return set_relation(scope, state, node, interface, member, value);
    }
    let original = wrap_node(scope, state, node)?;
    let name = attribute(member).unwrap();
    let value = if value.is_null_or_undefined() {
        None
    } else {
        let value = value
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        Some(rust_string_preserving_utf16(scope, value)?)
    };
    let (owner, node) =
        node_identity::argument(scope, original.into())?.ok_or(DomError::StaleNode(node))?;
    let state = &owner;
    if interface == "ElementInternals" {
        let changed = state
            .dom
            .borrow()
            .arena()
            .get(node)?
            .default_accessibility_attribute(name)
            .map(String::as_str)
            != value.as_deref();
        state
            .dom
            .borrow_mut()
            .set_default_accessibility_attribute(node, name, value)?;
        if changed {
            custom_elements::mark_internals_changed(state, node);
        }
    } else if let Some(value) = value {
        state.dom.borrow_mut().set_attribute(node, name, value)?;
    } else {
        state.dom.borrow_mut().remove_attribute(node, name)?;
    }
    Ok(())
}

pub(super) fn relation(member: &str) -> Option<(&'static str, bool)> {
    Some(match member {
        "ariaActionsElements" => ("aria-actions", true),
        "ariaActiveDescendantElement" => ("aria-activedescendant", false),
        "ariaControlsElements" => ("aria-controls", true),
        "ariaDescribedByElements" => ("aria-describedby", true),
        "ariaDetailsElements" => ("aria-details", true),
        "ariaErrorMessageElements" => ("aria-errormessage", true),
        "ariaFlowToElements" => ("aria-flowto", true),
        "ariaLabelledByElements" => ("aria-labelledby", true),
        "ariaOwnsElements" => ("aria-owns", true),
        _ => return None,
    })
}
fn relation_error() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "ARIA".into(),
        member: "TypeError: expected Element reference".into(),
    }
}
fn element(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<v8::Global<v8::Object>, V8BindingError> {
    let (owner, node) = node_identity::argument(scope, value)?.ok_or_else(relation_error)?;
    if !matches!(
        owner.dom.borrow().arena().get(node)?.kind,
        NodeKind::Element { .. }
    ) {
        return Err(relation_error());
    }
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| relation_error())?;
    Ok(v8::Global::new(scope, object))
}
fn elements(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<v8::Global<v8::Object>>, V8BindingError> {
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| relation_error())?;
    let key = v8::Symbol::get_iterator(scope);
    let method = object
        .get(scope, key.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    let method = v8::Local::<v8::Function>::try_from(method).map_err(|_| relation_error())?;
    let mut result: Vec<v8::Global<v8::Object>> = Vec::new();
    iterable_conversion::consume(scope, object, method, |scope, value| {
        let retained = element(scope, value)?;
        let object = v8::Local::new(scope, &retained);
        if !result
            .iter()
            .any(|prior| v8::Local::new(scope, prior).strict_equals(object.into()))
        {
            result
                .try_reserve(1)
                .map_err(|_| V8BindingError::Allocation("ARIA element references"))?;
            result.push(retained);
        }
        Ok(())
    })?;
    Ok(result)
}
pub(super) fn get_relation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    reconcile_node(scope, state, node)?;
    let (name, array) = relation(member).unwrap();
    let internals = interface == "ElementInternals";
    let (targets, _revision) = {
        let dom = state.dom.borrow();
        let record = dom.arena().get(node)?;
        let targets = if internals {
            record
                .default_accessibility_references(name)
                .map(<[NodeId]>::to_vec)
        } else if record.explicit_accessibility_references(name).is_some()
            || record.attributes.contains_null(name)
        {
            dom.arena().accessibility_relation(node, name)
        } else {
            None
        };
        (targets, record.accessibility_reference_revision())
    };
    let Some(targets) = targets else {
        return Ok(v8::null(scope).into());
    };
    if !array {
        return match targets.first() {
            Some(node) => Ok(wrap_node(scope, state, *node)?.into()),
            None => Ok(v8::null(scope).into()),
        };
    }
    let cache_key = (node, interface.to_owned(), member.to_owned());
    if let Some((old, weak)) = state.aria.arrays.borrow().get(&cache_key)
        && old == &targets
        && let Some(array) = weak.to_local(scope)
    {
        return Ok(array.into());
    }
    let mut values = Vec::new();
    for target in &targets {
        values.push(wrap_node(scope, state, *target)?.into());
    }
    let array = v8::Array::new_with_elements(scope, &values);
    array
        .set_integrity_level(scope, v8::IntegrityLevel::Frozen)
        .ok_or(V8BindingError::JavaScriptException)?;
    state
        .aria
        .arrays
        .borrow_mut()
        .insert(cache_key, (targets, v8::Weak::new(scope, array)));
    Ok(array.into())
}
pub(super) fn set_relation(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let original = wrap_node(scope, state, node)?;
    let (name, array) = relation(member).unwrap();
    let targets = if value.is_null_or_undefined() {
        None
    } else if array {
        Some(elements(scope, value)?)
    } else {
        Some(vec![element(scope, value)?])
    };
    // Conversion may adopt the host, any earlier yielded element, or both.
    // Keep the original wrappers as the association; native IDs are only the
    // current same-arena projection used by layout/accessibility.
    let (owner, node) =
        node_identity::argument(scope, original.into())?.ok_or(DomError::StaleNode(node))?;
    let state = &owner;
    let mut projected = Vec::new();
    let mut values = Vec::new();
    for target in targets.iter().flatten() {
        let object = v8::Local::new(scope, target);
        let (target_owner, target) =
            node_identity::argument(scope, object.into())?.ok_or_else(relation_error)?;
        if Rc::ptr_eq(&target_owner.dom, &state.dom) {
            projected.push(target);
        }
        values.push(object.into());
    }
    let internals = interface == "ElementInternals";
    let key = migration_key(scope, name, !internals)?;
    let prior = original
        .get_private(scope, key)
        .ok_or(V8BindingError::JavaScriptException)?;
    let array = targets
        .as_ref()
        .map(|_| v8::Array::new_with_elements(scope, &values));
    let changed = if let Some(array) = array {
        let old = prior.try_cast::<v8::Array>().ok();
        old.is_none_or(|old| {
            old.length() != array.length()
                || (0..array.length()).any(|index| {
                    let a = old.get_index(scope, index);
                    let b = array.get_index(scope, index);
                    !matches!((a,b),(Some(a),Some(b))if a.strict_equals(b))
                })
        })
    } else {
        !prior.is_undefined()
    };
    let tracked = state.aria.migrated.borrow().iter().any(|weak| {
        weak.to_local(scope)
            .is_some_and(|object| object.strict_equals(original.into()))
    });
    if targets.is_some() && !tracked {
        state
            .aria
            .migrated
            .borrow_mut()
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("ARIA reference owner"))?;
    }
    let value: v8::Local<v8::Value> = array.map_or_else(|| v8::undefined(scope).into(), Into::into);
    if original.set_private(scope, key, value) != Some(true) {
        return Err(V8BindingError::JavaScriptException);
    }
    let projected = targets.as_ref().map(|_| projected);
    let (native_changed, prior_revision) = {
        let dom = state.dom.borrow();
        let record = dom.arena().get(node)?;
        let current = if internals {
            record.default_accessibility_references(name)
        } else {
            record.explicit_accessibility_references(name)
        };
        (
            current != projected.as_deref(),
            record.accessibility_reference_revision(),
        )
    };
    let result = state.dom.borrow_mut().set_accessibility_references(
        node,
        name,
        internals,
        projected.clone(),
    );
    if let Err(error) = result {
        let committed = {
            let dom = state.dom.borrow();
            let record = dom.arena().get(node)?;
            let current = if internals {
                record.default_accessibility_references(name)
            } else {
                record.explicit_accessibility_references(name)
            };
            current == projected.as_deref()
                && record.accessibility_reference_revision() != prior_revision
        };
        if !committed {
            original.set_private(scope, key, prior);
        } else {
            if targets.is_some() && !tracked {
                state
                    .aria
                    .migrated
                    .borrow_mut()
                    .push(v8::Weak::new(scope, original));
            }
            if internals && (changed || native_changed) {
                custom_elements::mark_internals_changed(state, node);
            }
            state
                .aria
                .arrays
                .borrow_mut()
                .remove(&(node, interface.to_owned(), member.to_owned()));
        }
        return Err(error.into());
    }
    if targets.is_none() {
        original.delete_private(scope, key);
    } else if !tracked {
        state
            .aria
            .migrated
            .borrow_mut()
            .push(v8::Weak::new(scope, original));
    }
    if internals && (changed || native_changed) {
        custom_elements::mark_internals_changed(state, node);
    }
    state
        .aria
        .arrays
        .borrow_mut()
        .remove(&(node, interface.to_owned(), member.to_owned()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, code: &str) -> v8::Local<'s, v8::Value> {
        super::super::test_eval::eval(scope, code)
    }
    #[test]
    fn internals_scalar_and_custom_state_conversion_follow_adopted_host() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let foreign = v8::Context::new(scope, Default::default());
        let (_foreign_state, document) = {
            let scope = &mut v8::ContextScope::new(scope, foreign);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(scope, foreign, dom, RealmId(81), 1).unwrap();
            let document = eval(scope, "document");
            (state, v8::Global::new(scope, document))
        };
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><x-moving-internals id=x></x-moving-internals></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(82), 1).unwrap();
        let foreign_document = v8::Local::new(scope, &document);
        let key = v8::String::new(scope, "foreignDocument").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), foreign_document);
        assert!(eval(scope,r#"(()=>{
            const check=(value,label)=>{if(!value)throw Error(label)};
            customElements.define('x-moving-internals',class extends HTMLElement{constructor(){super();this.i=this.attachInternals()}});
            const host=document.getElementById('x'),states=host.i.states;
            states.add({toString(){foreignDocument.adoptNode(host);foreignDocument.body.append(host);return 'moved'}});
            check(host.ownerDocument===foreignDocument&&states.has('moved'),'adopted add');
            check(states.has({toString(){document.adoptNode(host);document.body.append(host);return 'moved'}}),'adopted has');
            check(states.delete({toString(){foreignDocument.adoptNode(host);return 'moved'}})&&!states.has('moved'),'adopted delete');
            document.adoptNode(host);document.body.append(host);
            host.i.ariaLabel={toString(){foreignDocument.adoptNode(host);return 'moved label'}};
            check(host.ownerDocument===foreignDocument&&host.i.ariaLabel==='moved label'&&!host.hasAttribute('aria-label'),'adopted internals scalar');
            document.adoptNode(host);document.body.append(host);
            const foreignLabel=foreignDocument.createElement('span');foreignDocument.body.append(foreignLabel);
            host.ariaActiveDescendantElement=foreignLabel;
            check(host.ariaActiveDescendantElement===null,'foreign assignment accepted but out of scope');
            document.body.append(document.adoptNode(foreignLabel));
            check(host.ariaActiveDescendantElement===foreignLabel,'foreign reference follows original target');
            const localLabel=document.createElement('span');document.body.append(localLabel);
            foreignDocument.body.append(foreignDocument.adoptNode(foreignLabel));
            host.ariaControlsElements={*[Symbol.iterator](){yield localLabel;foreignDocument.body.append(foreignDocument.adoptNode(host));yield foreignLabel}};
            check(host.ariaControlsElements.length===1&&host.ariaControlsElements[0]===foreignLabel,'iterator moves receiver after earlier target');
            foreignDocument.body.append(foreignDocument.adoptNode(localLabel));
            check(host.ariaControlsElements.length===2&&host.ariaControlsElements[0]===localLabel&&host.ariaControlsElements[1]===foreignLabel,'retained cross-arena target becomes visible');
            let closed=false;try{host.ariaControlsElements={*[Symbol.iterator](){try{yield foreignLabel;yield {}}finally{closed=true}}}}catch{}
            check(closed&&host.ariaControlsElements.length===2&&host.ariaControlsElements[0]===localLabel,'failed conversion preserves prior association');

            return true;
        })()"#).is_true());
    }
    #[test]
    fn internals_defaults_reflect_without_attributes_and_relations_keep_identity() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<x-aria id=x></x-aria><span id=label>Label</span><span id=other>Other</span>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};customElements.define('x-aria',class extends HTMLElement{constructor(){super();this.i=this.attachInternals()}});
            const x=document.getElementById('x'),i=x.i,label=document.getElementById('label'),other=document.getElementById('other');
            check(i.role===null&&i.ariaLabel===null&&i.ariaControlsElements===null,'unset');i.role='button';i.ariaLabel='Default';check(x.getAttribute('role')===null&&x.getAttribute('aria-label')===null,'not authored');
            x.ariaLabel='Author';check(i.ariaLabel==='Default'&&x.getAttribute('aria-label')==='Author','separate stores');x.ariaLabel=null;check(x.getAttribute('aria-label')===null&&i.ariaLabel==='Default','remove override');
            i.ariaLabelledByElements=[label];const first=i.ariaLabelledByElements;check(Object.isFrozen(first)&&first===i.ariaLabelledByElements&&first[0]===label,'frozen identity');i.ariaControlsElements=[other];check(first===i.ariaLabelledByElements,'independent cache');
            x.ariaLabelledByElements=[other];check(x.getAttribute('aria-labelledby')===''&&x.ariaLabelledByElements[0]===other&&i.ariaLabelledByElements[0]===label,'explicit override');
            x.setAttribute('aria-labelledby','label');check(x.ariaLabelledByElements[0]===label,'attribute invalidates explicit reference');
            let closed=false;const bad={*[Symbol.iterator](){try{yield label;yield {}}finally{closed=true}}};let threw=false,caught='no exception';try{i.ariaControlsElements=bad}catch(e){caught=String(e)+' / '+e?.name;threw=e?.name==='TypeError'}check(threw,'invalid ARIA element throws TypeError: '+caught+'; closed='+closed);check(closed,'invalid ARIA element closes iterator');check(i.ariaControlsElements[0]===other,'failed ARIA conversion preserves previous reference');
            const original={};let conversionError;i.ariaControlsElements=[other];try{i.ariaControlsElements={*[Symbol.iterator](){try{yield label;throw original}finally{}}}}catch(error){conversionError=error}check(conversionError===original&&i.ariaControlsElements[0]===other,'sequence exception survives setter checkpoint');
            i.ariaLabelledByElements=null;check(i.ariaLabelledByElements===null&&first[0]===label,'saved array detached');
            check(x.ariaActionsElements===null&&i.ariaActionsElements===null,'action defaults');i.ariaActionsElements=[label,other];check(i.ariaActionsElements.length===2&&i.ariaActionsElements[0]===label,'internals action order');
            x.setAttribute('aria-actions','label');check(x.ariaActionsElements[0]===label,'action content reflection');x.ariaActionsElements=[other];check(x.getAttribute('aria-actions')===''&&x.ariaActionsElements[0]===other,'action override');
            const shadowHost=document.createElement('div');document.body.append(shadowHost);const root=shadowHost.attachShadow({mode:'open'}),inside=document.createElement('button');root.append(inside);x.ariaActionsElements=[inside];check(x.ariaActionsElements.length===0,'foreign shadow excluded');x.ariaActionsElements=null;check(x.ariaActionsElements===null,'action null');return true;
        })()"#).is_true());
    }
}

const RELATIONS: [&str; 11] = [
    "interestfor",
    "commandfor",
    "aria-actions",
    "aria-activedescendant",
    "aria-controls",
    "aria-describedby",
    "aria-details",
    "aria-errormessage",
    "aria-flowto",
    "aria-labelledby",
    "aria-owns",
];
pub(super) fn migration_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
    explicit: bool,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let name = v8::String::new(scope, &format!("oryn.aria.migrated.{}.{}", explicit, name))
        .ok_or(V8BindingError::Allocation("ARIA migration key"))?;
    Ok(v8::Private::for_api(scope, Some(name)))
}
pub(super) struct Migration {
    owners: Vec<v8::Global<v8::Object>>,
}
pub(super) fn capture_migration(
    scope: &mut v8::PinScope,
    states: &[Rc<V8BindingState>],
    source: &SharedDom,
    nodes: &BTreeSet<NodeId>,
) -> Result<Migration, V8BindingError> {
    let mut owners = Vec::new();
    let mut seen = BTreeSet::new();
    for state in states {
        // Previously migrated relations may span several arenas. Their private
        // arrays retain the original ordered Node objects, not numeric IDs.
        for weak in state.aria.migrated.borrow().iter() {
            if let Some(object) = weak.to_local(scope) {
                owners.push(v8::Global::new(scope, object));
            }
        }
        if !Rc::ptr_eq(&state.dom, source) {
            continue;
        }
        let records = {
            let dom = source.borrow();
            let mut records = Vec::new();
            for (owner, record) in dom.arena().iter() {
                for name in RELATIONS {
                    for explicit in [false, true] {
                        let refs = if explicit {
                            record.explicit_accessibility_references(name)
                        } else {
                            record.default_accessibility_references(name)
                        };
                        if let Some(refs) = refs
                            && (nodes.contains(&owner)
                                || refs.iter().any(|node| nodes.contains(node)))
                        {
                            records.push((owner, name, explicit, refs.to_vec()));
                        }
                    }
                }
            }
            records
        };
        for (owner, name, explicit, refs) in records {
            let object = wrap_node(scope, state, owner)?;
            let key = migration_key(scope, name, explicit)?;
            if !object.has_private(scope, key).unwrap_or(false) {
                let mut values = Vec::new();
                for node in refs {
                    values.push(wrap_node(scope, state, node)?.into());
                }
                let values = v8::Array::new_with_elements(scope, &values);
                object
                    .set_private(scope, key, values.into())
                    .filter(|set| *set)
                    .ok_or(V8BindingError::JavaScriptException)?;
            }
            if seen.insert((state.dom.borrow().arena().id(), owner)) {
                owners.push(v8::Global::new(scope, object));
            }
        }
    }
    Ok(Migration { owners })
}
impl Migration {
    pub(super) fn publish(&mut self, scope: &mut v8::PinScope) -> Result<(), V8BindingError> {
        for owner in &self.owners {
            let object = v8::Local::new(scope, owner);
            let Some(state) = binding_state_for_receiver(scope, object) else {
                continue;
            };
            let node = argument_node(scope, &state, object.into())?;
            reconcile_node(scope, &state, node)?;
            let mut tracked = state.aria.migrated.borrow_mut();
            if !tracked.iter().any(|weak| {
                weak.to_local(scope)
                    .is_some_and(|prior| prior.strict_equals(object.into()))
            }) {
                tracked.push(v8::Weak::new(scope, object));
            }
        }
        Ok(())
    }
}
pub(super) fn reconcile_node(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<(), V8BindingError> {
    let object = wrap_node(scope, state, node)?;
    for name in RELATIONS {
        for explicit in [false, true] {
            let key = migration_key(scope, name, explicit)?;
            let Some(values) = object
                .get_private(scope, key)
                .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
            else {
                continue;
            };
            let current = {
                let dom = state.dom.borrow();
                let record = dom.arena().get(node)?;
                if explicit {
                    record
                        .explicit_accessibility_references(name)
                        .map(<[NodeId]>::to_vec)
                } else {
                    record
                        .default_accessibility_references(name)
                        .map(<[NodeId]>::to_vec)
                }
            };
            // Author attribute replacement clears the explicit native association.
            if current.is_none() {
                object.delete_private(scope, key);
                continue;
            }
            let mut projected = Vec::new();
            for index in 0..values.length() {
                let value = values
                    .get_index(scope, index)
                    .ok_or(V8BindingError::JavaScriptException)?;
                if let Some((owner, target)) = node_identity::argument(scope, value)?
                    && Rc::ptr_eq(&owner.dom, &state.dom)
                {
                    projected.push(target);
                }
            }
            if current.as_ref() != Some(&projected) {
                state.dom.borrow_mut().set_accessibility_references(
                    node,
                    name,
                    !explicit,
                    Some(projected),
                )?;
            }
        }
    }
    Ok(())
}
pub(super) fn reconcile_agent(scope: &mut v8::PinScope) -> Result<(), V8BindingError> {
    let states = scope
        .get_slot::<BindingStateRegistry>()
        .map(|registry| {
            registry
                .instances
                .borrow()
                .values()
                .filter_map(RcWeak::upgrade)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for state in states {
        let objects = state
            .aria
            .migrated
            .borrow()
            .iter()
            .filter_map(|weak| weak.to_local(scope))
            .map(|object| v8::Global::new(scope, object))
            .collect::<Vec<_>>();
        for object in objects {
            let object = v8::Local::new(scope, &object);
            if let Some(owner) = binding_state_for_receiver(scope, object) {
                let node = argument_node(scope, &owner, object.into())?;
                reconcile_node(scope, &owner, node)?;
            }
        }
        state
            .aria
            .migrated
            .borrow_mut()
            .retain(|weak| !weak.is_empty());
    }
    Ok(())
}

#[cfg(test)]
mod setter_exception_tests {
    use super::*;
    #[test]
    fn cancellation_allows_first_evaluation_after_setter_conversion_or_reaction_termination() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<x-stop-setter id=x></x-stop-setter>")
                .into_runtime()
                .unwrap(),
        ));
        install_generated_bindings(scope, context, dom, RealmId(510), 1).unwrap();
        fn halt(
            scope: &mut v8::PinScope,
            _args: v8::FunctionCallbackArguments,
            _rv: v8::ReturnValue,
        ) {
            scope.terminate_execution();
        }
        let callback = v8::Function::new(scope, halt).unwrap();
        let key = v8::String::new(scope, "halt").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), callback.into());
        super::super::test_eval::eval(
            scope,
            "customElements.define('x-stop-setter',class extends HTMLElement{static observedAttributes=['title'];attributeChangedCallback(){halt()}})",
        );
        for source in [
            "x.ariaLabel={toString(){halt();return 'unreachable'}}",
            "x.title='stop'",
            "x.setAttribute('title','again')",
        ] {
            let source = v8::String::new(scope, source).unwrap();
            let script = v8::Script::compile(scope, source, None).unwrap();
            // Public V8 clears termination at the outermost API return when no
            // TryCatch exists. Observe it at the host boundary before preserving it
            // for the explicit cancellation below, just as the native host does.
            {
                v8::tc_scope!(let caught,scope);
                assert!(script.run(caught).is_none());
                assert!(caught.has_terminated());
                assert!(caught.is_execution_terminating());
                caught.rethrow();
            }
            assert!(scope.is_execution_terminating());
            scope.cancel_terminate_execution();
            assert!(
                super::super::test_eval::eval(scope, "1+1===2").is_true(),
                "the first evaluation after cancellation must succeed"
            );
        }
    }
}
