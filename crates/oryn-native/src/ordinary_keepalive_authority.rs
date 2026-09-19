use super::*;
impl BrowserContext {
    pub(crate) fn freeze_ordinary_keepalive_authority(
        &self,
        page: PageId,
        creator: &oryn_js::worker_wire::Creator,
    ) -> Result<crate::context_service::ordinary_keepalive::Authority, RuntimeError> {
        let oryn_js::worker_wire::Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "Window keepalive cannot impersonate worker".into(),
            ));
        };
        let (owner, _, _) =
            self.window_worker_creator(page, target, *document_generation, *stream_epoch)?;
        let native = self
            .native_worker_authority(&owner)
            .map_err(|e| RuntimeError::ContextService(e.to_string()))?;
        let controller = self
            .existing_service_registry()
            .map(|registry| registry.freeze_fetch_controller(&owner))
            .transpose()
            .map_err(|e| RuntimeError::ContextService(e.to_string()))?
            .flatten();
        let referrer = crate::referrer::ReferrerPolicy::default().after_response(&native.2.headers);
        self.with_window_worker_creator(
            page,
            target,
            *document_generation,
            *stream_epoch,
            true,
            |(current, partition, origin), _, top| {
                if current != owner || partition != native.0 || origin != native.1 {
                    return Err(RuntimeError::NavigationCancelled);
                }
                Ok(crate::context_service::ordinary_keepalive::Authority {
                    owner,
                    native,
                    top_url: top.ok_or_else(|| {
                        RuntimeError::ProtocolViolation("keepalive top URL missing".into())
                    })?,
                    referrer,
                    controller,
                    blob: None,
                })
            },
        )
    }
}
