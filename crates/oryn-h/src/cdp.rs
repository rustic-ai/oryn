use chromiumoxide::{Browser, BrowserConfig, Page};
use futures::StreamExt;
use tokio::task::JoinHandle;

pub struct CdpClient {
    pub browser: Browser,
    pub handler_task: JoinHandle<()>,
    pub page: Page,
}

impl CdpClient {
    pub async fn launch() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut config_builder = BrowserConfig::builder();
        config_builder = config_builder.no_sandbox(); // Often needed in docker/CI/restricted envs

        // Support custom Chrome path via CHROME_BIN environment variable
        if let Ok(chrome_bin) = std::env::var("CHROME_BIN") {
            tracing::info!("Using custom Chrome binary: {}", chrome_bin);
            config_builder = config_builder.chrome_executable(chrome_bin);
        }

        let (browser, mut handler) = Browser::launch(
            config_builder
                .build()
                .map_err(|e| format!("Failed to build browser config: {}", e))?,
        )
        .await
        .map_err(|e| format!("Failed to launch browser: {}", e))?;

        // Spawn handler loop
        let handler_task = tokio::spawn(async move {
            while let Some(h) = handler.next().await {
                if let Err(e) = h {
                    tracing::error!("Browser handler error (ignoring): {}", e);
                    continue;
                }
            }
            tracing::info!("Browser handler task ended");
        });

        // Create page
        let page = browser
            .new_page("about:blank")
            .await
            .map_err(|e| format!("Failed to create page: {}", e))?;

        // Enable Console events
        // Note: chromiumoxide Page provides methods to subscribe to events easily?
        // Actually, we can just use `page.event_listener::<Event>()`.
        // Let's spawn a listener for Console events.

        let mut console_events = page
            .event_listener::<chromiumoxide::cdp::js_protocol::runtime::EventConsoleApiCalled>()
            .await
            .map_err(|e| format!("Failed to subscribe to console events: {}", e))?;

        tokio::spawn(async move {
            while let Some(event) = console_events.next().await {
                // Log simplified message
                // Event has `args` which are RemoteObjects. We might get basic description.
                let args_str: Vec<String> = event
                    .args
                    .iter()
                    .map(|arg| {
                        arg.description
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string())
                    })
                    .collect();
                tracing::info!(
                    "Browser Console [{:?}]: {}",
                    event.r#type,
                    args_str.join(" ")
                );
            }
        });

        // Handle JavaScript Dialogs (Alert, Confirm, Prompt) - Auto-accept
        let mut dialog_events = page
            .event_listener::<chromiumoxide::cdp::browser_protocol::page::EventJavascriptDialogOpening>()
            .await
            .map_err(|e| format!("Failed to subscribe to dialog events: {}", e))?;

        let page_clone = page.clone();
        tokio::spawn(async move {
            while let Some(event) = dialog_events.next().await {
                tracing::info!(
                    "Handling JavaScript Dialog: {} ({:?})",
                    event.message,
                    event.r#type
                );
                // Auto-accept (true) and use default prompt text (None)
                let cmd =
                    chromiumoxide::cdp::browser_protocol::page::HandleJavaScriptDialogParams::new(
                        true,
                    );
                if let Err(e) = page_clone.execute(cmd).await {
                    tracing::error!("Failed to handle/accept dialog: {}", e);
                }
            }
        });

        // Enable Network Logging via features module
        if let Err(e) = crate::features::enable_network_logging(&page).await {
            tracing::warn!("Failed to enable network logging: {}", e);
        }

        Ok(Self {
            browser,
            handler_task,
            page,
        })
    }

    pub async fn close(mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.browser
            .close()
            .await
            .map_err(|e| format!("Error closing browser: {}", e))?;
        self.handler_task
            .await
            .map_err(|e| format!("Error awaiting handler: {}", e))?;
        Ok(())
    }
}
