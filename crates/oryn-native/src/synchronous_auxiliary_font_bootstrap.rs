// Called in the admitted auxiliary's own ContextScope before returning its Window.
pub(crate) fn bootstrap_auxiliary_public_fonts(
    scope: &mut v8::PinScope,
    creator: v8::Local<v8::Context>,
    state: &Rc<oryn_js::v8_bindings::V8BindingState>,
    realm: RealmId,
) -> Result<(), HostError> {
    let layout = state
        .native_layout_provider()
        .ok_or(HostError::CachedAuthorityExpired)?;
    let backend = layout
        .native_backend()
        .and_then(|value| value.downcast_ref::<LiveLayoutProvider>())
        .ok_or_else(|| HostError::Execution("auxiliary font provider unavailable".into()))?;
    let provider_identity = Rc::as_ptr(&layout) as *const () as usize;
    let identity = ParserDocumentIdentity {
        realm,
        document: state.document_root(),
        document_generation: state.document_generation(),
        stream_epoch: state.document_stream_epoch(),
    };
    layout.bind_stylesheet_document_owner(&state.dom().borrow(),
        oryn_js::v8_bindings::BindingStylesheetDocumentOwner {
            realm: identity.realm,
            document: identity.document,
            document_generation: identity.document_generation,
            stream_epoch: identity.stream_epoch,
        }).map_err(HostError::Execution)?;
    // Refresh the original creator's already installed receipt before allocating
    // child bootstrap metadata. The temporary ContextScope is released before
    // capture resumes in the child's owning scope.
    let creator_state = creator
        .get_slot::<oryn_js::v8_bindings::V8BindingState>()
        .ok_or(HostError::CachedAuthorityExpired)?;
    let creator_layout = creator_state
        .native_layout_provider()
        .ok_or(HostError::CachedAuthorityExpired)?;
    let creator_backend = creator_layout
        .native_backend()
        .and_then(|value| value.downcast_ref::<LiveLayoutProvider>())
        .ok_or(HostError::CachedAuthorityExpired)?;
    let original = creator_backend
        .native_retained_allocation
        .try_borrow()
        .map_err(|_| HostError::Execution("creator bootstrap receipt borrow".into()))?
        .as_ref()
        .cloned()
        .ok_or(HostError::CachedAuthorityExpired)?;
    let _creator_receipt = original.scope_guard();
    {
        let creator_scope = &mut v8::ContextScope::new(scope, creator);
        original
            .refresh(creator_scope)
            .map_err(|error| HostError::Execution(error.to_string()))?;
    }
    fn arc_bytes<T>() -> Result<usize, HostError> {
        std::alloc::Layout::new::<[std::sync::atomic::AtomicUsize; 2]>()
            .extend(std::alloc::Layout::new::<T>())
            .map(|(layout, _)| layout.pad_to_align().size())
            .map_err(|_| HostError::Execution("auxiliary Arc layout overflow".into()))
    }
    let bytes = arc_bytes::<crate::context_service::ResourceCancellation>()?
        .checked_add(arc_bytes::<std::sync::atomic::AtomicBool>()?)
        .and_then(|bytes| {
            bytes.checked_add(arc_bytes::<oryn_js::v8_bindings::BindingRetainedAllocation>().ok()?)
        })
        .and_then(|bytes| bytes.checked_add(arc_bytes::<AuxiliaryBootstrapAdmission>().ok()?))
        .ok_or_else(|| HostError::Execution("auxiliary bootstrap metadata overflow".into()))?;
    let bootstrap_charge = original.reserve(bytes).map_err(HostError::Execution)?;
    let cancellation = Arc::new(crate::context_service::ResourceCancellation::default());
    let captured = state
        .capture_retained_allocation(scope, cancellation.flag.clone())
        .map_err(|error| HostError::Execution(error.to_string()))?;
    let allocation = Arc::new(captured.with_metadata_owner(bootstrap_charge.clone()));
    let shared: SharedRetainedAdmission = Arc::new(AuxiliaryBootstrapAdmission {
        inner: NativeRetainedAdmission {
            allocation: allocation.clone(),
            cancellation,
        },
        _metadata: bootstrap_charge,
    });
    let _receipt = allocation.scope_guard();
    let mut prepared = backend
        .prepare_shared_retained_admission(shared.clone(), allocation.clone())
        .map_err(HostError::Execution)?;
    state
        .install_retained_allocation_with(allocation, || {
            backend.finish_shared_retained_admission(&mut prepared)
        })
        .map_err(|error| HostError::Execution(error.to_string()))?;
    let charge = shared
        .reserve(
            std::mem::size_of::<NativeProviderAdmissionFromShared>()
                + 2 * std::mem::size_of::<usize>(),
        )
        .map_err(HostError::Execution)?;
    let admission = Rc::new(NativeProviderAdmissionFromShared {
        shared,
        _charge: charge,
    });
    let backing = backend
        .implicit_font_store(admission.as_ref())
        .map_err(HostError::Execution)?;
    let metadata = admission
        .reserve(
            std::mem::size_of::<NativeFontLoadingProvider>() + 2 * std::mem::size_of::<usize>(),
        )
        .map_err(HostError::Execution)?;
    if state.execution_retired()
        || state.document_root() != identity.document
        || state.document_generation() != identity.document_generation
        || state.document_stream_epoch() != identity.stream_epoch
        || !state
            .native_layout_provider()
            .is_some_and(|current| Rc::ptr_eq(&current, &layout))
    {
        return Err(HostError::CachedAuthorityExpired);
    }
    state.install_native_font_loading(Rc::new(NativeFontLoadingProvider {
        dom: state.dom(),
        registry: backend.retained_stylesheets.clone(),
        identity,
        admission,
        _metadata: metadata,
        backing,
        provider_identity,
        provider: Rc::downgrade(&layout),
        state: Rc::downgrade(state),
    }));
    Ok(())
}

// Payload fields precede the reservation owner, so their destructor runs first.
struct AuxiliaryBootstrapAdmission {
    inner: NativeRetainedAdmission,
    _metadata: Arc<dyn Send + Sync>,
}
impl oryn_layout::cssom::native_import_graph::ImportAdmission for AuxiliaryBootstrapAdmission {
    fn checkpoint(&self) -> Result<(), String> {
        oryn_layout::cssom::native_import_graph::ImportAdmission::checkpoint(&self.inner)
    }
    fn reserve(&self, bytes: usize) -> Result<Arc<dyn Send + Sync>, String> {
        oryn_layout::cssom::native_import_graph::ImportAdmission::reserve(&self.inner, bytes)
    }
}
