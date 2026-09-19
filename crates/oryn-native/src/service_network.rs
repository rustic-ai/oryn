//! In-process navigation interception uses the same parent registration registry
//! as the isolated page broker. Other requests retain their existing transport.
use crate::{network::*, runtime::WeakBrowserContext};
use oryn_common::v3::PageId;
pub(crate) struct ServiceNavigationBroker {
    pub context: WeakBrowserContext,
    pub page: PageId,
    pub inner: SharedNetworkBroker,
}
impl NetworkBroker for ServiceNavigationBroker {
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        let context = self
            .context
            .upgrade()
            .ok_or_else(|| NetworkError::Transport("navigation context retired".into()))?;
        let timing = ServiceWorkerTiming::default();
        let response = match context.intercept_service_navigation(
            self.page,
            &request,
            self.inner.clone(),
            &timing,
        )? {
            Some(response) => response,
            None => self.inner.send(request)?,
        };
        Ok(timing.response(response))
    }
    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        let context = self
            .context
            .upgrade()
            .ok_or_else(|| NetworkError::Transport("navigation context retired".into()))?;
        let timing = ServiceWorkerTiming::default();
        let operation = match context.intercept_service_navigation_operation(
            self.page,
            &request,
            self.inner.clone(),
            &timing,
        )? {
            Some(operation) => operation,
            None => self.inner.start_operation(request)?,
        };
        Ok(timing.operation(operation))
    }
    fn cancel_operation(&self, id: u64) -> Result<(), NetworkError> {
        if let Some(context) = self.context.upgrade() {
            context.cancel_service_navigation(self.page, id);
        }
        self.inner.cancel_operation(id)
    }
}
