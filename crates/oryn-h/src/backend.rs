use crate::cdp::CdpClient;
use crate::inject::execute_command;
use async_trait::async_trait;
use oryn_engine::backend::{Backend, BackendError, NavigationResult};
use oryn_engine::protocol::{ScannerProtocolResponse, ScannerAction};
use tracing::info;

pub struct HeadlessBackend {
    client: Option<CdpClient>,
}

impl HeadlessBackend {
    pub fn new() -> Self {
        Self { client: None }
    }

    pub fn get_client(&self) -> Option<&CdpClient> {
        self.client.as_ref()
    }
}

impl Default for HeadlessBackend {
    fn default() -> Self {
        Self::new()
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
        command: ScannerAction,
    ) -> Result<ScannerProtocolResponse, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;

        // Serialize command to get action + params
        // But our `execute_command` takes (action, params).
        // `ScannerAction` tag="action".
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
        // ScannerAction deserialization expects `ScanRequest` fields inside.
        // `ScannerAction` is { action: "scan", ...fields }.
        // `Oryn.process(action, params)` expects `params` to be the object including data fields.
        // Yes, passing the whole `value` is fine, scanner ignores extra `action` field or uses it.
        // Wait, `ScannerAction` struct has `action` as tag.
        // If we access `Scan(ScanRequest)`, `ScanRequest` doesn't have `action`.
        // But `serde(tag="action")` flattens it?
        // Serialize `ScannerAction::Scan(...)` -> `{ "action": "scan", "max_elements": ... }`
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

    async fn pdf(&mut self) -> Result<Vec<u8>, BackendError> {
        let client = self.client.as_ref().ok_or(BackendError::NotReady)?;
        let bytes = client
            .page
            .pdf(chromiumoxide::cdp::browser_protocol::page::PrintToPdfParams::builder().build())
            .await
            .map_err(|e| BackendError::Other(format!("PDF generation failed: {}", e)))?;

        Ok(bytes)
    }

    async fn get_cookies(&mut self) -> Result<Vec<oryn_engine::protocol::Cookie>, BackendError> {
        let client = self.client.as_ref().ok_or(BackendError::NotReady)?;
        let cookies = client
            .page
            .get_cookies()
            .await
            .map_err(|e| BackendError::Other(format!("Get cookies failed: {}", e)))?;

        Ok(cookies
            .into_iter()
            .map(|c| oryn_engine::protocol::Cookie {
                name: c.name,
                value: c.value,
                domain: Some(c.domain),
                path: Some(c.path),
                expires: Some(c.expires),
                http_only: Some(c.http_only),
                secure: Some(c.secure),
            })
            .collect())
    }

    async fn get_tabs(&mut self) -> Result<Vec<oryn_engine::protocol::TabInfo>, BackendError> {
        let client = self.client.as_ref().ok_or(BackendError::NotReady)?;
        let pages = client
            .browser
            .pages()
            .await
            .map_err(|e| BackendError::Other(format!("Get pages failed: {}", e)))?;

        let mut tabs = Vec::new();
        for page in pages {
            let url = page.url().await.unwrap_or_default().unwrap_or_default();
            let title = page
                .get_title()
                .await
                .unwrap_or_default()
                .unwrap_or_default();
            tabs.push(oryn_engine::protocol::TabInfo {
                id: "unknown".to_string(),
                url,
                title,
                active: false,
            });
        }
        Ok(tabs)
    }

    async fn go_back(&mut self) -> Result<NavigationResult, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;

        // Use JavaScript history.back()
        client
            .page
            .evaluate("history.back();")
            .await
            .map_err(|e| BackendError::Navigation(format!("go_back failed: {}", e)))?;

        // Wait for navigation
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

        let title = client
            .page
            .get_title()
            .await
            .unwrap_or_default()
            .unwrap_or_default();

        let url = client
            .page
            .url()
            .await
            .map_err(|e| BackendError::Navigation(e.to_string()))?
            .unwrap_or_default();

        Ok(NavigationResult {
            url: url.to_string(),
            title,
            status: 200,
        })
    }

    async fn go_forward(&mut self) -> Result<NavigationResult, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;

        // Use JavaScript history.forward()
        client
            .page
            .evaluate("history.forward();")
            .await
            .map_err(|e| BackendError::Navigation(format!("go_forward failed: {}", e)))?;

        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

        let title = client
            .page
            .get_title()
            .await
            .unwrap_or_default()
            .unwrap_or_default();

        let url = client
            .page
            .url()
            .await
            .map_err(|e| BackendError::Navigation(e.to_string()))?
            .unwrap_or_default();

        Ok(NavigationResult {
            url: url.to_string(),
            title,
            status: 200,
        })
    }

    async fn refresh(&mut self) -> Result<NavigationResult, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;

        client
            .page
            .reload()
            .await
            .map_err(|e| BackendError::Navigation(format!("refresh failed: {}", e)))?;

        let title = client
            .page
            .get_title()
            .await
            .unwrap_or_default()
            .unwrap_or_default();

        let url = client
            .page
            .url()
            .await
            .map_err(|e| BackendError::Navigation(e.to_string()))?
            .unwrap_or_default();

        Ok(NavigationResult {
            url: url.to_string(),
            title,
            status: 200,
        })
    }

    async fn press_key(&mut self, key: &str, modifiers: &[String]) -> Result<(), BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;

        // Build modifier flags
        let mut modifier_flags = 0;
        for m in modifiers {
            match m.to_lowercase().as_str() {
                "alt" => modifier_flags |= 1,
                "ctrl" | "control" => modifier_flags |= 2,
                "meta" | "cmd" | "command" => modifier_flags |= 4,
                "shift" => modifier_flags |= 8,
                _ => {}
            }
        }

        // Send key down and up events via CDP Input.dispatchKeyEvent
        use chromiumoxide::cdp::browser_protocol::input::{
            DispatchKeyEventParams, DispatchKeyEventType,
        };

        // Key down
        let key_down = DispatchKeyEventParams::builder()
            .r#type(DispatchKeyEventType::KeyDown)
            .key(key)
            .modifiers(modifier_flags)
            .build()
            .map_err(|e| BackendError::Other(format!("Failed to build key event: {:?}", e)))?;

        client
            .page
            .execute(key_down)
            .await
            .map_err(|e| BackendError::Other(format!("press_key down failed: {}", e)))?;

        // Key up
        let key_up = DispatchKeyEventParams::builder()
            .r#type(DispatchKeyEventType::KeyUp)
            .key(key)
            .modifiers(modifier_flags)
            .build()
            .map_err(|e| BackendError::Other(format!("Failed to build key event: {:?}", e)))?;

        client
            .page
            .execute(key_up)
            .await
            .map_err(|e| BackendError::Other(format!("press_key up failed: {}", e)))?;

        Ok(())
    }
}
