use chromiumoxide::{Browser, BrowserConfig, Page};
use futures::StreamExt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::task::JoinHandle;

pub struct CdpClient {
    pub browser: Browser,
    pub handler_task: JoinHandle<()>,
    pub page: Page,
    user_data_dir: Option<PathBuf>,
    cleanup_user_data_dir: bool,
}

impl CdpClient {
    pub async fn launch(visible: bool) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut config_builder = BrowserConfig::builder();
        config_builder = config_builder.no_sandbox(); // Often needed in docker/CI/restricted envs
        let (user_data_dir, cleanup_user_data_dir) = resolve_user_data_dir()?;
        config_builder = config_builder.user_data_dir(&user_data_dir);

        // Enable visible/headed mode if requested
        if visible {
            tracing::info!("Launching browser in visible mode");
            config_builder = config_builder.with_head();
        } else {
            tracing::info!("Launching browser in headless mode");
        }

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
        if should_enable_network_logging() {
            if let Err(e) = crate::features::enable_network_logging(&page).await {
                tracing::warn!("Failed to enable network logging: {}", e);
            }
        } else {
            tracing::info!("Network logging disabled (set ORYN_ENABLE_NETWORK_LOG=1 to enable)");
        }

        Ok(Self {
            browser,
            handler_task,
            page,
            user_data_dir: Some(user_data_dir),
            cleanup_user_data_dir,
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

        if self.cleanup_user_data_dir {
            if let Some(dir) = &self.user_data_dir {
                if let Err(e) = std::fs::remove_dir_all(dir) {
                    tracing::debug!("Failed to clean up user-data-dir {}: {}", dir.display(), e);
                }
            }
        }

        Ok(())
    }
}

fn should_enable_network_logging() -> bool {
    if let Ok(value) = std::env::var("ORYN_ENABLE_NETWORK_LOG") {
        let normalized = value.trim().to_ascii_lowercase();
        return normalized == "1"
            || normalized == "true"
            || normalized == "yes"
            || normalized == "on";
    }
    false
}

fn resolve_user_data_dir() -> Result<(PathBuf, bool), Box<dyn std::error::Error + Send + Sync>> {
    if let Ok(dir) = std::env::var("ORYN_USER_DATA_DIR") {
        let path = PathBuf::from(dir);
        std::fs::create_dir_all(&path)?;
        tracing::info!(
            "Using user data dir from ORYN_USER_DATA_DIR: {}",
            path.display()
        );
        return Ok((path, false));
    }

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("System clock error: {}", e))?
        .as_nanos();
    let unique = format!("oryn-chromium-profile-{}-{}", std::process::id(), nanos);
    let path = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&path)?;
    tracing::info!("Using isolated user data dir: {}", path.display());
    Ok((path, true))
}
