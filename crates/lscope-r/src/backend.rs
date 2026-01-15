use crate::server::{RemoteServer, ServerHandle};
use async_trait::async_trait;
use lscope_core::backend::{Backend, BackendError, NavigationResult};
use lscope_core::protocol::{ScannerProtocolResponse, ScannerRequest};
use tracing::info;

pub struct RemoteBackend {
    port: u16,
    server_handle: Option<ServerHandle>,
}

impl RemoteBackend {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            server_handle: None,
        }
    }
}

#[async_trait]
impl Backend for RemoteBackend {
    async fn launch(&mut self) -> Result<(), BackendError> {
        info!("Launching Remote Backend on port {}", self.port);
        let server = RemoteServer::new(self.port);
        match server.start().await {
            Ok(handle) => {
                self.server_handle = Some(handle);
                Ok(())
            }
            Err(e) => Err(BackendError::Other(format!("Connection failed: {}", e))),
        }
    }

    async fn close(&mut self) -> Result<(), BackendError> {
        self.server_handle = None;
        Ok(())
    }

    async fn is_ready(&self) -> bool {
        self.server_handle.is_some()
    }

    async fn navigate(&mut self, url: &str) -> Result<NavigationResult, BackendError> {
        use lscope_core::protocol::ExecuteRequest;
        let script = format!("window.location.href = '{}';", url);
        let req = ScannerRequest::Execute(ExecuteRequest {
            script,
            args: vec![],
        });

        self.execute_scanner(req).await?;

        Ok(NavigationResult {
            url: url.to_string(),
            title: "".into(),
            status: 200,
        })
    }

    async fn execute_scanner(
        &mut self,
        command: ScannerRequest,
    ) -> Result<ScannerProtocolResponse, BackendError> {
        let handle = self.server_handle.as_ref().ok_or(BackendError::NotReady)?;

        if let Err(e) = handle.command_tx.send(command) {
            return Err(BackendError::Other(format!("Failed to broadcast: {}", e)));
        }

        let mut rx = handle.response_rx.lock().await;
        match rx.recv().await {
            Some(resp) => Ok(resp),
            None => Err(BackendError::ConnectionLost),
        }
    }

    async fn screenshot(&mut self) -> Result<Vec<u8>, BackendError> {
        Err(BackendError::NotSupported(
            "Screenshot not implemented in Remote Mode yet".into(),
        ))
    }
}
