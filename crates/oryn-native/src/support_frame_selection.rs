// Included beside frame_response_stream in rust_executable.rs.
// Ask the parent about the exact existing stream; never infer from a URL.
fn prepare_support_frame_decoder(
    host: &RawV8Host,
    realm: RealmId,
    body: &frame_response_stream::FrameResponseBody,
    budget: &Arc<std::sync::atomic::AtomicUsize>,
) -> Result<Option<(Rc<oryn_js::v8_bindings::V8BindingState>,
                     Rc<support_native_document::NativeSupportDocument>)>, HostError> {
    let creator = host.native_creator_for_realm(realm)?;
    let response = body.client.request(crate::context_service::ContextServiceRequest::SelectSupportFrameResponse {
        page_id: body.page_id,
        stream_id: body.id,
        creator: creator.clone(),
    }).map_err(|error| HostError::Execution(error.to_string()))?;
    match response {
        crate::context_service::ContextServiceResponse::SupportFrameResponseSelected { selected: false } => Ok(None),
        crate::context_service::ContextServiceResponse::SupportFrameResponseSelected { selected: true } => {
            let owner = host.retained_binding_state(realm)?;
            if owner.execution_retired() || owner.execution_suspended() {
                return Err(HostError::CachedAuthorityExpired);
            }
            // Same original response lease. The actual Document slot is
            // committed before the decoder can consume any selected bytes.
            let custody = support_native_document::NativeSupportDocument::install(
                &owner, body.clone(), budget)?;
            Ok(Some((owner, custody)))
        }
        _ => Err(HostError::Execution("unexpected support response selection".into())),
    }
}
