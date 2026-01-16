use crate::cog::{self, CogProcess};
use crate::webdriver::WebDriverClient;
use async_trait::async_trait;
use lscope_core::backend::{Backend, BackendError, NavigationResult};
use lscope_core::protocol::{ScannerProtocolResponse, ScannerRequest};
use tracing::info;

pub struct EmbeddedBackend {
    client: Option<WebDriverClient>,
    webdriver_url: Option<String>,
    cog_process: Option<CogProcess>,
    force_headless: bool,
    port: u16,
}

impl EmbeddedBackend {
    /// Create a new embedded backend that will auto-launch COG
    /// Uses headless mode if no display is available
    pub fn new() -> Self {
        Self {
            client: None,
            webdriver_url: None,
            cog_process: None,
            force_headless: false,
            port: cog::DEFAULT_COG_PORT,
        }
    }

    /// Create a new embedded backend that always runs headless
    /// Useful for CI/testing environments
    pub fn new_headless() -> Self {
        Self {
            client: None,
            webdriver_url: None,
            cog_process: None,
            force_headless: true,
            port: cog::DEFAULT_COG_PORT,
        }
    }

    /// Create a new embedded backend that runs headless on a specific port
    /// Useful for parallel testing
    pub fn new_headless_on_port(port: u16) -> Self {
        Self {
            client: None,
            webdriver_url: None,
            cog_process: None,
            force_headless: true,
            port,
        }
    }

    /// Create a new embedded backend connecting to an existing WebDriver
    pub fn with_url(webdriver_url: String) -> Self {
        Self {
            client: None,
            webdriver_url: Some(webdriver_url),
            cog_process: None,
            force_headless: false,
            port: cog::DEFAULT_COG_PORT,
        }
    }
}

impl Default for EmbeddedBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for EmbeddedBackend {
    async fn launch(&mut self) -> Result<(), BackendError> {
        let (webdriver_url, capabilities) = if let Some(url) = &self.webdriver_url {
            // Use provided URL (external WebDriver) - no special capabilities needed
            info!("Connecting to external WebDriver at {}...", url);
            (url.clone(), None)
        } else {
            // Auto-launch WPEWebDriver which will spawn COG
            info!(
                "Launching WPEWebDriver for COG browser on port {}...",
                self.port
            );
            let cog = cog::launch_cog(self.port, self.force_headless)
                .await
                .map_err(BackendError::Other)?;
            let url = cog.webdriver_url();
            self.cog_process = Some(cog);
            info!("WPEWebDriver launched at {}", url);
            // Pass WPE capabilities to tell WPEWebDriver to use COG
            (url, Some(cog::wpe_capabilities()))
        };

        let client = WebDriverClient::connect(&webdriver_url, capabilities)
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

    async fn get_cookies(&mut self) -> Result<Vec<lscope_core::protocol::Cookie>, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;
        let cookies = client
            .client
            .get_all_cookies()
            .await
            .map_err(|e| BackendError::Other(format!("Get cookies failed: {}", e)))?;

        Ok(cookies
            .into_iter()
            .map(|c| lscope_core::protocol::Cookie {
                name: c.name().to_string(),
                value: c.value().to_string(),
                domain: c.domain().map(|s| s.to_string()),
                path: c.path().map(|s| s.to_string()),
                expires: None, // fantoccini 0.19 Cookie doesn't easily expose expiry for all backends
                http_only: c.http_only(),
                secure: c.secure(),
            })
            .collect())
    }

    async fn get_tabs(&mut self) -> Result<Vec<lscope_core::protocol::TabInfo>, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;
        let handles = client
            .client
            .windows()
            .await
            .map_err(|e| BackendError::Other(format!("Get windows failed: {}", e)))?;

        let mut tabs = Vec::new();
        for handle in handles {
            tabs.push(lscope_core::protocol::TabInfo {
                id: format!("{:?}", handle),
                url: "unknown".to_string(),
                title: "unknown".to_string(),
                active: false,
            });
        }
        Ok(tabs)
    }

    async fn go_back(&mut self) -> Result<NavigationResult, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;

        client
            .client
            .back()
            .await
            .map_err(|e| BackendError::Navigation(format!("go_back failed: {}", e)))?;

        let title = client.client.title().await.unwrap_or_default();
        let url = client
            .client
            .current_url()
            .await
            .map(|u| u.to_string())
            .unwrap_or_default();

        Ok(NavigationResult {
            url,
            title,
            status: 200,
        })
    }

    async fn go_forward(&mut self) -> Result<NavigationResult, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;

        client
            .client
            .forward()
            .await
            .map_err(|e| BackendError::Navigation(format!("go_forward failed: {}", e)))?;

        let title = client.client.title().await.unwrap_or_default();
        let url = client
            .client
            .current_url()
            .await
            .map(|u| u.to_string())
            .unwrap_or_default();

        Ok(NavigationResult {
            url,
            title,
            status: 200,
        })
    }

    async fn refresh(&mut self) -> Result<NavigationResult, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;

        client
            .client
            .refresh()
            .await
            .map_err(|e| BackendError::Navigation(format!("refresh failed: {}", e)))?;

        let title = client.client.title().await.unwrap_or_default();
        let url = client
            .client
            .current_url()
            .await
            .map(|u| u.to_string())
            .unwrap_or_default();

        Ok(NavigationResult {
            url,
            title,
            status: 200,
        })
    }

    async fn press_key(&mut self, key: &str, _modifiers: &[String]) -> Result<(), BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;

        // WebDriver uses Actions API for key presses
        // For simple keys, we can use execute script
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
            "#,
            key, key
        );

        client
            .client
            .execute(&script, vec![])
            .await
            .map_err(|e| BackendError::Other(format!("press_key failed: {}", e)))?;

        Ok(())
    }
}
