//! A stylesheet has an identity and lifetime independent of its owner element.
//! Internal detached style nodes hold sheet sources, as for constructed sheets.
use super::*;

struct Record {
    owner: Option<NodeId>,
    revision: u64,
}

#[derive(Default)]
pub(super) struct Store {
    current: RefCell<BTreeMap<NodeId, NodeId>>,
    snapshots: RefCell<BTreeMap<NodeId, Rc<str>>>,
    records: RefCell<BTreeMap<NodeId, Record>>,
    dom: RefCell<Option<std::rc::Weak<RefCell<DomRuntime>>>>,
}

impl Drop for Store {
    fn drop(&mut self) {
        if let Some(dom) = self.dom.get_mut().as_ref().and_then(std::rc::Weak::upgrade)
            && let Ok(mut dom) = dom.try_borrow_mut()
        {
            for sheet in self.current.get_mut().values() {
                let _ = dom.release_native(*sheet);
            }
        }
    }
}

fn detach(state: &V8BindingState, sheet: NodeId) -> Result<(), V8BindingError> {
    let owner = state
        .stylesheet_ownership
        .records
        .borrow_mut()
        .get_mut(&sheet)
        .and_then(|r| r.owner.take());
    if let Some(owner) = owner {
        if state.stylesheet_ownership.current.borrow().get(&owner) == Some(&sheet) {
            state
                .stylesheet_ownership
                .current
                .borrow_mut()
                .remove(&owner);
        }
        state.dom.borrow_mut().release_native(sheet)?;
    }
    Ok(())
}

/// Return the active owner only while its DOM stylesheet generation matches.
/// A retained obsolete sheet must never publish into a replacement stylesheet.
pub(super) fn active_owner(
    state: &V8BindingState,
    sheet: NodeId,
) -> Result<Option<NodeId>, V8BindingError> {
    let snapshot = state
        .stylesheet_ownership
        .records
        .borrow()
        .get(&sheet)
        .and_then(|r| r.owner.map(|owner| (owner, r.revision)));
    let Some((owner, revision)) = snapshot else {
        return Ok(None);
    };
    let active = {
        let dom = state.dom.borrow();
        dom.arena()
            .stylesheet_revision(owner)
            .is_ok_and(|r| r == revision)
            && dom.is_connected(owner).unwrap_or(false)
    };
    if active {
        Ok(Some(owner))
    } else {
        detach(state, sheet)?;
        Ok(None)
    }
}

pub(super) fn is_sheet(state: &V8BindingState, node: NodeId) -> bool {
    state
        .stylesheet_ownership
        .records
        .borrow()
        .contains_key(&node)
}

pub(super) fn for_node(
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<Option<NodeId>, V8BindingError> {
    if is_sheet(state, node) || state.constructed_stylesheets.borrow().contains(&node) {
        return Ok(Some(node));
    }
    let previous = state
        .stylesheet_ownership
        .current
        .borrow()
        .get(&node)
        .copied();
    if let Some(sheet) = previous
        && active_owner(state, sheet)? == Some(node)
    {
        return Ok(Some(sheet));
    }
    let (source, media, revision, connected) = {
        let dom = state.dom.borrow();
        (
            dom.arena().stylesheet_text(node)?,
            dom.arena().stylesheet_media(node)?,
            dom.arena().stylesheet_revision(node)?,
            dom.is_connected(node)?,
        )
    };
    if !connected {
        return Ok(None);
    }
    let document = owner_document_for_node(state, node)?;
    let sheet = {
        let mut dom = state.dom.borrow_mut();
        let sheet = dom.create_element("style");
        dom.arena_mut().get_mut(sheet)?.stylesheet_override = Some(source);
        dom.arena_mut().get_mut(sheet)?.stylesheet_media = Some(media);
        dom.retain_native(sheet)?;
        sheet
    };
    state.dom.borrow_mut().set_node_document(sheet, document)?;
    *state.stylesheet_ownership.dom.borrow_mut() = Some(Rc::downgrade(&state.dom));
    state.stylesheet_ownership.records.borrow_mut().insert(
        sheet,
        Record {
            owner: Some(node),
            revision,
        },
    );
    state
        .stylesheet_ownership
        .current
        .borrow_mut()
        .insert(node, sheet);
    Ok(Some(sheet))
}

/// Share full-sheet parse context across all rules in that generation.
pub(super) fn source_snapshot(
    state: &V8BindingState,
    sheet: NodeId,
) -> Result<Rc<str>, V8BindingError> {
    let source = state.dom.borrow().arena().stylesheet_text(sheet)?;
    let mut snapshots = state.stylesheet_ownership.snapshots.borrow_mut();
    if let Some(snapshot) = snapshots.get(&sheet)
        && snapshot.as_ref() == source
    {
        return Ok(snapshot.clone());
    }
    let snapshot: Rc<str> = Rc::from(source);
    snapshots.insert(sheet, snapshot.clone());
    Ok(snapshot)
}

pub(super) fn sweep(state: &V8BindingState) -> usize {
    let sheets = state
        .stylesheet_ownership
        .current
        .borrow()
        .values()
        .copied()
        .collect::<Vec<_>>();
    let mut released = 0;
    for sheet in sheets {
        if matches!(active_owner(state, sheet), Ok(None)) {
            released += 1;
        }
    }
    let dom = state.dom.borrow();
    state
        .stylesheet_ownership
        .records
        .borrow_mut()
        .retain(|node, _| dom.arena().get(*node).is_ok());
    state
        .stylesheet_ownership
        .snapshots
        .borrow_mut()
        .retain(|node, _| dom.arena().get(*node).is_ok());
    released
}

/// Refresh a live sheet from its owner, retaining the last value after detachment.
pub(super) fn media_text(state: &V8BindingState, sheet: NodeId) -> Result<String, V8BindingError> {
    if let Some(owner) = active_owner(state, sheet)? {
        let text = state.dom.borrow().arena().stylesheet_media(owner)?;
        state
            .dom
            .borrow_mut()
            .arena_mut()
            .get_mut(sheet)?
            .stylesheet_media = Some(text.clone());
        return Ok(text);
    }
    Ok(state.dom.borrow().arena().stylesheet_media(sheet)?)
}

pub(super) fn set_media_text(
    state: &V8BindingState,
    sheet: NodeId,
    text: String,
) -> Result<(), V8BindingError> {
    let owner = active_owner(state, sheet)?;
    let mut dom = state.dom.borrow_mut();
    dom.set_stylesheet_media(sheet, text.clone())?;
    if let Some(owner) = owner {
        dom.set_stylesheet_media(owner, text)?;
    }
    Ok(())
}
