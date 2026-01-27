use crate::cog::{self, CogProcess};
use crate::webdriver::WebDriverClient;
use async_trait::async_trait;
use oryn_engine::backend::{Backend, BackendError, NavigationResult};
use oryn_engine::protocol::{ActionResult, ScannerAction, ScannerData, ScannerProtocolResponse};
use tracing::{info, warn};

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

impl EmbeddedBackend {
    async fn get_navigation_result(
        client: &WebDriverClient,
    ) -> Result<NavigationResult, BackendError> {
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
        // Explicitly drop cog_process to trigger its cleanup
        self.cog_process = None;
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

        Self::get_navigation_result(client).await
    }

    async fn execute_scanner(
        &mut self,
        command: ScannerAction,
    ) -> Result<ScannerProtocolResponse, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;

        let mut last_error = None;
        for attempt in 1..=3 {
            if attempt > 1 {
                warn!("Retrying scanner execution (attempt {})...", attempt);
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }

            // 1. Inject Scanner JS if needed
            let check_script = "return typeof window.Oryn !== 'undefined';";
            let is_injected = match client.client.execute(check_script, vec![]).await {
                Ok(val) => val.as_bool().unwrap_or(false),
                Err(_) => false, // Assume not injected or context lost
            };

            if !is_injected {
                info!("Injecting scanner...");
                let script = oryn_scanner::SCANNER_JS;
                if let Err(e) = client.client.execute(script, vec![]).await {
                    last_error = Some(BackendError::Other(format!(
                        "Failed to inject scanner: {}",
                        e
                    )));
                    continue;
                }
            }

            // 2. Prepare arguments
            let args_json = serde_json::to_value(&command)?;

            // 3. Execute
            let exec_script = r#"
                const args = arguments[0];
                return window.Oryn.process(args);
            "#;

            match client.client.execute(exec_script, vec![args_json]).await {
                Ok(result_value) => {
                    // Handle Null result (common in WPE during navigation or context destruction)
                    if result_value.is_null() {
                        match command {
                            ScannerAction::Click(_)
                            | ScannerAction::Submit(_)
                            | ScannerAction::Type(_) => {
                                info!(
                                    "Scanner returned Null, synthesizing success for action: {:?}",
                                    command
                                );
                                return Ok(ScannerProtocolResponse::Ok {
                                    data: Box::new(ScannerData::Action(ActionResult {
                                        success: true,
                                        message: Some(
                                            "Synthesized success (suspected navigation)".into(),
                                        ),
                                        navigation: Some(true),
                                    })),
                                    warnings: vec![
                                        "Scanner returned Null, assuming navigation occurred"
                                            .into(),
                                    ],
                                });
                            }
                            _ => {
                                last_error =
                                    Some(BackendError::Scanner("Scanner returned Null".into()));
                                continue;
                            }
                        }
                    }

                    // 4. Deserialize result
                    match serde_json::from_value::<ScannerProtocolResponse>(result_value) {
                        Ok(response) => return Ok(response),
                        Err(e) => {
                            last_error =
                                Some(BackendError::Scanner(format!("Serialization error: {}", e)));
                            continue;
                        }
                    }
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    if err_msg.contains("undefined is not an object")
                        || err_msg.contains("Oryn is not defined")
                    {
                        last_error = Some(BackendError::Scanner(err_msg));
                        continue;
                    }
                    return Err(BackendError::Scanner(err_msg));
                }
            }
        }

        Err(last_error.unwrap_or(BackendError::Scanner("Failed after maximum retries".into())))
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

    async fn get_cookies(&mut self) -> Result<Vec<oryn_engine::protocol::Cookie>, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;
        let cookies = client
            .client
            .get_all_cookies()
            .await
            .map_err(|e| BackendError::Other(format!("Get cookies failed: {}", e)))?;

        Ok(cookies
            .into_iter()
            .map(|c| oryn_engine::protocol::Cookie {
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

    async fn get_tabs(&mut self) -> Result<Vec<oryn_engine::protocol::TabInfo>, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;
        let handles = client
            .client
            .windows()
            .await
            .map_err(|e| BackendError::Other(format!("Get windows failed: {}", e)))?;

        Ok(handles
            .into_iter()
            .map(|handle| oryn_engine::protocol::TabInfo {
                id: format!("{:?}", handle),
                url: "unknown".to_string(),
                title: "unknown".to_string(),
                active: false,
            })
            .collect())
    }

    async fn go_back(&mut self) -> Result<NavigationResult, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;

        client
            .client
            .back()
            .await
            .map_err(|e| BackendError::Navigation(format!("go_back failed: {}", e)))?;

        Self::get_navigation_result(client).await
    }

    async fn go_forward(&mut self) -> Result<NavigationResult, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;

        client
            .client
            .forward()
            .await
            .map_err(|e| BackendError::Navigation(format!("go_forward failed: {}", e)))?;

        Self::get_navigation_result(client).await
    }

    async fn refresh(&mut self) -> Result<NavigationResult, BackendError> {
        let client = self.client.as_mut().ok_or(BackendError::NotReady)?;

        client
            .client
            .refresh()
            .await
            .map_err(|e| BackendError::Navigation(format!("refresh failed: {}", e)))?;

        Self::get_navigation_result(client).await
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
