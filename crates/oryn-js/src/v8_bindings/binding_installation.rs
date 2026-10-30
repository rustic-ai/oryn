//! Binding installation phases have separate native stack frames.
//! In particular, state initialization temporaries are gone before template
//! allocation can trigger V8 GC during reentrant frame creation.
use super::*;

/// Native callback overrides retain the generated interface's WebIDL metadata.
/// Inspect only newly constructed native prototypes, before exposing them to
/// author code; descriptor reads never invoke the member's getter or setter.
fn restore_native_member_metadata(
    scope: &mut v8::PinScope,
    prototype: v8::Local<v8::Object>,
    interface: &crate::generated::GeneratedInterface,
) -> Result<(), V8BindingError> {
    for member in interface.members {
        if member.static_member || member.name.is_empty() {
            continue;
        }
        let key = v8::String::new(scope, member.name)
            .ok_or(V8BindingError::Allocation("native member metadata key"))?;
        let Some(descriptor) = prototype.get_own_property_descriptor(scope, key.into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok()) else {
            continue;
        };
        let fields = match member.kind {
            GeneratedMemberKind::Attribute => [("get", "get ", 0), ("set", "set ", 1)],
            GeneratedMemberKind::Operation => [("value", "", interface.members.iter()
                .filter(|overload| overload.kind == GeneratedMemberKind::Operation
                    && overload.name == member.name && !overload.static_member)
                .map(|overload| overload.arity).min().unwrap_or(member.arity)), ("", "", 0)],
            _ => continue,
        };
        for (field, prefix, arity) in fields {
            if field.is_empty() { continue; }
            let field = v8::String::new(scope, field)
                .ok_or(V8BindingError::Allocation("native member descriptor field"))?;
            let Some(function) = descriptor.get(scope, field.into())
                .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok()) else {
                continue;
            };
            let name = v8::String::new(scope, &format!("{prefix}{}", member.name))
                .ok_or(V8BindingError::Allocation("native member function name"))?;
            function.set_name(name);
            let length = v8::String::new(scope, "length")
                .ok_or(V8BindingError::Allocation("native member function length"))?;
            let arity = v8::Integer::new(scope, i32::from(arity));
            if function.define_own_property(scope, length.into(), arity.into(),
                v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_ENUM) != Some(true) {
                return Err(V8BindingError::Allocation("native member function metadata"));
            }
        }
    }
    Ok(())
}
#[inline(never)]
pub(super) fn create_state(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    dom: SharedDom,
    realm: RealmId,
    document_generation: u64,
    document: NodeId,
    binding_global: BindingGlobal,
) -> Result<Rc<V8BindingState>, V8BindingError> {
    let global_object = context.global(scope);
    custom_state_iterator::install(scope, context)?;
    {
        let mut dom = dom.borrow_mut();
        let metadata = dom
            .arena_mut()
            .get_mut(document)?
            .document_metadata
            .get_or_insert_with(|| Box::new(oryn_dom::DocumentMetadata::html()));
        if metadata.url.is_none() {
            metadata.url = Some("about:blank".into());
        }
    }
    dom.borrow_mut()
        .set_pattern_matcher(crate::pattern_service::for_isolate(scope)?);
    // Live ranges and decoded/canvas pixels belong to the shared DOM, not
    // the realm that happened to create a wrapper. Sharing native maps keeps
    // adopted nodes authoritative without copying or losing taint on adoption.
    let range_owner = scope
        .get_slot::<BindingStateRegistry>()
        .and_then(|registry| {
            registry
                .instances
                .borrow()
                .values()
                .filter_map(RcWeak::upgrade)
                .find(|candidate| Rc::ptr_eq(&candidate.dom, &dom))
        });
    let selection_range = Rc::new(Cell::new(None));
    let selection_composed_range = Rc::new(Cell::new(None));
    let range_companions = range_owner.as_ref().map_or_else(
        || Rc::new(RefCell::new(Vec::new())),
        |owner| owner.range_companions.clone(),
    );
    range_companions.borrow_mut().push((
        Rc::downgrade(&selection_range),
        Rc::downgrade(&selection_composed_range),
    ));
    let range_selections = range_owner.as_ref().map_or_else(
        || Rc::new(RefCell::new(Vec::new())),
        |owner| owner.range_selections.clone(),
    );
    range_selections
        .borrow_mut()
        .push(Rc::downgrade(&selection_range));
    range_selections
        .borrow_mut()
        .push(Rc::downgrade(&selection_composed_range));
    let details_toggle_cursor = dom.borrow().mutation_cursor();
    let state = Rc::new(V8BindingState {
        stylesheet_execution_owner: RefCell::new(std::rc::Weak::new()),
        inline_publication_cursor: Cell::new(None),
        document_provenance_capture: RefCell::new(None),
        animation_frames: RefCell::new(animation_frames::State::default()),
        web_animations: RefCell::new(web_animations::State::default()),
        animation_failure_diagnostic: web_animations::FailureDiagnostic::default(),
        page_revealed: Cell::new(false),
        speculation_rules: RefCell::new(speculation_registry::Registry::default()),
        document_prerendering: Cell::new(false),
        initial_pageshow_delivered: Cell::new(false),
        document_completely_loaded: Cell::new(false),
        stream_anchors: RefCell::new(BTreeMap::new()),
        stream_activity_roots: RefCell::new(BTreeMap::new()),
        stream_anchor_releases: Rc::new(RefCell::new(Vec::new())),
        stream_abort_pending: RefCell::new(BTreeSet::new()),
        pipe_dependents: RefCell::new(Vec::new()),
        native_timeout_scheduler: Cell::new(None),
        document_refresh: RefCell::new(Default::default()),
        automatic_features_allowed: Cell::new(true),
        location_navigation_backend: RefCell::new(None),
        navigation_target_selector: Cell::new(None),
        document_domain_authorizer: Cell::new(None),
        document_cookie_access: RefCell::new(None),
        origin_agent_cluster: Cell::new(None),
        hyperlink_audit_preparer: Cell::new(None),
        selected_link_dispatcher: Cell::new(None),
        xpath: RefCell::new(xpath::Registry::default()),
        historical_input_authority: Cell::new(None),
        historical_input_serial: Cell::new(0),
        binding_global,
        prompt_controller: std::sync::Arc::new(prompts::PromptController::new()),
        message_transport: RefCell::new(None),
        message_target: RefCell::new(crate::message_wire::WindowTarget::Root),
        worker_transport: RefCell::new(None),
        #[cfg(unix)]
        shared_waits: RefCell::new(None),
        worker_creator: RefCell::new(None),
        worker_base_url: RefCell::new(None),
        service_events: RefCell::new(Default::default()),
        fetch_bodies: RefCell::new(Default::default()),
        deferred_fetch: RefCell::new(Default::default()),
        deferred_fetch_backend: RefCell::new(None),
        deferred_fetch_backend_provider: Cell::new(None),
        body_consumers: RefCell::new(Vec::new()),
        native_drag: RefCell::new(None),
        drag_images: RefCell::new(Default::default()),
        native_pointers: RefCell::new(Default::default()),
        interests: RefCell::new(Default::default()),
        popover_publications: RefCell::new(Default::default()),
        details_toggle_cursor: Cell::new(details_toggle_cursor),
        fullscreen: RefCell::new(Default::default()),
        dialogs: RefCell::new(Default::default()),
        form_submission_jobs: RefCell::new(Default::default()),
        activation_jobs: RefCell::new(Default::default()),
        error_reports: RefCell::new(Default::default()),
        script_error_diagnostics: RefCell::new(Default::default()),
        picture_in_picture: RefCell::new(Default::default()),
        permissions_policy_objects: RefCell::new(Default::default()),
        native_visual_viewport: RefCell::new(Default::default()),
        visual_viewport_observed: Cell::new(None),
        visual_viewport_scroll_end: Cell::new(false),
        native_drag_dispatching: Cell::new(false),
        before_unload_prototype: RefCell::new(None),
        before_unload_dispatching: Cell::new(false),
        before_unload_owner: RefCell::new(None),
        instance_id: NEXT_BINDING_STATE_ID.fetch_add(1, AtomicOrdering::Relaxed),
        dom,
        realm,
        document: Cell::new(document),
        document_generation: Cell::new(document_generation),
        document_stream_epoch: Cell::new(0),
        native_input_identity: RefCell::new(None),
        document_authority_epoch: Cell::new(0),
        closed_document_stream: Cell::new(None),
        document_load_stopped: Cell::new(false),
        load_cancellation_epoch: Cell::new(0),
        wrapper_generation: document_generation,
        wrappers: Rc::new(RefCell::new(WrapperRegistry::default())),
        objects: RefCell::new(BTreeMap::new()),
        wrapper_graph_revision: Cell::new(None),
        wrapper_graph_new_nodes: RefCell::new(BTreeSet::new()),
        wrapper_graph_registry_nodes: RefCell::new(BTreeSet::new()),
        wrapper_graph_components: RefCell::new(BTreeMap::new()),
        instance_templates: RefCell::new(BTreeMap::new()),
        html_constructors: RefCell::new(BTreeMap::new()),
        html_prototypes: RefCell::new(BTreeMap::new()),
        associated_wrappers: RefCell::new(BTreeMap::new()),
        custom_states: range_owner.as_ref().map_or_else(
            || Rc::new(RefCell::new(BTreeMap::new())),
            |owner| owner.custom_states.clone(),
        ),
        custom_state_order: range_owner.as_ref().map_or_else(
            || Rc::new(RefCell::new(BTreeMap::new())),
            |owner| owner.custom_state_order.clone(),
        ),
        clock_origin: Cell::new(Instant::now()),
        time_origin_ms: Cell::new(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as f64,
        ),
        next_event_id: Cell::new(1),
        events: RefCell::new(BTreeMap::new()),
        event_objects: RefCell::new(BTreeMap::new()),
        released_events: Rc::new(RefCell::new(Vec::new())),
        listeners: RefCell::new(BTreeMap::new()),
        event_handlers: RefCell::new(BTreeMap::new()),
        suppressed_content_handlers: RefCell::new(BTreeSet::new()),
        next_mutation_observer_id: Cell::new(1),
        mutation_observers: RefCell::new(BTreeMap::new()),
        mutation_record_brand: RefCell::new(None),
        error_event_brand: RefCell::new(None),
        promise_rejection_event_brand: RefCell::new(None),
        ui_event_brand: RefCell::new(None),
        click_in_progress: RefCell::new(BTreeSet::new()),
        reset_in_progress: RefCell::new(Vec::new()),
        submission_events_in_progress: RefCell::new(Vec::new()),
        message_event_brand: RefCell::new(None),
        computed_style_brand: RefCell::new(None),
        event_brand: RefCell::new(None),
        global_object: v8::Weak::new(scope, global_object),
        realm_context: v8::Weak::new(scope, context),
        reporting_exception: Cell::new(false),
        worker_error_reporter: RefCell::new(None),
        next_event_listener_id: Cell::new(1),
        ranges: range_owner.as_ref().map_or_else(
            || Rc::new(RefCell::new(BTreeMap::new())),
            |owner| owner.ranges.clone(),
        ),
        static_range_custody: range_owner.as_ref().map_or_else(
            || Rc::new(RefCell::new(BTreeMap::new())),
            |owner| owner.static_range_custody.clone(),
        ),
        static_ranges: range_owner.as_ref().map_or_else(
            || Rc::new(RefCell::new(BTreeSet::new())),
            |owner| owner.static_ranges.clone(),
        ),
        range_objects: range_owner.as_ref().map_or_else(
            || Rc::new(RefCell::new(BTreeMap::new())),
            |owner| owner.range_objects.clone(),
        ),
        range_memory_delta: range_owner.as_ref().map_or_else(
            || Rc::new(Cell::new(0)),
            |owner| owner.range_memory_delta.clone(),
        ),
        traced_range_revision: Cell::new(None),
        opaque_range_objects: RefCell::new(BTreeMap::new()),
        node_lists: node_lists::Store::default(),
        selection_range,
        selection_composed_range,
        selection_range_removal_revision: Cell::new(0),
        range_companions,
        range_selections,
        rooted_range_selection_ids: range_owner.as_ref().map_or_else(
            || Rc::new(RefCell::new(None)),
            |owner| owner.rooted_range_selection_ids.clone(),
        ),
        inherited_base_url: RefCell::new(None),
        selection_backward: Cell::new(false),
        selection_anchor: Cell::new(None),
        selection_focus: Cell::new(None),
        selection_revision: Cell::new(0),
        selection_modify_inline: Cell::new(None),
        selection_modify_caret: Cell::new(None),
        delivered_selection_revision: Cell::new(0),
        selected_text_control: RefCell::new(None),
        text_control_selections: RefCell::new(BTreeMap::new()),
        control_modify_goal: RefCell::new(None),
        control_event_tasks: RefCell::new(VecDeque::new()),
        stylesheet_event_tasks: RefCell::new(VecDeque::new()),
        window_messages: RefCell::new(window_messages::Queue::new(context)),
        control_tasks_retired: ExecutionRetirement::default(),
        execution_suspended: Cell::new(false),
        worker_tasks_closed: Cell::new(false),
        message_port_dispatches: RefCell::new(BTreeMap::new()),
        stream_transfers: RefCell::new(BTreeMap::new()),
        selection_object: RefCell::new(None),
        implementation_documents: RefCell::new(BTreeMap::new()),
        history: RefCell::new(None),
        navigation_api: RefCell::new(navigation_api::NavigationState::default()),
        joint_history: RefCell::new(Rc::new(RefCell::new(
            crate::joint_history::Timeline::default(),
        ))),
        joint_route: RefCell::new("root".into()),
        joint_attach_step: Cell::new(0),
        pending_history_traversal: RefCell::new(VecDeque::new()),
        storage_fallback: RefCell::new([BTreeMap::new(), BTreeMap::new()]),
        next_platform_object_id: Cell::new(1),
        platform_objects: RefCell::new(BTreeMap::new()),
        canvas_blob_custody: RefCell::new(canvas_encoding::Custody::default()),
        image_bitmap_custody: RefCell::new(canvas_encoding::Custody::default()),
        text_encoding_releases: RefCell::new(Vec::new()),
        location_ancestor_origins: RefCell::new(Vec::new()),
        security_origin: RefCell::new(
            context
                .get_slot::<crate::realm::RealmDescriptor>()
                .map(|descriptor| descriptor.origin.serialize()),
        ),
        location_url: RefCell::new(Url::parse("about:blank").expect("valid initial URL")),
        location_object: Cell::new(None),
        idb_factory_object: Cell::new(None),
        pending_location_navigation: RefCell::new(None),
        url_search_params: RefCell::new(BTreeMap::new()),
        url_iterator_prototype: RefCell::new(None),
        form_data_iterator_prototype: RefCell::new(None),
        headers_iterator_prototype: RefCell::new(None),
        request_constructor: RefCell::new(None),
        platform_wrappers: RefCell::new(BTreeMap::new()),
        platform_edges: RefCell::new(Default::default()),
        idb_payload_ledger: std::sync::Arc::new(Default::default()),
        retained_allocation: RefCell::new(None),
        font_loading: RefCell::new(font_loading::State::default()),
        queued_owned_idb: RefCell::new(VecDeque::new()),
        pending_owned_idb: RefCell::new(Vec::new()),
        platform_host: RefCell::new(None),
        next_platform_operation_id: Cell::new(1),
        pending_platform_operations: RefCell::new(BTreeMap::new()),
        queued_idb_operations: RefCell::new(VecDeque::new()),
        idb_deliveries: Default::default(),
        idb_retry_requests: RefCell::new(BTreeMap::new()),
        idb_blocked_requests: RefCell::new(BTreeSet::new()),
        ready_platform_completions: RefCell::new(VecDeque::new()),
        active_idb_connections: RefCell::new(BTreeMap::new()),
        owned_active_connections: RefCell::new(BTreeMap::new()),
        blocked_idb_completions: RefCell::new(VecDeque::new()),
        deferred_platform_tasks: RefCell::new(VecDeque::new()),
        deferred_task_queue_storage: RefCell::new(None),
        prefer_deferred_platform_tasks: Cell::new(false),
        cancelled_platform_operations: RefCell::new(BTreeSet::new()),
        platform_listeners: RefCell::new(BTreeMap::new()),
        platform_listener_roots: RefCell::new(BTreeMap::new()),
        platform_listener_watchers: RefCell::new(BTreeMap::new()),
        platform_listener_watch_serial: Cell::new(1),
        platform_listener_releases: Rc::new(RefCell::new(Vec::new())),
        global_handler_sources: RefCell::new(BTreeMap::new()),
        platform_handlers: RefCell::new(BTreeMap::new()),
        secure_context: Cell::new(
            context
                .get_slot::<crate::realm::SecureContextState>()
                .is_some_and(|state| state.0),
        ),
        service_workers: RefCell::new(service_workers::Bindings::default()),
        audio_worklet: RefCell::new(audio_worklet::WindowState::default()),
        cache_storage: RefCell::new(cache_storage::Bindings::default()),
        blob_urls: RefCell::new(BTreeMap::new()),
        idb_transactions_to_deactivate: RefCell::new(BTreeSet::new()),
        idb_callback_protection: RefCell::new(BTreeMap::new()),
        next_traversal_id: Cell::new(1),
        node_iterators: RefCell::new(BTreeMap::new()),
        tree_walkers: RefCell::new(BTreeMap::new()),
        layout: RefCell::new(BindingLayout::default()),
        layout_cache_key: RefCell::new(None),
        layout_provider: RefCell::new(None),
        frame_viewport_parent: RefCell::new(None),
        document_focus_admission: Cell::new(None),
        paint_environment: RefCell::new((Vec::new(), 0)),
        page_paint_environment: RefCell::new((Vec::new(), 0)),
        frame_roots: RefCell::new(BTreeMap::new()),
        native_input_frames: RefCell::new(BTreeMap::new()),
        frame_windows: RefCell::new(BTreeMap::new()),
        window_target_name: RefCell::new(Rc::new(
            RefCell::new(window_named::TargetName::default()),
        )),
        frame_target_names: RefCell::new(BTreeMap::new()),
        frame_committed_windows: RefCell::new(BTreeMap::new()),
        intrinsic_reflect_set: adopted_stylesheets::capture_reflect_set(scope)?,
        constructed_stylesheets: RefCell::new(BTreeSet::new()),
        stylesheet_ownership: stylesheet_ownership::Store::default(),
        adopted_stylesheets: RefCell::new(BTreeMap::new()),
        stylesheet_rule_lists: RefCell::new(BTreeMap::new()),
        page_rules: RefCell::new(BTreeMap::new()),
        next_page_rule_id: Cell::new(1),
        native_stylesheets: RefCell::new(NativeStylesheetWrappers::default()),
        current_script: Cell::new(None),
        native_csp_tasks: RefCell::new(VecDeque::new()),
        inline_style_policy: RefCell::new(None),
        snapshot_inline_styles: RefCell::new(BTreeMap::new()),
        inline_style_checks: RefCell::new(BTreeMap::new()),
        script_report_retirement: RefCell::new(None),
        native_support_custody: RefCell::new(None),
        meta_csp_policy: RefCell::new(None),
        local_script_policy_capture: RefCell::new(None),
        local_inline_script_policy: RefCell::new(None),
        script_policy_capture: RefCell::new(None),
        parser_inline_policy: RefCell::new(None),
        document_parser_gate: RefCell::new(None),
        parser_insertion_suppression: Cell::new(None),
        parser_input_active: Cell::new(false),
        design_mode: Cell::new(false),
        focus_transition_revision: Cell::new(0),
        style_with_css: Cell::new(false),
        editing_overrides: RefCell::new(EditingOverrides::default()),
        default_paragraph_separator: RefCell::new("div".into()),
        undo_history: RefCell::new(UndoHistory::default()),
        editing_recovery: RefCell::new(Vec::new()),
        range_clone_completions: RefCell::new(std::collections::VecDeque::new()),
        range_clone_retry: Cell::new(None),
        pending_split_undo: Cell::new(None),
        custom_elements: custom_elements::Registry::default(),
        submit_behaviors: submit_behavior::Registry::default(),
        aria: aria::Registry::default(),
        clipboard_backend: RefCell::new(None),
        native_platform: RefCell::new(native_platform::State::default()),
        reporting: RefCell::new(reporting::State::default()),
        permission_elements: RefCell::new(Default::default()),
        usermedia_legacy: context
            .get_slot::<UserMediaElementProfile>()
            .is_some_and(|profile| profile.legacy),
        native_media: RefCell::new(Default::default()),
        text_tracks: RefCell::new(Default::default()),
        av_tracks: RefCell::new(Default::default()),
        clipboard_operation_active: Cell::new(false),
        clipboard_operation_identity: Cell::new(None),
        keyboard: native_keyboard::State::default(),
        composition: native_composition::State::default(),
        detached_document_parsers: RefCell::new(BTreeMap::new()),
        document_write_roots: RefCell::new(BTreeSet::new()),
        parser_script_depth: Cell::new(0),
        unload_counter: Cell::new(0),
        ignore_destructive_writes: Cell::new(0),
        document_write_depth: Cell::new(0),
        parser_custom_construction_depth: RefCell::new(BTreeMap::new()),
        script_preparation_receipts: RefCell::new(HashMap::new()),
        pending_dynamic_scripts: RefCell::new(VecDeque::new()),
        post_connection_scripts: post_connection_scripts::registry(scope),
        editing_continuations: editing_commands::registry(scope),
        pending_dynamic_stylesheets: RefCell::new(VecDeque::new()),
        prepared_stylesheets: RefCell::new(BTreeMap::new()),
        pending_referrer_policies: RefCell::new(VecDeque::new()),
        document_referrer_policy: RefCell::new("strict-origin-when-cross-origin".into()),
        pending_dynamic_frames: RefCell::new(VecDeque::new()),
        prepared_frames: RefCell::new(BTreeSet::new()),
        frame_referrer_snapshots: RefCell::new(BTreeMap::new()),
        pending_default_actions: RefCell::new(VecDeque::new()),
        reserved_default_actions: Cell::new(0),
        dynamic_import_map: RefCell::new(None),
        planned_submissions: RefCell::new(Vec::new()),
        entry_lists_in_progress: RefCell::new(Vec::new()),
        forms_allowed: Cell::new(true),
        modals_allowed: Cell::new(true),
        storage_access_allowed: Cell::new(true),
        sticky_user_activation: Cell::new(false),
        activation_backend: RefCell::new(None),
        local_activation_backend: RefCell::new(None),
        trusted_policy_check: RefCell::new(None),
        trusted_registry: RefCell::new(TrustedTypesRegistry::new(TrustedTypesConfig {
            allow_any_policy: true,
            allow_duplicates: true,
            ..TrustedTypesConfig::default()
        })),
        trusted_policies: RefCell::new(BTreeMap::new()),
        trusted_values: RefCell::new(BTreeMap::new()),
        trusted_prototypes: RefCell::new(BTreeMap::new()),
        next_trusted_id: Cell::new(1),
        css_images: RefCell::new(css_images::State::default()),
        resource_timing: RefCell::new(resource_timing::State::default()),
        performance_observers: RefCell::new(Default::default()),
        resize_observers: RefCell::new(Default::default()),
        intersection_observers: RefCell::new(Default::default()),
        preload_resources: RefCell::new(preload_resources::State::default()),
        image_sources: RefCell::new(BTreeMap::new()),
        failed_images: RefCell::new(BTreeSet::new()),
        svg_image_origin_clean: range_owner.as_ref().map_or_else(
            || Rc::new(RefCell::new(BTreeMap::new())),
            |owner| owner.svg_image_origin_clean.clone(),
        ),
        image_generations: RefCell::new(BTreeMap::new()),
        images_deferred: Cell::new(false),
        image_selection_cursor: Cell::new(0),
        image_response_urls: RefCell::new(BTreeMap::new()),
        decoded_svg_images: range_owner.as_ref().map_or_else(
            || Rc::new(RefCell::new(BTreeMap::new())),
            |owner| owner.decoded_svg_images.clone(),
        ),
        decoded_images: range_owner.as_ref().map_or_else(
            || Rc::new(RefCell::new(BTreeMap::new())),
            |owner| owner.decoded_images.clone(),
        ),
        canvas_images: range_owner.as_ref().map_or_else(
            || Rc::new(RefCell::new(BTreeMap::new())),
            |owner| owner.canvas_images.clone(),
        ),
        bitmap_renderer_surfaces: range_owner.as_ref().map_or_else(
            || Rc::new(RefCell::new(bitmap_renderer::Surfaces::default())),
            |owner| owner.bitmap_renderer_surfaces.clone(),
        ),
        canvas_draw_states: range_owner.as_ref().map_or_else(
            || Rc::new(RefCell::new(BTreeMap::new())),
            |owner| owner.canvas_draw_states.clone(),
        ),
        canvas_context_settings: range_owner.as_ref().map_or_else(
            || Rc::new(RefCell::new(BTreeMap::new())),
            |owner| owner.canvas_context_settings.clone(),
        ),
        canvas_saved_states: range_owner.as_ref().map_or_else(
            || Rc::new(RefCell::new(BTreeMap::new())),
            |owner| owner.canvas_saved_states.clone(),
        ),
        canvas_paths: range_owner.as_ref().map_or_else(
            || Rc::new(RefCell::new(BTreeMap::new())),
            |owner| owner.canvas_paths.clone(),
        ),
        tainted_canvases: range_owner.as_ref().map_or_else(
            || Rc::new(RefCell::new(BTreeSet::new())),
            |owner| owner.tainted_canvases.clone(),
        ),
        scroll_offsets: RefCell::new(BTreeMap::new()),
    });
    *state.stylesheet_execution_owner.borrow_mut() = Rc::downgrade(&state);
    context.set_slot(state.clone());
    if matches!(
        state.binding_global,
        BindingGlobal::Window | BindingGlobal::Worker
    ) {
        secure_context::install(scope, context)?;
    }
    #[cfg(unix)]
    native_shared_factory::install(scope);
    #[cfg(unix)]
    native_shared_policy::install(scope, context)?;
    if state.binding_global == BindingGlobal::Window {
        origin_agent_cluster::install(scope, context)?;
    }
    if scope.get_slot::<BindingStateRegistry>().is_none() {
        scope.set_slot(BindingStateRegistry::default());
    }
    scope
        .get_slot::<BindingStateRegistry>()
        .expect("binding-state registry was installed")
        .realms
        .borrow_mut()
        .insert(realm, Rc::downgrade(&state));
    scope
        .get_slot::<BindingStateRegistry>()
        .expect("binding-state registry was installed")
        .instances
        .borrow_mut()
        .insert(state.instance_id, Rc::downgrade(&state));

    Ok(state)
}
#[inline(never)]
pub(super) fn install_interfaces(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    state: &Rc<V8BindingState>,
    binding_global: BindingGlobal,
) -> Result<BTreeMap<&'static str, v8::Global<v8::Function>>, V8BindingError> {
    // Installation creates thousands of temporary template/member handles.
    // Do not retain them in the caller's script scope: a script can create
    // hundreds of iframe realms before that scope returns. Published objects
    // are owned by the context graph and the returned constructors are Globals.
    v8::scope!(let scope, scope);
    let mut templates = BTreeMap::<&'static str, v8::Global<v8::FunctionTemplate>>::new();
    let mut constructors = BTreeMap::<&'static str, v8::Global<v8::Function>>::new();
    let mut unforgeable_getters = BTreeMap::<u32, v8::Global<v8::FunctionTemplate>>::new();
    let mut remaining = GENERATED_INTERFACES.iter().enumerate().collect::<Vec<_>>();
    while !remaining.is_empty() {
        let before = remaining.len();
        let mut index = 0;
        while index < remaining.len() {
            let (interface_index, interface) = remaining[index];
            if interface
                .parent
                .is_some_and(|parent| !templates.contains_key(parent))
            {
                index += 1;
                continue;
            }
            let index_data = v8::Integer::new(scope, interface_index as i32);
            let constructor_data: v8::Local<v8::Value> =
                if interface.name.starts_with("HTML") && interface.name.ends_with("Element") {
                    // Callback data survives in the cached instance template.
                    // A primitive has no prototype/native-context retention edge.
                    v8::BigInt::new_from_words(
                        scope,
                        false,
                        &[state.instance_id, interface_index as u64 + 1],
                    )
                    .ok_or(V8BindingError::Allocation("HTML constructor metadata"))?
                    .into()
                } else {
                    index_data.into()
                };
            let constructor_arity = interface.members.iter()
                .filter(|member| member.kind == GeneratedMemberKind::Constructor)
                .map(|member| member.arity).min().unwrap_or(0);
            let template = v8::FunctionTemplate::builder(generated_constructor)
                .data(constructor_data)
                .length(i32::from(constructor_arity))
                .build(scope);
            if interface.name.starts_with("HTML") && interface.name.ends_with("Element") {
                crate::v8_context_bridge::defer_html_constructor_prototype(template);
            }
            template.set_class_name(
                v8::String::new(scope, interface.name)
                    .ok_or(V8BindingError::Allocation("interface name"))?,
            );
            if let Some(parent) = interface.parent {
                let parent = v8::Local::new(
                    scope,
                    templates
                        .get(parent)
                        .ok_or(V8BindingError::MissingInterface(parent))?,
                );
                if interface.name == "Window" {
                    let carrier = window_named::carrier(scope, context, parent)?;
                    template.inherit(carrier);
                    template.prototype_template(scope).set_immutable_proto();
                } else {
                    template.inherit(parent);
                }
            }
            let instance = template.instance_template(scope);
            // DOM wrappers carry the realm-local wrapper lease plus the
            // generation-checked Rust identity required for same-origin
            // cross-realm argument conversion. Stateful non-node interfaces
            // use only field zero.
            instance.set_internal_field_count(6);
            if interface.name == "DOMStringList" {
                stylesheet_sets::configure(instance);
            }
            if interface.name == "Storage" {
                storage_properties::configure(instance);
            }
            if interface.name == "DOMStringMap" {
                dom_string_map::configure(instance);
            }
            if matches!(
                interface.name,
                "NodeList"
                    | "StyleSheetList"
                    | "NamedNodeMap"
                    | "HTMLCollection"
                    | "HTMLFormControlsCollection"
                    | "RadioNodeList"
                    | "HTMLOptionsCollection"
                    | "HTMLSelectElement"
            ) {
                node_lists::configure(instance);
            }
            if interface.name == "DOMTokenList" {
                token_lists::configure(instance);
            }
            if matches!(
                interface.name,
                "CSSStyleDeclaration"
                    | "CSSStyleProperties"
                    | "CSSPageDescriptors"
                    | "CSSFontFaceDescriptors"
            ) {
                inline_styles::configure(instance);
            }
            if interface.name == "Location" {
                location::configure(scope, instance);
            }
            if interface.name == "MediaList" {
                page_rules::media_lists::configure(instance);
            }
            if interface.name == "CSSRuleList" {
                css_rule_lists::configure(instance);
            }
            if interface.name == "DOMRectList" {
                rect_lists::configure(instance);
            }
            if interface.name == "TouchList" {
                touch_objects::configure(instance);
            }
            if matches!(
                interface.name,
                "TextTrackList" | "TextTrackCueList" | "AudioTrackList" | "VideoTrackList"
            ) {
                text_tracks::configure(instance);
            }
            if interface.name == "DataTransferItemList" {
                data_transfer::configure(instance);
            }
            if interface.name == "FileList" {
                file_lists::configure(instance);
            }
            if interface.name == "HTMLAllCollection" {
                node_lists::all::configure(instance);
            }
            if matches!(
                interface.name,
                "HTMLCollection"
                    | "HTMLFormControlsCollection"
                    | "HTMLOptionsCollection"
                    | "NamedNodeMap"
            ) {
                node_lists::configure_named(instance);
            }
            let prototype = template.prototype_template(scope);
            prototype.set_with_attr(
                v8::Symbol::get_to_string_tag(scope).into(),
                v8::String::new(scope, interface.name)
                    .ok_or(V8BindingError::Allocation("interface toStringTag"))?
                    .into(),
                v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_ENUM,
            );
            let mut members = interface.members.iter().collect::<Vec<_>>();
            let mut parent = interface.parent;
            while let Some(name) = parent {
                let inherited = GENERATED_INTERFACES
                    .iter()
                    .find(|item| item.name == name)
                    .ok_or(V8BindingError::MissingInterface(name))?;
                members.extend(inherited.members.iter().filter(|member| member.unforgeable));
                parent = inherited.parent;
            }
            let has_indexed_getter = members.iter().any(|member| member.indexed_getter);
            if has_indexed_getter {
                prototype.set_intrinsic_data_property(
                    v8::Symbol::get_iterator(scope).into(),
                    v8::Intrinsic::ArrayProtoValues,
                    v8::PropertyAttribute::DONT_ENUM,
                );
            }
            for (member_position, member) in members.iter().enumerate() {
                if interface.name == "HTMLUserMediaElement"
                    && (member.name == "isTypeSupported"
                        || !state.usermedia_legacy
                            && matches!(
                                member.name,
                                "type"
                                    | "isValid"
                                    | "invalidReason"
                                    | "onpromptaction"
                                    | "onpromptdismiss"
                                    | "onvalidationstatuschange"
                            ))
                {
                    continue;
                }
                if member.secure_context && !state.secure_context.get() { continue; }
                if !member.exposed.is_empty()
                    && !exposed_on_global(member.exposed, binding_global, context)
                {
                    continue;
                }
                // Anonymous special operations describe interceptors, not
                // callable properties on the interface prototype.
                if member.kind == GeneratedMemberKind::Operation && member.name.is_empty() {
                    continue;
                }
                if members[..member_position]
                    .iter()
                    .any(|previous| previous.name == member.name && previous.kind == member.kind)
                {
                    continue;
                }
                let data = v8::Integer::new(scope, member.id.0 as i32);
                let name = v8::String::new(scope, member.name)
                    .ok_or(V8BindingError::Allocation("member name"))?;
                match member.kind {
                    GeneratedMemberKind::Attribute => {
                        let getter = if let Some(getter) = unforgeable_getters.get(&member.id.0) {
                            v8::Local::new(scope, getter)
                        } else {
                            let getter = v8::FunctionTemplate::builder(generated_attribute_getter)
                                .data(data.into())
                                .length(0)
                                .constructor_behavior(v8::ConstructorBehavior::Throw)
                                .build(scope);
                            getter.set_class_name(
                                v8::String::new(scope, &format!("get {}", member.name))
                                    .ok_or(V8BindingError::Allocation("attribute getter name"))?,
                            );
                            if member.unforgeable {
                                unforgeable_getters
                                    .insert(member.id.0, v8::Global::new(scope, getter));
                            }
                            getter
                        };
                        let setter = (!member.readonly
                            || member.replaceable
                            || member.put_forwards.is_some())
                        .then(|| {
                            v8::FunctionTemplate::builder(generated_attribute_setter)
                                .data(data.into())
                                .length(1)
                                .constructor_behavior(v8::ConstructorBehavior::Throw)
                                .build(scope)
                        });
                        if let Some(setter) = setter.as_ref() {
                            setter.set_class_name(
                                v8::String::new(scope, &format!("set {}", member.name))
                                    .ok_or(V8BindingError::Allocation("attribute setter name"))?,
                            );
                        }
                        if member.static_member {
                            template.set_accessor_property(
                                name.into(),
                                Some(getter),
                                setter,
                                v8::PropertyAttribute::NONE,
                            );
                            continue;
                        }
                        // [Global] attributes are installed on the actual global,
                        // never on this interface's prototype. Inherited attributes
                        // remain on their non-global declaring prototype.
                        if !interface.global_names.is_empty() && !member.unforgeable {
                            continue;
                        }
                        let target = if member.unforgeable {
                            instance
                        } else {
                            prototype
                        };
                        target.set_accessor_property(
                            name.into(),
                            Some(getter),
                            setter,
                            if member.unforgeable {
                                v8::PropertyAttribute::DONT_DELETE
                            } else {
                                v8::PropertyAttribute::NONE
                            },
                        );
                    }
                    GeneratedMemberKind::Operation => {
                        // A [Global] declaration is published on the actual
                        // global by generated_globals, not its prototype.
                        // Nonglobal ancestors retain their own prototype members.
                        if !member.static_member && !member.unforgeable
                            && !interface.global_names.is_empty() {
                            continue;
                        }
                        let arity = interface
                            .members
                            .iter()
                            .filter(|overload| {
                                overload.kind == GeneratedMemberKind::Operation
                                    && overload.name == member.name
                                    && overload.static_member == member.static_member
                            })
                            .map(|overload| overload.arity)
                            .min()
                            .unwrap_or(member.arity);
                        let function = v8::FunctionTemplate::builder(generated_operation)
                            .data(data.into())
                            .length(arity.into())
                            .constructor_behavior(v8::ConstructorBehavior::Throw)
                            .build(scope);
                        if member.static_member {
                            template.set(name.into(), function.into());
                        } else if member.unforgeable {
                            instance.set_with_attr(
                                name.into(),
                                function.into(),
                                v8::PropertyAttribute::READ_ONLY
                                    | v8::PropertyAttribute::DONT_DELETE,
                            );
                        } else {
                            prototype.set(name.into(), function.into());
                        }
                    }
                    GeneratedMemberKind::Constant => {
                        let value = generated_constant_value(
                            scope,
                            member
                                .constant_value
                                .ok_or(V8BindingError::Allocation("WebIDL constant value"))?,
                        )?;
                        prototype.set_with_attr(
                            name.into(),
                            value.into(),
                            v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_DELETE,
                        );
                    }
                    GeneratedMemberKind::Setlike => {
                        if interface.name != "FontFaceSet" {
                            return Err(V8BindingError::Unsupported {
                                interface: interface.name.into(), member: "native setlike installer".into(),
                            });
                        }
                        font_loading::install_setlike(scope, prototype)?;
                    }
                    GeneratedMemberKind::Iterable => {
                        if interface.name == "Headers" {
                            headers_iterators::install_iterable(scope, prototype)?;
                            continue;
                        }
                        if interface.name == "FormData" {
                            form_data_iterators::install_iterable(scope, prototype)?;
                            continue;
                        }
                        if interface.name == "URLSearchParams" {
                            url_objects::install_iterable(scope, prototype)?;
                            continue;
                        }
                        if member.value_iterable && has_indexed_getter {
                            // Web IDL value iterators use the realm's original
                            // Array intrinsics: https://webidl.spec.whatwg.org/#es-iterators
                            for (name, intrinsic) in [
                                ("entries", v8::Intrinsic::ArrayProtoEntries),
                                ("keys", v8::Intrinsic::ArrayProtoKeys),
                                ("values", v8::Intrinsic::ArrayProtoValues),
                                ("forEach", v8::Intrinsic::ArrayProtoForEach),
                            ] {
                                let name = v8::String::new(scope, name)
                                    .ok_or(V8BindingError::Allocation("iterator method name"))?;
                                prototype.set_intrinsic_data_property(
                                    name.into(),
                                    intrinsic,
                                    v8::PropertyAttribute::NONE,
                                );
                            }
                            continue;
                        }
                        let function = v8::FunctionTemplate::builder(generated_operation)
                            .data(data.into())
                            .length(0)
                            .constructor_behavior(v8::ConstructorBehavior::Throw)
                            .build(scope);
                        prototype.set(v8::Symbol::get_iterator(scope).into(), function.into());
                    }
                    GeneratedMemberKind::Constructor => {}
                }
            }
            if interface.callback {
                template.remove_prototype();
            }
            let constructor = template
                .get_function(scope)
                .ok_or(V8BindingError::MissingInterface(interface.name))?;
            if interface.name == "DOMQuad" {
                for method in ["fromRect", "fromQuad"] {
                    let key = v8::String::new(scope, method)
                        .ok_or(V8BindingError::Allocation("quad factory"))?;
                    let function = v8::Function::builder(quad_factory)
                        .data(key.into())
                        .length(0)
                        .constructor_behavior(v8::ConstructorBehavior::Throw)
                        .build(scope)
                        .ok_or(V8BindingError::Allocation("quad factory"))?;
                    set_object_property(scope, constructor.into(), method, function.into())?;
                }
            }
            if matches!(
                interface.name,
                "DOMRect"
                    | "DOMRectReadOnly"
                    | "DOMPoint"
                    | "DOMPointReadOnly"
                    | "DOMMatrix"
                    | "DOMMatrixReadOnly"
            ) {
                let method = if interface.name.starts_with("DOMRect") {
                    "fromRect"
                } else if interface.name.starts_with("DOMPoint") {
                    "fromPoint"
                } else {
                    "fromMatrix"
                };
                let data = v8::String::new(scope, interface.name)
                    .ok_or(V8BindingError::Allocation("geometry factory"))?;
                let function = v8::Function::builder(geometry_dictionary_factory)
                    .data(data.into())
                    .constructor_behavior(v8::ConstructorBehavior::Throw)
                    .build(scope)
                    .ok_or(V8BindingError::Allocation("geometry factory"))?;
                set_object_property(scope, constructor.into(), method, function.into())?;
            }
            if matches!(interface.name, "DOMMatrix" | "DOMMatrixReadOnly") {
                for method in ["fromFloat32Array", "fromFloat64Array"] {
                    let data = v8::String::new(scope, &format!("{}:{method}", interface.name))
                        .ok_or(V8BindingError::Allocation("matrix array factory"))?;
                    let function = v8::Function::builder(matrix_typed_array_factory)
                        .data(data.into())
                        .length(1)
                        .constructor_behavior(v8::ConstructorBehavior::Throw)
                        .build(scope)
                        .ok_or(V8BindingError::Allocation("matrix array factory"))?;
                    set_object_property(scope, constructor.into(), method, function.into())?;
                }
            }
            if interface.name == "Request" {
                let retained = context_intrinsics::retain_function(
                    scope,
                    &format!("request-constructor-{}", state.instance_id),
                    constructor,
                )?;
                state.request_constructor.borrow_mut().replace(retained);
            }
            let prototype_name = v8::String::new(scope, "prototype")
                .ok_or(V8BindingError::Allocation("prototype name"))?;
            let interface_prototype = constructor
                .get(scope, prototype_name.into())
                .ok_or(V8BindingError::Allocation("interface prototype"))?;
            if interface.name.starts_with("HTML") && interface.name.ends_with("Element") {
                let prototype = interface_prototype
                    .try_cast::<v8::Object>()
                    .map_err(|_| V8BindingError::Allocation("HTML interface prototype"))?;
                state
                    .html_prototypes
                    .borrow_mut()
                    .insert(interface.name, v8::Weak::new(scope, prototype));
                let key =
                    v8::String::new(scope, &format!("oryn.html.prototype.{}", interface.name))
                        .ok_or(V8BindingError::Allocation("HTML prototype graph"))?;
                let key = v8::Private::for_api(scope, Some(key));
                context
                    .global(scope)
                    .set_private(scope, key, prototype.into())
                    .ok_or(V8BindingError::JavaScriptException)?;
            }

            if interface.name == "DOMRectList" {
                let prototype = v8::Local::<v8::Object>::try_from(interface_prototype)
                    .map_err(|_| V8BindingError::Allocation("rect list prototype"))?;
                rect_lists::install(scope, prototype)?;
            }
            if interface.name == "DOMQuad" {
                let prototype = v8::Local::<v8::Object>::try_from(interface_prototype)
                    .map_err(|_| V8BindingError::Allocation("quad prototype"))?;
                install_quad_members(scope, prototype)?;
            }
            if matches!(interface.name, "DOMRect" | "DOMRectReadOnly") {
                let prototype = v8::Local::<v8::Object>::try_from(interface_prototype)
                    .map_err(|_| V8BindingError::Allocation("rect prototype"))?;
                install_rect_members(scope, prototype, interface.name == "DOMRectReadOnly")?;
            }
            if matches!(interface.name, "DOMPoint" | "DOMPointReadOnly") {
                let prototype = v8::Local::<v8::Object>::try_from(interface_prototype)
                    .map_err(|_| V8BindingError::Allocation("point prototype"))?;
                install_point_members(scope, prototype, interface.name == "DOMPointReadOnly")?;
            }
            if matches!(interface.name, "DOMMatrix" | "DOMMatrixReadOnly") {
                let prototype = v8::Local::<v8::Object>::try_from(interface_prototype)
                    .map_err(|_| V8BindingError::Allocation("matrix prototype"))?;
                install_matrix_members(scope, prototype, interface.name == "DOMMatrixReadOnly")?;
            }
            if interface.name == "Path2D" {
                let prototype = v8::Local::<v8::Object>::try_from(interface_prototype)
                    .map_err(|_| V8BindingError::Allocation("Path2D prototype"))?;
                install_path2d_members(scope, prototype)?;
            }
            if matches!(
                interface.name,
                "CanvasRenderingContext2D" | "OffscreenCanvasRenderingContext2D"
            ) {
                let prototype = v8::Local::<v8::Object>::try_from(interface_prototype)
                    .map_err(|_| V8BindingError::Allocation("canvas context prototype"))?;
                install_canvas_context_members(scope, prototype)?;
                restore_native_member_metadata(scope, prototype, interface)?;
            }
            if interface.name == "OffscreenCanvas" {
                let prototype = v8::Local::<v8::Object>::try_from(interface_prototype)
                    .map_err(|_| V8BindingError::Allocation("OffscreenCanvas prototype"))?;
                offscreen_canvas::install_members(scope, prototype)?;
                restore_native_member_metadata(scope, prototype, interface)?;
            }
            if interface.name == "ImageBitmapRenderingContext" {
                let prototype = v8::Local::<v8::Object>::try_from(interface_prototype)
                    .map_err(|_| V8BindingError::Allocation("bitmap renderer prototype"))?;
                bitmap_renderer::install_members(scope, prototype)?;
                restore_native_member_metadata(scope, prototype, interface)?;
            }
            if matches!(
                interface.name,
                "Element"
                    | "Document"
                    | "Animation"
                    | "AnimationTimeline"
                    | "AnimationEffect"
                    | "KeyframeEffect"
                    | "CSSAnimation"
                    | "CSSTransition"
                    | "AnimationPlaybackEvent"
                    | "AnimationEvent"
                    | "TransitionEvent"
            ) {
                let prototype = v8::Local::<v8::Object>::try_from(interface_prototype)
                    .map_err(|_| V8BindingError::Allocation("animation prototype"))?;
                web_animations::install(scope, prototype, interface.name)?;
            }
            if interface.name == "Notification" {
                let prototype = v8::Local::<v8::Object>::try_from(interface_prototype)
                    .map_err(|_| V8BindingError::Allocation("Notification prototype"))?;
                web_notifications::install(
                    scope,
                    constructor,
                    prototype,
                    state.binding_global == BindingGlobal::Window,
                )?;
            }
            if matches!(interface.name, "MediaSource" | "ManagedMediaSource") {
                let prototype = v8::Local::<v8::Object>::try_from(interface_prototype)
                    .map_err(|_| V8BindingError::Allocation("MediaSource prototype"))?;
                media_source::install_members(
                    scope,
                    prototype,
                    state.binding_global == BindingGlobal::Worker,
                )?;
            }
            if interface.name == "AudioData" {
                let prototype = v8::Local::<v8::Object>::try_from(interface_prototype)
                    .map_err(|_| V8BindingError::Allocation("AudioData prototype"))?;
                audio_data::install_members(scope, prototype)?;
            }
            if interface.name == "VideoFrame" {
                let prototype = v8::Local::<v8::Object>::try_from(interface_prototype)
                    .map_err(|_| V8BindingError::Allocation("VideoFrame prototype"))?;
                video_frame::install_members(scope, prototype)?;
            }
            if interface.name == "ImageBitmap" {
                let prototype = v8::Local::<v8::Object>::try_from(interface_prototype)
                    .map_err(|_| V8BindingError::Allocation("ImageBitmap prototype"))?;
                image_bitmap::install_members(scope, prototype)?;
            }
            if interface.name == "ImageData" {
                let prototype = v8::Local::<v8::Object>::try_from(interface_prototype)
                    .map_err(|_| V8BindingError::Allocation("ImageData prototype"))?;
                install_image_data_members(scope, prototype)?;
            }
            if interface.name == "CustomStateSet" {
                let prototype = v8::Local::<v8::Object>::try_from(interface_prototype)
                    .map_err(|_| V8BindingError::Allocation("custom state prototype"))?;
                let values = v8::String::new(scope, "values")
                    .ok_or(V8BindingError::Allocation("values key"))?;
                let function = prototype
                    .get(scope, values.into())
                    .ok_or(V8BindingError::Allocation("values method"))?;
                let keys =
                    v8::String::new(scope, "keys").ok_or(V8BindingError::Allocation("keys key"))?;
                prototype.set(scope, keys.into(), function);
                let iterator = v8::Symbol::get_iterator(scope);
                prototype.set(scope, iterator.into(), function);
            }
            if !interface.callback {
                constructor.define_own_property(
                    scope,
                    prototype_name.into(),
                    interface_prototype,
                    v8::PropertyAttribute::READ_ONLY
                        | v8::PropertyAttribute::DONT_ENUM
                        | v8::PropertyAttribute::DONT_DELETE,
                );
            }
            if let Some(parent) = interface.parent {
                // A child exposed in Worker can inherit from a parent whose
                // constructor is intentionally not exposed on that global.
                // Prototype linkage must therefore use the generated parent
                // constructor registry, not a lookup through globalThis.
                let parent_constructor = v8::Local::new(
                    scope,
                    constructors
                        .get(parent)
                        .ok_or(V8BindingError::MissingInterface(parent))?,
                );
                constructor.set_prototype(scope, parent_constructor.into());
                let child_prototype = constructor
                    .get(scope, prototype_name.into())
                    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                    .ok_or(V8BindingError::Allocation("child interface prototype"))?;
                let parent_prototype = parent_constructor
                    .get(scope, prototype_name.into())
                    .ok_or(V8BindingError::Allocation("parent interface prototype"))?;
                if interface.name != "Window" {
                    child_prototype.set_prototype(scope, parent_prototype);
                }
            }
            let unscopable_members = interface
                .members
                .iter()
                .filter(|member| member.unscopable)
                .collect::<Vec<_>>();
            if !unscopable_members.is_empty() {
                let prototype = constructor
                    .get(scope, prototype_name.into())
                    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                    .ok_or(V8BindingError::Allocation("unscopables prototype"))?;
                let unscopables = v8::Object::new(scope);
                unscopables.set_prototype(scope, v8::null(scope).into());
                for member in unscopable_members {
                    let name = v8::String::new(scope, member.name)
                        .ok_or(V8BindingError::Allocation("unscopable member name"))?;
                    unscopables.create_data_property(
                        scope,
                        name.into(),
                        v8::Boolean::new(scope, true).into(),
                    );
                }
                prototype.define_own_property(
                    scope,
                    v8::Symbol::get_unscopables(scope).into(),
                    unscopables.into(),
                    v8::PropertyAttribute::DONT_ENUM,
                );
            }
            for member in interface
                .members
                .iter()
                .filter(|member| member.kind == GeneratedMemberKind::Constant)
            {
                let name = v8::String::new(scope, member.name)
                    .ok_or(V8BindingError::Allocation("WebIDL constant name"))?;
                let value = generated_constant_value(
                    scope,
                    member
                        .constant_value
                        .ok_or(V8BindingError::Allocation("WebIDL constant value"))?,
                )?;
                constructor.define_own_property(
                    scope,
                    name.into(),
                    value,
                    v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_DELETE,
                );
            }
            if (!interface.secure_context || state.secure_context.get())
                && exposed_on_global(interface.exposed, binding_global, context) {
                let global = context.global(scope);
                global.define_own_property(
                    scope,
                    v8::String::new(scope, interface.name).unwrap().into(),
                    constructor.into(),
                    v8::PropertyAttribute::DONT_ENUM,
                );
            }
            state
                .instance_templates
                .borrow_mut()
                .insert(interface.name, v8::Weak::new(scope, instance));
            if interface.name.starts_with("HTML") && interface.name.ends_with("Element") {
                state
                    .html_constructors
                    .borrow_mut()
                    .insert(interface.name, v8::Weak::new(scope, constructor));
            }
            // The Context owns original constructors through its traced graph.
            // Native template caches must not turn that graph into a GC root.
            context_intrinsics::retain_function_in_context(
                scope,
                context,
                &format!(
                    "owner-{}-instance-constructor-{}",
                    state.instance_id, interface.name
                ),
                constructor,
            )?;
            constructors.insert(interface.name, v8::Global::new(scope, constructor));
            templates.insert(interface.name, v8::Global::new(scope, template));
            remaining.remove(index);
        }
        if before == remaining.len() {
            return Err(V8BindingError::MissingInterface(
                remaining[0].1.parent.unwrap_or(remaining[0].1.name),
            ));
        }
    }

    Ok(constructors)
}
#[inline(never)]
pub(super) fn publish_objects(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    state: &Rc<V8BindingState>,
    document: NodeId,
    binding_global: BindingGlobal,
    constructors: &BTreeMap<&'static str, v8::Global<v8::Function>>,
) -> Result<(), V8BindingError> {
    generated_globals::install(scope, context, state, binding_global, constructors)?;
    web_crypto::install(scope, state, context, constructors)?;
    submit_behavior::install(scope, state, constructors)?;
    if binding_global == BindingGlobal::Window {
        // Establish the document wrapper graph at installation, as for the
        // other realm-owned objects; the accessor must not defer this lifetime.
        wrap_node(scope, state, document)?;
        window_document::install(scope, context)?;
        storage_access::install(scope, context)?;
        window_messages::install(scope, context)?;
        prompts::install(scope, context)?;
        async_clipboard::install(scope, context)?;
        native_user_activation::install(scope, context)?;
        native_platform::install(scope, context)?;
        reporting::install(scope, context)?;
        fetch_later::install(scope, context, state)?;
        native_media::install(scope, context)?;
        service_workers::install(scope, context)?;
        cache_storage::install(scope, state)?;
        navigation_api::install(scope, state)?;
        publish_named_window_properties(scope, context, state, document)?;
        for (name, id) in [("localStorage", 1), ("sessionStorage", 2)] {
            let storage = new_stateful_object(scope, state, "Storage", id)?;
            storage_events::retain(scope, context, storage, id == 1)?;
            storage_events::install_accessor(scope, context, name, id == 1)?;
        }
        let restore = v8::Function::new(scope, restore_storage_callback)
            .ok_or(V8BindingError::Allocation("storage restore function"))?;
        let name = v8::String::new(scope, "__oryn_restore_storage")
            .ok_or(V8BindingError::Allocation("storage restore name"))?;
        context
            .global(scope)
            .set(scope, name.into(), restore.into());
    }
    let now = v8::Function::new(scope, performance_now_callback)
        .ok_or(V8BindingError::Allocation("performance clock"))?;
    let now_name = v8::String::new(scope, "__oryn_performance_now")
        .ok_or(V8BindingError::Allocation("performance clock name"))?;
    context
        .global(scope)
        .set(scope, now_name.into(), now.into());
    let origin_name = v8::String::new(scope, "__oryn_performance_time_origin")
        .ok_or(V8BindingError::Allocation("performance origin name"))?;
    context.global(scope).set(
        scope,
        origin_name.into(),
        v8::Number::new(scope, state.time_origin_ms.get()).into(),
    );
    resource_timing::install(scope, state, context)?;
    install_dom_exception_intrinsics(scope, context)?;
    // These constructors are exposed only to Window and Worker by the IDL.
    if matches!(
        binding_global,
        BindingGlobal::Window | BindingGlobal::Worker
    ) {
        install_abort_signal_statics(scope, context)?;
        install_url_statics(scope, context)?;
        install_response_statics(scope, context)?;
    }
    if binding_global == BindingGlobal::Window {
        install_document_statics(scope, context)?;
        selection_bindings::install_window(scope, context)?;
        let media = v8::Function::new(scope, native_media_matches_callback)
            .ok_or(V8BindingError::Allocation("native media callback"))?;
        let key = v8::String::new(scope, "__oryn_match_media")
            .ok_or(V8BindingError::Allocation("native media callback name"))?;
        context.global(scope).set(scope, key.into(), media.into());
        let key = v8::String::new(scope, "matchMedia")
            .ok_or(V8BindingError::Allocation("matchMedia name"))?;
        context.global(scope).set(scope, key.into(), media.into());

        install_script_statics(scope, context)?;
        permission_elements::install_legacy_statics(scope, context, state)?;
        let get_computed_style = v8::Function::new(scope, get_computed_style_callback)
            .ok_or(V8BindingError::Allocation("getComputedStyle function"))?;
        let name = v8::String::new(scope, "getComputedStyle")
            .ok_or(V8BindingError::Allocation("getComputedStyle name"))?;
        context
            .global(scope)
            .set(scope, name.into(), get_computed_style.into());
        let get_accessibility = v8::Function::new(scope, get_computed_accessibility_callback)
            .ok_or(V8BindingError::Allocation(
                "computed accessibility function",
            ))?;
        let name = v8::String::new(scope, "__oryn_computed_accessibility")
            .ok_or(V8BindingError::Allocation("computed accessibility name"))?;
        context
            .global(scope)
            .set(scope, name.into(), get_accessibility.into());
        install_image_factory(scope, context)?;
        install_option_factory(scope, context)?;
        install_audio_factory(scope, context)?;
    }
    if matches!(
        binding_global,
        BindingGlobal::Window | BindingGlobal::Worker
    ) {
        install_trusted_types(scope, context)?;
    }
    let fetch = v8::Function::new(scope, platform_fetch_callback)
        .ok_or(V8BindingError::Allocation("fetch function"))?;
    let name = v8::String::new(scope, "fetch").ok_or(V8BindingError::Allocation("fetch name"))?;
    context.global(scope).set(scope, name.into(), fetch.into());
    let create_object_url = v8::Function::new(scope, blob_url_create_callback)
        .ok_or(V8BindingError::Allocation("Blob URL create function"))?;
    let name = v8::String::new(scope, "__oryn_blob_url_create")
        .ok_or(V8BindingError::Allocation("Blob URL create name"))?;
    context
        .global(scope)
        .set(scope, name.into(), create_object_url.into());
    let revoke_object_url = v8::Function::new(scope, blob_url_revoke_callback)
        .ok_or(V8BindingError::Allocation("Blob URL revoke function"))?;
    let name = v8::String::new(scope, "__oryn_blob_url_revoke")
        .ok_or(V8BindingError::Allocation("Blob URL revoke name"))?;
    context
        .global(scope)
        .set(scope, name.into(), revoke_object_url.into());
    if binding_global == BindingGlobal::Window {
        custom_elements::install(scope, state)?;
    }
    global_events::install(scope, context)?;
    structured_clone::install(scope, context)?;
    if matches!(
        binding_global,
        BindingGlobal::Window | BindingGlobal::Worker
    ) {
        browser_base64::install(scope, context)?;
    }
    image_bitmap::install(scope, context)?;
    css_registration::install(scope, context)?;
    if binding_global == BindingGlobal::Window {
        location::install(scope, state)?;
        window_scroll::install(scope, context)?;
    }
    Ok(())
}

