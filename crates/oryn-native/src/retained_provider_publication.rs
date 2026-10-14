fn publish_ready_retained_stylesheet(
    completion: &mut RetainedParserCompletion,
    host: &mut RawV8Host,
    arena: &SharedDomRuntime,
) -> Result<bool, HostError> {
    let identity = completion.authority.document;
    let Some(provider) = host.native_layout_provider_in_realm(identity.realm) else {
        completion.cancel_pending();
        return Ok(true);
    };
    let Some(backend) = provider
        .native_backend()
        .and_then(|backend| backend.downcast_ref::<LiveLayoutProvider>())
    else {
        return Err(HostError::Execution(
            "retained stylesheet requires native owning provider".into(),
        ));
    };
    if !Rc::ptr_eq(&backend.dom, arena)
        || backend.native_document_identity.get().is_none_or(|owner| {
            owner.realm != identity.realm
                || owner.document != identity.document
                || owner.document_generation != identity.document_generation
                || owner.stream_epoch != identity.stream_epoch
        })
    {
        completion.cancel_pending();
        return Ok(true);
    }
    let allocation = backend
        .native_retained_allocation
        .try_borrow()
        .map_err(|_| HostError::Execution("retained publication receipt borrow conflict".into()))?
        .as_ref()
        .cloned()
        .ok_or_else(|| {
            HostError::Execution("retained publication allocation receipt missing".into())
        })?;
    host.refresh_native_retained_allocation(identity.realm, &allocation)?;
    let _receipt_scope = allocation.scope_guard();
    let admission = backend
        .retained_revision_admission()
        .map_err(HostError::Execution)?;
    let quirks = arena
        .try_borrow()
        .map_err(|_| HostError::Execution("retained stylesheet quirks borrow conflict".into()))?
        .arena()
        .get(identity.document)
        .map_err(|error| HostError::Execution(error.to_string()))?
        .document_metadata
        .as_ref()
        .is_some_and(|metadata| metadata.quirks);
    // Completion delivery can occur while another frame is incumbent. Native
    // event and dependency queues are realm-scoped, so use the validated owner
    // rather than whichever author realm last executed on this host.
    let previous_realm = host.active_realm();
    host.activate_realm(identity.realm)?;
    let publication = apply_retained_stylesheet_ready(
        completion,
        host,
        arena,
        &backend.retained_stylesheets,
        backend.style_lock.clone(),
        quirks,
        admission,
    );
    let invalidation = allocation.invalidate().map_err(HostError::Execution);
    let restoration = host.activate_realm(previous_realm);
    // Preserve the primary publication diagnostic if restoration also fails.
    match publication {
        Err(error) => Err(error),
        Ok(published) => invalidation.and(restoration).map(|()| published),
    }
}
