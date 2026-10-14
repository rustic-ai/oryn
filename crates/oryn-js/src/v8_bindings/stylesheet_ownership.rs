//! A stylesheet has an identity and lifetime independent of its owner element.
//! Internal detached style nodes hold sheet sources, as for constructed sheets.
use super::*;

struct Record {
    owner: Option<NodeId>,
    revision: u64,
    document: NodeId,
    href: Option<String>,
    title: Option<String>,
    lease: Option<oryn_dom::TrackedNodeLease>,
}

#[derive(Default)]
pub(super) struct Store {
    current: RefCell<BTreeMap<NodeId, NodeId>>,
    snapshots: RefCell<BTreeMap<NodeId, Rc<str>>>,
    records: RefCell<BTreeMap<NodeId, Record>>,
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
        let lease = state
            .stylesheet_ownership
            .records
            .borrow_mut()
            .get_mut(&sheet)
            .and_then(|record| record.lease.take());
        drop(lease);
    }
    Ok(())
}

pub(super) fn available(state: &V8BindingState, node: NodeId) -> Result<bool, V8BindingError> {
    let dom = state.dom.borrow();
    if !dom.is_connected(node)? {
        return Ok(false);
    }
    let record = dom.arena().get(node)?;
    Ok(record.stylesheet_parser_ready
        && record.stylesheet_policy_allowed
        && dom.arena().is_css_stylesheet_owner(node)?
        && (matches!(&record.kind,NodeKind::Element{name} if name.local.as_ref()=="style")
            || record.stylesheet_override.is_some()))
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
        .and_then(|r| r.owner.map(|owner| (owner, r.revision, r.document)));
    let Some((owner, revision, document)) = snapshot else {
        return Ok(None);
    };
    let active = {
        let dom = state.dom.borrow();
        dom.arena()
            .stylesheet_revision(owner)
            .is_ok_and(|r| r == revision)
            && dom.is_connected(owner).unwrap_or(false)
            && dom.arena().node_document(owner).ok().flatten() == Some(document)
    };
    if active && available(state, owner)? {
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
    state.reconcile_inline_style_policy()?;
    if !state
        .dom
        .borrow()
        .arena()
        .get(node)?
        .stylesheet_policy_allowed
    {
        return Ok(None);
    }
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
    if !available(state, node)? {
        return Ok(None);
    }
    let (source, media, disabled, revision, connected) = {
        let dom = state.dom.borrow();
        (
            dom.arena().stylesheet_text(node)?,
            dom.arena().stylesheet_media(node)?,
            dom.arena().stylesheet_disabled(node)?,
            dom.arena().stylesheet_revision(node)?,
            dom.is_connected(node)?,
        )
    };
    if !connected {
        return Ok(None);
    }
    let document = owner_document_for_node(state, node)?;
    let (href, title) = {
        let dom = state.dom.borrow();
        let record = dom.arena().get(node)?;
        let href = if dom.is_element(node, "http://www.w3.org/1999/xhtml", "link")
            || dom.arena().xml_stylesheet(node)?.is_some()
        {
            record
                .stylesheet_base_url
                .clone()
                .or_else(|| record.attributes.get_null("href").cloned())
        } else {
            None
        };
        (href, dom.arena().stylesheet_title(node)?)
    };
    let sheet = {
        let mut dom = state.dom.borrow_mut();
        let sheet = dom.create_element("style");
        dom.arena_mut().get_mut(sheet)?.stylesheet_override = Some(source.into());
        dom.arena_mut().get_mut(sheet)?.stylesheet_media = Some(media);
        dom.arena_mut().get_mut(sheet)?.stylesheet_disabled = Some(disabled);
        sheet
    };
    let lease = oryn_dom::TrackedNodeLease::new(&state.dom, sheet)?;
    state.dom.borrow_mut().set_node_document(sheet, document)?;
    // This synthetic node stores CSS source; it has no custom-element
    // construction/upgrade semantics and must not root the document registry.
    state
        .dom
        .borrow_mut()
        .arena_mut()
        .get_mut(sheet)?
        .custom_element_registry = Some(None);
    state.stylesheet_ownership.records.borrow_mut().insert(
        sheet,
        Record {
            owner: Some(node),
            revision,
            document,
            href,
            title,
            lease: Some(lease),
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

/// Detach source sheets without moving their backing nodes or retained rules.
/// The destination owner receives a fresh sheet identity after authorization.
pub(super) fn detach_adopted(
    state: &V8BindingState,
    nodes: &BTreeMap<NodeId, NodeId>,
) -> Result<(), V8BindingError> {
    let sheets = state
        .stylesheet_ownership
        .current
        .borrow()
        .iter()
        .filter_map(|(owner, sheet)| nodes.contains_key(owner).then_some(*sheet))
        .collect::<Vec<_>>();
    for sheet in sheets {
        detach(state, sheet)?;
    }
    Ok(())
}

pub(super) fn retained_metadata(
    state: &V8BindingState,
    sheet: NodeId,
    member: &str,
) -> Option<String> {
    state
        .stylesheet_ownership
        .records
        .borrow()
        .get(&sheet)
        .and_then(|record| {
            if member == "href" {
                record.href.clone()
            } else {
                record.title.clone()
            }
        })
}

#[cfg(test)]
mod lease_tests {
    use super::*;
    #[test]
    fn synthetic_source_sheet_keeps_css_custody_without_a_custom_registry() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles,&mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<head><style>body{color:red}</style></head><body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(907), 1).unwrap();
        let source = v8::String::new(scope, "document.styleSheets[0].cssRules.length===1").unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
        let sheets = state
            .stylesheet_ownership
            .records
            .borrow()
            .keys()
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(sheets.len(), 1);
        for sheet in &sheets {
            let dom = state.dom.borrow();
            assert_eq!(
                dom.arena().get(*sheet).unwrap().custom_element_registry,
                Some(None)
            );
            assert_eq!(dom.native_root_count(*sheet).unwrap(), 1);
        }
        state.retire_execution();
        state.refresh_connected_wrapper_roots(scope);
        for sheet in sheets {
            assert!(
                active_owner(&state, sheet).unwrap().is_some(),
                "CSS source ownership is preserved for a retained retired Document"
            );
            assert_eq!(
                state
                    .dom
                    .borrow()
                    .arena()
                    .get(sheet)
                    .unwrap()
                    .custom_element_registry,
                Some(None)
            );
        }
    }
    #[test]
    fn store_drop_retains_release_custody_while_arena_is_borrowed() {
        let dom = Rc::new(RefCell::new(DomRuntime::new()));
        let document = dom.borrow().document();
        let sheet = dom.borrow_mut().create_element("style");
        let lease = oryn_dom::TrackedNodeLease::new(&dom, sheet).unwrap();
        let store = Store::default();
        store.current.borrow_mut().insert(document, sheet);
        store.records.borrow_mut().insert(
            sheet,
            Record {
                owner: Some(document),
                revision: 0,
                document,
                href: None,
                title: None,
                lease: Some(lease),
            },
        );
        let borrowed = dom.borrow_mut();
        drop(store);
        assert!(borrowed.arena().get(sheet).is_ok());
        drop(borrowed);
        oryn_dom::drain_tracked_node_leases().unwrap();
        assert!(dom.borrow().arena().get(sheet).is_err());
    }
}
