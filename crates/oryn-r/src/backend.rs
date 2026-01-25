use crate::server::{RemoteServer, ServerHandle};
use async_trait::async_trait;
use oryn_engine::backend::{Backend, BackendError, NavigationResult};
use oryn_engine::protocol::{
    Action, BackRequest, BrowserAction, ExecuteRequest, NavigateRequest, ScannerAction,
    ScannerProtocolResponse,
};
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

    async fn send_action(
        &mut self,
        action: Action,
    ) -> Result<ScannerProtocolResponse, BackendError> {
        let handle = self.server_handle.as_ref().ok_or(BackendError::NotReady)?;

        // Wait for at least one extension to connect
        if handle.command_tx.receiver_count() == 0 {
            info!("Waiting for browser extension to connect...");
            // Use timeout or loop? Original code looped.
            while handle.command_tx.receiver_count() == 0 {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
            info!("Extension connected.");
        }

        if let Err(e) = handle.command_tx.send(action) {
            return Err(BackendError::Other(format!("Failed to broadcast: {}", e)));
        }

        let mut rx = handle.response_rx.lock().await;
        match rx.recv().await {
            Some(resp) => Ok(resp),
            None => Err(BackendError::ConnectionLost),
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
        let action = Action::Browser(BrowserAction::Navigate(NavigateRequest {
            url: url.to_string(),
        }));

        self.send_action(action).await?;

        Ok(NavigationResult {
            url: url.to_string(),
            title: "".into(),
            status: 200,
        })
    }

    async fn execute_scanner(
        &mut self,
        command: ScannerAction,
    ) -> Result<ScannerProtocolResponse, BackendError> {
        self.send_action(Action::Scanner(command)).await
    }

    async fn screenshot(&mut self) -> Result<Vec<u8>, BackendError> {
        // Send screenshot request to extension
        // The extension should respond with base64-encoded PNG
        // Previously used ExecuteRequest.
        // Now use BrowserAction::Screenshot?
        // If extension supports BrowserAction::Screenshot, prefer that.
        // Assuming YES since we unified Action.
        use oryn_engine::protocol::ScreenshotRequest;

        let action = Action::Browser(BrowserAction::Screenshot(ScreenshotRequest {
            output: None,
            format: Some("png".into()),
            selector: None,
            fullpage: false,
        }));

        // Wait, did legacy implementation use ExecuteRequest because `ScreenshotRequest` wasn't supported by extension?
        // Legacy used `chrome.runtime.sendMessage({ action: 'screenshot' })`.
        // If I send `Action::Browser(Screenshot)`, it serializes to `{ action: "screenshot", ... }`.
        // This MATCHES `{ action: 'screenshot' }`!
        // So this is compatible!
        // Except older extension might expect just action field.
        // `ScreenshotRequest` has optional fields. Default serialization includes them as null or omitted?
        // `skip_serializing_if = "Option::is_none"` in protocol.rs.
        // So `{ action: "screenshot", format: "png", fullpage: false }`.

        let resp = self.send_action(action).await?;

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
                    "Invalid screenshot response format: expected Value(String)".into(),
                ))
            }
            ScannerProtocolResponse::Error { message, .. } => Err(BackendError::Other(format!(
                "Screenshot failed: {}",
                message
            ))),
        }
    }

    async fn go_back(&mut self) -> Result<NavigationResult, BackendError> {
        let action = Action::Browser(BrowserAction::Back(BackRequest {}));
        self.send_action(action).await?;
        Ok(NavigationResult {
            url: String::new(),
            title: String::new(),
            status: 200,
        })
    }

    async fn go_forward(&mut self) -> Result<NavigationResult, BackendError> {
        // Legacy used JS exec.
        // Now we have BrowserAction::Forward?
        // protocol.rs has ForwardRequest.
        use oryn_engine::protocol::ForwardRequest;
        let action = Action::Browser(BrowserAction::Forward(ForwardRequest::default()));
        self.send_action(action).await?;

        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
        Ok(NavigationResult {
            url: String::new(),
            title: String::new(),
            status: 200,
        })
    }

    async fn refresh(&mut self) -> Result<NavigationResult, BackendError> {
        use oryn_engine::protocol::RefreshRequest;
        let action = Action::Browser(BrowserAction::Refresh(RefreshRequest::default()));
        self.send_action(action).await?;

        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        Ok(NavigationResult {
            url: String::new(),
            title: String::new(),
            status: 200,
        })
    }

    async fn press_key(&mut self, key: &str, _modifiers: &[String]) -> Result<(), BackendError> {
        // Press key via JS execution (as legacy) or BrowserAction?
        // Currently no BrowserAction::PressKey.
        // ScannerAction::Execute is fine.
        let script = format!(
            r#"
            const event = new KeyboardEvent('keydown', {{ key: '{}', bubbles: true }});
            document.activeElement.dispatchEvent(event);
            const eventUp = new KeyboardEvent('keyup', {{ key: '{}', bubbles: true }});
            document.activeElement.dispatchEvent(eventUp);
            return true;
            "#,
            key, key
        );

        let req = ScannerAction::Execute(ExecuteRequest {
            script,
            args: vec![],
        });

        self.execute_scanner(req).await?;
        Ok(())
    }
}
