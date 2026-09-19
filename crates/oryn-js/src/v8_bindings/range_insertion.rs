//! Range insertion retains original identities across children-changed script callbacks.
use super::*;
pub(super) fn insert(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let range_object = state
        .range_objects
        .borrow()
        .get(&id)
        .and_then(|record| record.handle.to_local(scope))
        .ok_or(V8BindingError::Allocation("Range insertion identity"))?;
    let (source, node) =
        node_identity::argument(scope, value)?.ok_or(V8BindingError::Unsupported {
            interface: "Range".into(),
            member: "TypeError".into(),
        })?;
    let range: RangeRecord = state.dom.borrow().live_range(LiveRangeId(id))?.into();
    let (parent, reference, text) = {
        let dom = state.dom.borrow();
        let kind = &dom.arena().get(range.start_container)?.kind;
        if matches!(
            kind,
            NodeKind::ProcessingInstruction { .. }
                | NodeKind::Comment { .. }
                | NodeKind::Attribute { .. }
        ) || (Rc::ptr_eq(&source.dom, &state.dom) && node == range.start_container)
        {
            return Err(hierarchy_request_error());
        }
        if matches!(kind, NodeKind::Text { .. } | NodeKind::CDataSection { .. }) {
            (
                dom.parent(range.start_container)?
                    .ok_or_else(hierarchy_request_error)?,
                Some(range.start_container),
                true,
            )
        } else {
            (
                range.start_container,
                light_children(&dom, range.start_container)?
                    .get(range.start_offset as usize)
                    .copied(),
                false,
            )
        }
    };
    template_links::validate(scope, state, parent, &source, node)?;
    if Rc::ptr_eq(&source.dom, &state.dom) {
        validate_pre_insertion(&state.dom.borrow(), parent, node, reference, None)?;
    } else {
        node_migration::validate_foreign_insertion(state, parent, &source, node, reference, None)?;
    }
    post_connection_scripts::check_insertion_capacity(
        scope,
        state,
        parent,
        &source.dom.borrow(),
        node,
    )?;
    let parent_object = wrap_node(scope, state, parent)?;
    let reference_object = if text {
        let preparation =
            script_children_changed::character_data(scope, state, range.start_container)?;
        let result = (|| -> Result<v8::Local<v8::Object>, V8BindingError> {
            let tail = state
                .dom
                .borrow_mut()
                .split_text(range.start_container, range.start_offset)?;
            synchronize_live_ranges(state)?;
            wrap_node(scope, state, tail)
        })();
        Some(preparation.finish_result(scope, result)?)
    } else {
        reference
            .map(|node| wrap_node(scope, state, node))
            .transpose()?
    };
    let (owner, parent) =
        node_identity::argument(scope, parent_object.into())?.ok_or(DomError::StaleNode(parent))?;
    let reference = match reference_object {
        None => None,
        Some(reference) => {
            let (reference_owner, node) = node_identity::argument(scope, reference.into())?
                .ok_or(DomError::StaleNode(parent))?;
            if !Rc::ptr_eq(&owner.dom, &reference_owner.dom) {
                return Err(V8BindingError::Unsupported {
                    interface: "Range".into(),
                    member: "NotFoundError".into(),
                });
            }
            Some(node)
        }
    };
    let insertion =
        node_migration::insertion_argument(scope, &owner, parent, value, reference, None)?;
    let owner = &insertion.owner;
    let parent = insertion.parent;
    let node = insertion.node;
    let mut reference = insertion.reference;
    if reference == Some(node) {
        let dom = owner.dom.borrow();
        let siblings = light_children(&dom, parent)?;
        reference = siblings
            .iter()
            .position(|candidate| *candidate == node)
            .and_then(|index| siblings.get(index + 1).copied());
    }
    let preparation = post_connection_scripts::capture_insertion(scope, owner, parent, node)?;
    let result = (|| -> Result<u32, V8BindingError> {
        if owner.dom.borrow().parent(node)?.is_some() {
            owner.dom.borrow_mut().remove(node)?;
        }
        let offset = {
            let dom = owner.dom.borrow();
            let siblings = light_children(&dom, parent)?;
            let index = match reference {
                Some(reference) => siblings
                    .iter()
                    .position(|child| *child == reference)
                    .ok_or(DomError::NotAChild(reference, parent))?,
                None => siblings.len(),
            };
            let count = if matches!(dom.arena().get(node)?.kind, NodeKind::DocumentFragment) {
                light_children(&dom, node)?.len()
            } else {
                1
            };
            u32::try_from(index.saturating_add(count))
                .map_err(|_| V8BindingError::Allocation("Range insertion offset"))?
        };
        owner
            .dom
            .borrow_mut()
            .insert_dom_node(parent, node, reference)?;
        synchronize_live_ranges(owner)?;
        Ok(offset)
    })();
    let offset = preparation.finish_result(scope, result)?;
    let (current, id) = range_boundaries::owner(scope, range_object)?;
    let range: RangeRecord = current.dom.borrow().live_range(LiveRangeId(id))?.into();
    if range.start_container == range.end_container && range.start_offset == range.end_offset {
        let (owner, parent) = node_identity::argument(scope, parent_object.into())?
            .ok_or(DomError::StaleNode(parent))?;
        range_boundaries::set_end_native(scope, range_object, &owner, parent, offset)?;
    }
    Ok(())
}
