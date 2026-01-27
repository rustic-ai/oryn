use crate::cdp::CdpClient;
use crate::inject::execute_command;
use async_trait::async_trait;
use oryn_engine::backend::{Backend, BackendError, NavigationResult};
use oryn_engine::protocol::{ScannerAction, ScannerProtocolResponse};
use tracing::info;

pub struct HeadlessBackend {
    client: Option<CdpClient>,
    visible: bool,
}

impl HeadlessBackend {
    pub fn new() -> Self {
        Self {
            client: None,
            visible: false,
        }
    }

    pub fn new_with_visibility(visible: bool) -> Self {
        Self {
            client: None,
            visible,
        }
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

impl HeadlessBackend {
    async fn get_navigation_result(
        page: &chromiumoxide::Page,
    ) -> Result<NavigationResult, BackendError> {
        let title = page
            .get_title()
            .await
            .unwrap_or_default()
            .unwrap_or_default();
        let url = page
            .url()
            .await
            .map_err(|e| BackendError::Navigation(e.to_string()))?
            .unwrap_or_default();
        Ok(NavigationResult {
            url,
            title,
            status: 200,
        })
    }
}

#[async_trait]
impl Backend for HeadlessBackend {
    async fn launch(&mut self) -> Result<(), BackendError> {
        info!("Launching Headless Backend (Chromium)...");
        let client = CdpClient::launch(self.visible)
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

        Self::get_navigation_result(&client.page).await
    }

    async fn execute_scanner(
        &mut self,
        command: ScannerAction,
    ) -> Result<ScannerProtocolResponse, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;

        let value = serde_json::to_value(&command)?;

        let action = value
            .as_object()
            .and_then(|obj| obj.get("action"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| BackendError::Scanner("Missing action field".into()))?;

        let result_value = execute_command(&client.page, &action, value)
            .await
            .map_err(|e| BackendError::Scanner(e.to_string()))?;

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
        client
            .page
            .evaluate("history.back();")
            .await
            .map_err(|e| BackendError::Navigation(format!("go_back failed: {}", e)))?;
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
        Self::get_navigation_result(&client.page).await
    }

    async fn go_forward(&mut self) -> Result<NavigationResult, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;
        client
            .page
            .evaluate("history.forward();")
            .await
            .map_err(|e| BackendError::Navigation(format!("go_forward failed: {}", e)))?;
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
        Self::get_navigation_result(&client.page).await
    }

    async fn refresh(&mut self) -> Result<NavigationResult, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;
        client
            .page
            .reload()
            .await
            .map_err(|e| BackendError::Navigation(format!("refresh failed: {}", e)))?;
        Self::get_navigation_result(&client.page).await
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
