use crate::webdriver::WebDriverClient;
use async_trait::async_trait;
use lscope_core::backend::{Backend, BackendError, NavigationResult};
use lscope_core::protocol::{ScannerProtocolResponse, ScannerRequest};
use tracing::info;

pub struct EmbeddedBackend {
    client: Option<WebDriverClient>,
    webdriver_url: String,
}

impl EmbeddedBackend {
    pub fn new(webdriver_url: String) -> Self {
        Self {
            client: None,
            webdriver_url,
        }
    }
}

#[async_trait]
impl Backend for EmbeddedBackend {
    async fn launch(&mut self) -> Result<(), BackendError> {
        info!("Connecting to WebDriver at {}...", self.webdriver_url);
        let client = WebDriverClient::connect(&self.webdriver_url, None)
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
            .client
            .goto(url)
            .await
            .map_err(|e| BackendError::Navigation(e.to_string()))?;

        let title = client.client.title().await.unwrap_or_default();
        let current_url = client
            .client
            .current_url()
            .await
            .map(|u| u.to_string())
            .unwrap_or_else(|_| url.to_string());

        Ok(NavigationResult {
            url: current_url,
            title,
            status: 200,
        })
    }

    async fn execute_scanner(
        &mut self,
        command: ScannerRequest,
    ) -> Result<ScannerProtocolResponse, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;

        // 1. Inject Scanner JS if needed
        // We check if Lemmascope is defined.
        let check_script = "return typeof window.Lemmascope !== 'undefined';";
        let is_injected = client
            .client
            .execute(check_script, vec![])
            .await
            .map_err(|e| BackendError::Other(format!("Failed to check scanner injection: {}", e)))?
            .as_bool()
            .unwrap_or(false);

        if !is_injected {
            info!("Injecting scanner...");
            let script = lscope_scanner::SCANNER_JS;
            client
                .client
                .execute(script, vec![])
                .await
                .map_err(|e| BackendError::Other(format!("Failed to inject scanner: {}", e)))?;
        }

        // 2. Prepare arguments
        // `Lemmascope.process(message)` takes a single object.
        // `ScannerRequest` serializes to that object (e.g. {action: "scan", ...}).
        let args_json = serde_json::to_value(&command)?;

        // 3. Execute
        // fantoccini execute expects script and args.
        // We call window.Lemmascope.process(args) and return result.
        // Note: return await ...;
        let exec_script = r#"
            const args = arguments[0];
            return window.Lemmascope.process(args);
        "#;

        let result_value = client
            .client
            .execute(exec_script, vec![args_json])
            .await
            .map_err(|e| BackendError::Scanner(e.to_string()))?;

        // 4. Deserialize result
        let response: ScannerProtocolResponse = serde_json::from_value(result_value)?;
        Ok(response)
    }

    async fn screenshot(&mut self) -> Result<Vec<u8>, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;
        let bytes = client
            .client
            .screenshot()
            .await
            .map_err(|e| BackendError::Other(format!("Screenshot failed: {}", e)))?;
        Ok(bytes)
    }
}