#[cfg(test)]
mod native_stack_tests {
    use super::*;
    struct CreatedRealms(Rc<Cell<u32>>);

    fn create_child(
        scope: &mut v8::PinScope,
        args: v8::FunctionCallbackArguments,
        mut rv: v8::ReturnValue,
    ) {
        let Some(depth) = args.get(0).uint32_value(scope) else {
            return;
        };
        match install_child(scope, depth) {
            Ok(()) => rv.set(v8::Boolean::new(scope, true).into()),
            Err(error) => throw_binding_error(scope, &error),
        }
    }

    fn install_child(scope: &mut v8::PinScope, depth: u32) -> Result<(), V8BindingError> {
        let count = scope.get_slot::<CreatedRealms>().unwrap().0.clone();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>").into_runtime()?,
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(depth + 100), 1)?;
        count.set(count.get() + 1);
        // Force the exact V8 central-stack check while ancestor DOM operation
        // callbacks remain active; do not change V8's stack configuration.
        scope.low_memory_notification();
        let callback = v8::Function::new(scope, create_child)
            .ok_or(V8BindingError::Allocation("nested realm callback"))?;
        let key = v8::String::new(scope, "createChild")
            .ok_or(V8BindingError::Allocation("nested realm key"))?;
        context
            .global(scope)
            .set(scope, key.into(), callback.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        if depth != 0 {
            let source = format!(
                "customElements.define('x-stack',class extends HTMLElement{{connectedCallback(){{if(!createChild({}))throw Error('nested realm failed')}}}});document.body.appendChild(document.createElement('x-stack'));document.body.firstChild.localName==='x-stack'",
                depth - 1
            );
            let source = v8::String::new(scope, &source)
                .ok_or(V8BindingError::Allocation("nested realm source"))?;
            let result = v8::Script::compile(scope, source, None)
                .and_then(|script| script.run(scope))
                .ok_or(V8BindingError::JavaScriptException)?;
            if !result.is_true() {
                return Err(V8BindingError::Allocation("nested realm insertion"));
            }
        }
        Ok(())
    }

    #[test]
    fn nested_dom_callbacks_install_realms_and_collect_on_the_central_stack() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let count = Rc::new(Cell::new(0));
        scope.set_slot(CreatedRealms(count.clone()));
        install_child(scope, 6).unwrap();
        assert_eq!(
            count.get(),
            7,
            "every connected callback completed its child realm"
        );
    }
}
