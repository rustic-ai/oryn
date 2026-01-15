use crate::cdp::CdpClient;
use crate::inject::execute_command;
use async_trait::async_trait;
use lscope_core::backend::{Backend, BackendError, NavigationResult};
use lscope_core::protocol::{ScannerProtocolResponse, ScannerRequest};
use tracing::info;

pub struct HeadlessBackend {
    client: Option<CdpClient>,
}

impl HeadlessBackend {
    pub fn new() -> Self {
        Self { client: None }
    }
}

#[async_trait]
impl Backend for HeadlessBackend {
    async fn launch(&mut self) -> Result<(), BackendError> {
        info!("Launching Headless Backend (Chromium)...");
        let client = CdpClient::launch()
            .await
            .map_err(|e| BackendError::Other(e.to_string()))?;
        self.client = Some(client);
        Ok(())
    }

    async fn close(&mut self) -> Result<(), BackendError> {
        if let Some(client) = self.client.take() {
            client
                .close()
                .await
                .map_err(|e| BackendError::Other(e.to_string()))?;
        }
        Ok(())
    }

    async fn is_ready(&self) -> bool {
        self.client.is_some()
    }

    async fn navigate(&mut self, url: &str) -> Result<NavigationResult, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;

        info!("Navigating to: {}", url);
        client
            .page
            .goto(url)
            .await
            .map_err(|e| BackendError::Navigation(e.to_string()))?;

        // Wait for load? default goto waits for load event mostly.

        let title = client
            .page
            .get_title()
            .await
            .unwrap_or_default()
            .unwrap_or_default();

        Ok(NavigationResult {
            url: url.to_string(),
            title,
            status: 200, // We assume 200 if no error
        })
    }

    async fn execute_scanner(
        &mut self,
        command: ScannerRequest,
    ) -> Result<ScannerProtocolResponse, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;

        // Serialize command to get action + params
        // But our `execute_command` takes (action, params).
        // `ScannerRequest` tag="action".
        // We can serialize `command` to Value, then extract action.

        let value = serde_json::to_value(&command)?;

        // Extract action string, cloning it to allow moving `value` later
        let action = value
            .as_object()
            .and_then(|obj| obj.get("action"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| BackendError::Scanner("Missing action field".into()))?;

        // Passing the whole object as params acts as args?
        // ScannerRequest deserialization expects `ScanRequest` fields inside.
        // `ScannerRequest` is { action: "scan", ...fields }.
        // `Lemmascope.process(action, params)` expects `params` to be the object including data fields.
        // Yes, passing the whole `value` is fine, scanner ignores extra `action` field or uses it.
        // Wait, `ScannerRequest` struct has `action` as tag.
        // If we access `Scan(ScanRequest)`, `ScanRequest` doesn't have `action`.
        // But `serde(tag="action")` flattens it?
        // Serialize `ScannerRequest::Scan(...)` -> `{ "action": "scan", "max_elements": ... }`
        // So `value` is exactly what we want to pass as `params`.

        // Special case: `Execute` command.
        // If action is `execute`, we might want to run raw script via CDP or via scanner?
        // Scanner supports `execute` action too.

        let result_value = execute_command(&client.page, &action, value)
            .await
            .map_err(|e| BackendError::Scanner(e.to_string()))?;

        // Deserialize result to ScannerProtocolResponse
        let response: ScannerProtocolResponse = serde_json::from_value(result_value)?;
        Ok(response)
    }

    async fn screenshot(&mut self) -> Result<Vec<u8>, BackendError> {
        let client = self.client.as_ref().ok_or(BackendError::NotReady)?;
        let bytes = client
            .page
            .screenshot(chromiumoxide::page::ScreenshotParams::builder().build())
            .await
            .map_err(|e| BackendError::Other(format!("Screenshot failed: {}", e)))?;

        Ok(bytes)
    }
}
