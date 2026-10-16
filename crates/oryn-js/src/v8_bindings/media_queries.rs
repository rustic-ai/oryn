//! MediaQueryList identities, native matches and rendering-checkpoint changes.
use super::*;
pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    query: &str,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let context = state
        .realm_context
        .to_local(scope)
        .ok_or(V8BindingError::Allocation("media query owner context"))?;
    let scope = &mut v8::ContextScope::new(scope, context);
    synchronize_embedded_viewport_in_scope(scope, state)?;
    let _retained_scope = state.enter_retained_allocation(scope)?;
    let base = state.native_document_base_url(false)?;
    let quirks = state
        .dom
        .borrow()
        .arena()
        .get(state.document_root())?
        .document_metadata
        .as_ref()
        .is_some_and(|m| m.quirks);
    let normalized = state
        .layout_provider
        .borrow()
        .as_ref()
        .and_then(|p| {
            p.mutate_stylesheet_media(query, BindingMediaListEdit::Replace(query), &base, quirks)
        })
        .ok_or(V8BindingError::Allocation(
            "native media parser unavailable",
        ))?
        .map_err(V8BindingError::EventCallback)?
        .queries
        .join(", ");
    let observed = state
        .native_media_matches(&normalized)
        .map_err(V8BindingError::EventCallback)?;
    let id = register_platform_object(
        state,
        PlatformObjectRecord::MediaQueryList {
            query: normalized,
            observed,
        },
    );
    new_platform_object(scope, state, "MediaQueryList", id)
}
pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let id = stateful_id(scope, object)?;
    let query = match &*platform_object(state, id)? {
        PlatformObjectRecord::MediaQueryList { query, .. } => query.clone(),
        _ => return Err(V8BindingError::Allocation("MediaQueryList receiver")),
    };
    if member == "media" {
        v8_string(scope, &query)
    } else {
        let matches = native_media_matches_in_scope(scope, state, &query)?;
        Ok(v8::Boolean::new(scope, matches).into())
    }
}
pub(super) fn legacy_listener(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
    callback: v8::Local<v8::Value>,
    add: bool,
) -> Result<(), V8BindingError> {
    let id = stateful_id(scope, object)?;
    if !matches!(
        &*platform_object(state, id)?,
        PlatformObjectRecord::MediaQueryList { .. }
    ) {
        return Err(V8BindingError::Allocation("MediaQueryList receiver"));
    }
    if callback.is_null_or_undefined() {
        return Ok(());
    }
    if !callback.is_object() {
        return Err(V8BindingError::Unsupported {
            interface: "MediaQueryList".into(),
            member: "TypeError: listener must be an object or null".into(),
        });
    }
    {
        let mut all = state.platform_listeners.borrow_mut();
        let listeners = all.entry((id, "change".into())).or_default();
        if add {
            if !listeners.iter().any(|entry| {
                !entry.global_handler && !entry.capture && entry.callback.equals(scope, callback)
            }) {
                let registration_id = state.next_event_listener_id.get();
                state.next_event_listener_id.set(
                    registration_id
                        .checked_add(1)
                        .ok_or(V8BindingError::Allocation("listener identity exhausted"))?,
                );
                listeners.push(PlatformEventListener {
                    registration_id,
                    global_handler: false,
                    callback: platform_listener_graph::Callback::new(scope, callback),
                    capture: false,
                    once: false,
                    passive: false,
                    signal: None,
                });
            }
        } else {
            listeners.retain(|entry| {
                entry.global_handler || entry.capture || !entry.callback.equals(scope, callback)
            });
        }
    }
    platform_listener_graph::refresh(scope, state, id)
}
pub(super) fn initialize_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> Result<(), V8BindingError> {
    let media =
        event_init_member(scope, init, "media").ok_or(V8BindingError::JavaScriptException)?;
    let media = if media.is_undefined() {
        String::new()
    } else {
        media
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?
            .to_rust_string_lossy(scope)
    };
    let matches = event_init_member(scope, init, "matches")
        .ok_or(V8BindingError::JavaScriptException)?
        .boolean_value(scope);
    set_payload(scope, event, &media, matches)
}
fn set_payload(
    scope: &mut v8::PinScope,
    event: v8::Local<v8::Object>,
    media: &str,
    matches: bool,
) -> Result<(), V8BindingError> {
    let payload = v8::Array::new(scope, 3);
    let brand = v8::String::new(scope, "MediaQueryListEvent")
        .ok_or(V8BindingError::Allocation("media event brand"))?;
    payload.set_index(scope, 2, brand.into());
    let media = v8_string(scope, media)?;
    payload.set_index(scope, 0, media);
    let matches = v8::Boolean::new(scope, matches);
    payload.set_index(scope, 1, matches.into());
    event.set_internal_field(2, payload.into());
    Ok(())
}
pub(super) fn event_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    event_id(scope, event)?;
    let payload = event
        .get_internal_field(scope, 2)
        .and_then(|v| v8::Local::<v8::Array>::try_from(v).ok())
        .ok_or(V8BindingError::Allocation("MediaQueryListEvent receiver"))?;
    if !payload
        .get_index(scope, 2)
        .is_some_and(|v| v.to_rust_string_lossy(scope) == "MediaQueryListEvent")
    {
        return Err(V8BindingError::Allocation("MediaQueryListEvent receiver"));
    }
    payload
        .get_index(scope, if member == "media" { 0 } else { 1 })
        .ok_or(V8BindingError::Allocation("MediaQueryListEvent payload"))
}
pub fn poll(scope: &mut v8::PinScope, state: &Rc<V8BindingState>) -> Result<usize, V8BindingError> {
    if state.execution_retired() {
        return Ok(0);
    }
    let context = state
        .realm_context
        .to_local(scope)
        .ok_or(V8BindingError::Allocation("media query owner context"))?;
    let scope = &mut v8::ContextScope::new(scope, context);
    // One scoped viewport synchronization for the checkpoint; query evaluation
    // below cannot recursively trigger a parent's layout without admission.
    synchronize_embedded_viewport_in_scope(scope, state)?;
    let _retained_scope = state.enter_retained_allocation(scope)?;
    let queries = state
        .platform_objects
        .borrow()
        .iter()
        .filter_map(|(id, record)| match record {
            PlatformObjectRecord::MediaQueryList { query, observed } => Some((
                *id,
                query.clone(),
                *observed,
                state
                    .platform_wrappers
                    .borrow()
                    .get(id)
                    .and_then(|w| w.to_local(scope))
                    .is_some(),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut changed = Vec::new();
    for (id, query, previous, live) in queries {
        if !live {
            state.platform_objects.borrow_mut().remove(&id);
            state.platform_wrappers.borrow_mut().remove(&id);
            continue;
        }

        let next = state
            .native_media_matches(&query)
            .map_err(V8BindingError::EventCallback)?;
        if let Some(PlatformObjectRecord::MediaQueryList { observed, .. }) =
            state.platform_objects.borrow_mut().get_mut(&id)
        {
            *observed = next;
        }
        if live && next != previous {
            changed.push((id, query, next));
        }
    }
    let identity = (
        state.document_root(),
        state.document_generation(),
        state.document_stream_epoch(),
    );
    let mut delivered = 0;
    for (id, query, matches) in changed {
        if state.execution_retired()
            || identity
                != (
                    state.document_root(),
                    state.document_generation(),
                    state.document_stream_epoch(),
                )
        {
            break;
        }
        let event = create_event_object_with_interface(
            scope,
            state,
            "MediaQueryListEvent",
            "change",
            false,
            false,
        )?;
        set_payload(scope, event, &query, matches)?;
        let event_id = event_id(scope, event)?;
        if let Some(record) = state.events.borrow_mut().get_mut(&event_id) {
            record.is_trusted = true;
        }
        dispatch_platform_event(scope, state, id, "change", Some(event))?;
        delivered += 1;
    }
    Ok(delivered)
}

#[cfg(test)]
mod scoped_tests {
    use super::*;

    struct Provider {
        frame: Option<NodeId>,
        width: Cell<f64>,
        computes: Cell<usize>,
    }
    impl BindingLayoutProvider for Provider {
        fn compute_layout(
            &self,
            _: &BTreeMap<NodeId, BTreeSet<String>>,
        ) -> Result<BindingLayout, String> {
            self.computes.set(self.computes.get() + 1);
            let mut layout = BindingLayout::default();
            if let Some(frame) = self.frame {
                let rect = oryn_common::v4::CssRect {
                    x: 0.,
                    y: 0.,
                    width: self.width.get(),
                    height: 100.,
                };
                layout.hit_projections.insert(
                    node_key(frame),
                    oryn_common::v4::PaintProjection {
                        scroll: None,
                        fragment_boxes: Vec::new(),
                        border_segments: Vec::new(),
                        border_edges: None,
                        text_matrix: None,
                        text_clips: Vec::new(),
                        matrix: [
                            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
                        ],
                        box_model: oryn_common::v4::BoxModel {
                            border_box: rect,
                            padding_box: rect,
                            content_box: rect,
                            margin: Default::default(),
                            border: Default::default(),
                            padding: Default::default(),
                        },
                        text: Vec::new(),
                        clips: Vec::new(),
                    },
                );
            }
            Ok(layout)
        }
        fn set_viewport(&self, width: f64, _: f64) {
            self.width.set(width);
        }
        fn native_media_matches(&self, _: &str, _: &str, _: bool) -> Result<bool, String> {
            Ok(self.width.get() > 600.)
        }
    }

    #[test]
    fn idle_media_hint_never_projects_parent_but_scoped_evaluation_sees_resize() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let parent_context = v8::Context::new(handles, Default::default());
        let parent_dom = Rc::new(RefCell::new(
            html::parse("<iframe id=f></iframe>")
                .into_runtime()
                .unwrap(),
        ));
        let parent = {
            let scope = &mut v8::ContextScope::new(handles, parent_context);
            install_generated_bindings(scope, parent_context, parent_dom.clone(), RealmId(0), 1)
                .unwrap()
        };
        let frame = parent_dom.borrow().find_by_id("f").unwrap();
        let parent_provider = Rc::new(Provider {
            frame: Some(frame),
            width: Cell::new(400.),
            computes: Cell::new(0),
        });
        parent.set_layout_provider(parent_provider.clone());
        let child_context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, child_context);
        let child_dom = Rc::new(RefCell::new(
            html::parse("<p>child</p>").into_runtime().unwrap(),
        ));
        let child =
            install_generated_bindings(scope, child_context, child_dom, RealmId(1), 1).unwrap();
        child.set_layout_provider(Rc::new(Provider {
            frame: None,
            width: Cell::new(400.),
            computes: Cell::new(0),
        }));
        child.set_frame_viewport_parent(&parent, frame).unwrap();
        register_platform_object(
            &child,
            PlatformObjectRecord::MediaQueryList {
                query: "(width > 600px)".into(),
                observed: false,
            },
        );
        assert!(child.has_pending_media_query_change());
        assert_eq!(parent_provider.computes.get(), 0);
        assert!(!native_media_matches_in_scope(scope, &child, "(width > 600px)").unwrap());
        let previous = parent_provider.computes.get();
        parent_provider.width.set(800.);
        parent.layout_cache_key.borrow_mut().take();
        assert!(child.has_pending_media_query_change());
        assert_eq!(parent_provider.computes.get(), previous);
        assert!(native_media_matches_in_scope(scope, &child, "(width > 600px)").unwrap());
        assert!(parent_provider.computes.get() > previous);
    }
}
