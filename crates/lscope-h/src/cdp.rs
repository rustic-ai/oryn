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
        let (browser, mut handler) = Browser::launch(
            BrowserConfig::builder()
                .no_sandbox() // Often needed in docker/CI/restricted envs
                // .with_head() // Uncomment for debugging
                .build()
                .map_err(|e| format!("Failed to build browser config: {}", e))?,
        )
        .await
        .map_err(|e| format!("Failed to launch browser: {}", e))?;

        // Spawn handler loop
        let handler_task = tokio::spawn(async move {
            while let Some(h) = handler.next().await {
                if h.is_err() {
                    break;
                }
            }
        });

        // Initialize with blank page
        let page = browser
            .new_page("about:blank")
            .await
            .map_err(|e| format!("Failed to create page: {}", e))?;

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
