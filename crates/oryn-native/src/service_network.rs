//! In-process navigation interception uses the same parent registration registry
//! as the isolated page broker. Other requests retain their existing transport.
use crate::{network::*, runtime::WeakBrowserContext};
use oryn_common::v3::PageId;
pub(crate) struct ServiceNavigationBroker {
    pub context: WeakBrowserContext,
    pub page: PageId,
    pub inner: SharedNetworkBroker,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker_route::WorkerRoute;
    use std::sync::{Arc, Mutex};

    #[test]
    fn auxiliary_routes_reach_parent_without_opener_interception() {
        struct Parent(WorkerRoute, Arc<Mutex<Vec<&'static str>>>);
        impl NetworkBroker for Parent {
            fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                Err(NetworkError::Unavailable)
            }
            fn start_routed(&self, route: Option<&WorkerRoute>, _: NetworkRequest)
                -> Result<NetworkOperation, NetworkError> {
                assert_eq!(route, Some(&self.0));
                self.1.lock().unwrap().push("start");
                Err(NetworkError::Unavailable)
            }
            fn cancel_routed(&self, route: Option<&WorkerRoute>, id: u64)
                -> Result<(), NetworkError> {
                assert_eq!(route, Some(&self.0));
                assert_eq!(id, 41);
                self.1.lock().unwrap().push("cancel");
                Ok(())
            }
            fn discard_response_policy_routed(&self, route: Option<&WorkerRoute>, token: &str)
                -> Result<(), NetworkError> {
                assert_eq!(route, Some(&self.0));
                assert_eq!(token, "grant");
                self.1.lock().unwrap().push("discard");
                Ok(())
            }
        }
        let context = crate::runtime::Browser::in_process_with_config(Default::default())
            .new_context_with_options(Default::default());
        let route = WorkerRoute::issue(1, 2, 0).unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let broker = ServiceNavigationBroker {
            context: context.downgrade(), page: PageId(1),
            inner: SharedNetworkBroker::new(Parent(route.clone(), calls.clone())),
        };
        drop(context);
        let request = NetworkRequest {
            request_id: 41, method: "GET".into(), url: "https://target.test/".into(),
            headers: Vec::new(), body: Vec::new(), credentials: false,
            security: Default::default(), resolved_addrs: Vec::new(),
        };
        assert!(matches!(broker.start_routed(Some(&route), request.clone()),
            Err(NetworkError::Unavailable)));
        broker.cancel_routed(Some(&route), 41).unwrap();
        broker.discard_response_policy_routed(Some(&route), "grant").unwrap();
        assert!(broker.start_routed(None, request).is_err());
        assert_eq!(*calls.lock().unwrap(), ["start", "cancel", "discard"]);
    }
}
impl NetworkBroker for ServiceNavigationBroker {
    fn start_routed(
        &self,
        route: Option<&crate::worker_route::WorkerRoute>,
        request: NetworkRequest,
    ) -> Result<NetworkOperation, NetworkError> {
        if route.is_some() {
            // Isolated auxiliary authority belongs to the parent router, not
            // this in-process opener's interception registry. Preserve it.
            self.inner.start_routed(route, request)
        } else {
            self.start_operation(request)
        }
    }
    fn cancel_routed(
        &self,
        route: Option<&crate::worker_route::WorkerRoute>,
        id: u64,
    ) -> Result<(), NetworkError> {
        if route.is_some() {
            self.inner.cancel_routed(route, id)
        } else {
            self.cancel_operation(id)
        }
    }
    fn discard_response_policy_routed(
        &self,
        route: Option<&crate::worker_route::WorkerRoute>,
        token: &str,
    ) -> Result<(), NetworkError> {
        self.inner.discard_response_policy_routed(route, token)
    }
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
