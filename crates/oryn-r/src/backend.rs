use crate::server::{RemoteServer, ServerHandle};
use async_trait::async_trait;
use oryn_engine::backend::{Backend, BackendError, NavigationResult};
use oryn_engine::protocol::{ScannerProtocolResponse, ScannerRequest};
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
        use oryn_engine::protocol::NavigateRequest;
        let req = ScannerRequest::Navigate(NavigateRequest {
            url: url.to_string(),
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

        // Wait for at least one extension to connect
        if handle.command_tx.receiver_count() == 0 {
            info!("Waiting for browser extension to connect...");
            while handle.command_tx.receiver_count() == 0 {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
            info!("Extension connected.");
        }

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
        // Send screenshot request to extension
        // The extension should respond with base64-encoded PNG
        use oryn_engine::protocol::ExecuteRequest;

        let script = r#"
            return new Promise((resolve, reject) => {
                if (typeof chrome !== 'undefined' && chrome.runtime && chrome.runtime.sendMessage) {
                    chrome.runtime.sendMessage({ action: 'screenshot' }, (response) => {
                        if (chrome.runtime.lastError) {
                            reject(chrome.runtime.lastError.message);
                        } else if (response && response.data) {
                            resolve(response.data);
                        } else {
                            reject('No screenshot data received');
                        }
                    });
                } else {
                    reject('Extension API not available');
                }
            });
        "#;

        let req = ScannerRequest::Execute(ExecuteRequest {
            script: script.to_string(),
            args: vec![],
        });

        let resp = self.execute_scanner(req).await?;

        // Extract base64 data from response
        match resp {
            ScannerProtocolResponse::Ok { data, .. } => {
                if let oryn_engine::protocol::ScannerData::Value(value) = *data
                    && let Some(base64_str) = value.as_str()
                {
                    use base64::Engine;
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(base64_str)
                        .map_err(|e| BackendError::Other(format!("Base64 decode failed: {}", e)))?;
                    return Ok(bytes);
                }
                Err(BackendError::Other(
                    "Invalid screenshot response format".into(),
                ))
            }
            ScannerProtocolResponse::Error { message, .. } => Err(BackendError::Other(format!(
                "Screenshot failed: {}",
                message
            ))),
        }
    }

    async fn go_back(&mut self) -> Result<NavigationResult, BackendError> {
        use oryn_engine::protocol::BackRequest;

        let req = ScannerRequest::Back(BackRequest {});

        self.execute_scanner(req).await?;

        Ok(NavigationResult {
            url: String::new(), // Will be updated by next scan
            title: String::new(),
            status: 200,
        })
    }

    async fn go_forward(&mut self) -> Result<NavigationResult, BackendError> {
        use oryn_engine::protocol::ExecuteRequest;

        let script =
            "history.forward(); return { url: window.location.href, title: document.title };";
        let req = ScannerRequest::Execute(ExecuteRequest {
            script: script.to_string(),
            args: vec![],
        });

        self.execute_scanner(req).await?;

        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

        Ok(NavigationResult {
            url: String::new(),
            title: String::new(),
            status: 200,
        })
    }

    async fn refresh(&mut self) -> Result<NavigationResult, BackendError> {
        use oryn_engine::protocol::ExecuteRequest;

        let script = "location.reload(); return true;";
        let req = ScannerRequest::Execute(ExecuteRequest {
            script: script.to_string(),
            args: vec![],
        });

        self.execute_scanner(req).await?;

        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        Ok(NavigationResult {
            url: String::new(),
            title: String::new(),
            status: 200,
        })
    }

    async fn press_key(&mut self, key: &str, _modifiers: &[String]) -> Result<(), BackendError> {
        use oryn_engine::protocol::ExecuteRequest;

        let script = format!(
            r#"
            const event = new KeyboardEvent('keydown', {{
                key: '{}',
                bubbles: true
            }});
            document.activeElement.dispatchEvent(event);
            const eventUp = new KeyboardEvent('keyup', {{
                key: '{}',
                bubbles: true
            }});
            document.activeElement.dispatchEvent(eventUp);
            return true;
            "#,
            key, key
        );

        let req = ScannerRequest::Execute(ExecuteRequest {
            script,
            args: vec![],
        });

        self.execute_scanner(req).await?;

        Ok(())
    }
}
