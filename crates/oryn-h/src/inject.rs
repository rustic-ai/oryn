use chromiumoxide::Page;
use std::error::Error;
use std::time::Duration;

const SCANNER_JS: &str = include_str!("../../oryn-scanner/src/scanner.js");

/// Default timeout for JavaScript evaluation (10 seconds).
/// This prevents hanging when dialogs (alert/confirm/prompt) block the JS thread.
const EVAL_TIMEOUT: Duration = Duration::from_secs(10);

/// Maximum retries for context errors during page navigation.
const MAX_CONTEXT_RETRIES: u32 = 10;

/// Delay between retries when context is not found (page navigating).
const CONTEXT_RETRY_DELAY: Duration = Duration::from_millis(100);

pub async fn inject_scanner(page: &Page) -> Result<(), Box<dyn Error + Send + Sync>> {
    // Retry loop to handle context errors during page navigation.
    // When the page navigates, the old JavaScript context is destroyed and a new one
    // is created. We may need to wait for the new context to be ready.
    let mut last_error = None;
    for attempt in 0..MAX_CONTEXT_RETRIES {
        match try_inject_scanner(page).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                let err_str = e.to_string();
                // Check if this is a context error (page is navigating)
                if err_str.contains("Cannot find context")
                    || err_str.contains("Execution context was destroyed")
                    || err_str.contains("-32000")
                {
                    tracing::debug!(
                        "Context not ready (attempt {}/{}), retrying...",
                        attempt + 1,
                        MAX_CONTEXT_RETRIES
                    );
                    last_error = Some(e);
                    tokio::time::sleep(CONTEXT_RETRY_DELAY).await;
                    continue;
                }
                // Not a context error, fail immediately
                return Err(e);
            }
        }
    }

    // All retries exhausted
    Err(last_error.unwrap_or_else(|| "Failed to inject scanner after retries".into()))
}

/// Internal function that attempts scanner injection once.
async fn try_inject_scanner(page: &Page) -> Result<(), Box<dyn Error + Send + Sync>> {
    // Check if loaded
    let is_loaded: bool = page
        .evaluate("typeof window.Oryn !== 'undefined'")
        .await
        .map_err(|e| format!("Failed to check scanner status: {}", e))?
        .into_value()
        .map_err(|e| format!("Failed to get bool value: {}", e))?;

    if !is_loaded {
        // Inject
        page.evaluate(SCANNER_JS)
            .await
            .map_err(|e| format!("Failed to inject scanner.js: {}", e))?;
    }

    Ok(())
}

pub async fn execute_command(
    page: &Page,
    _action: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, Box<dyn Error + Send + Sync>> {
    inject_scanner(page).await?;

    // Construct JS call: window.Oryn.process(action, params)
    // We need to pass params safely. serialized JSON string is easiest.
    let params_json = serde_json::to_string(&params)?;

    // Evaluate returns a RemoteObject. We want the value.
    // NOTE: chromiumoxide evaluate returns generic types.
    // We can evaluate an expression that returns a JSON string, then parse it?
    // Or let serialization handle it.

    // We pass params_json as the single argument, which contains "action" field.
    let expression = format!("window.Oryn.process({})", params_json);
    tracing::info!("Evaluating script: {}", expression);

    // Use timeout to prevent indefinite blocking when dialogs (alert/confirm/prompt) appear.
    // In headless mode, these dialogs block the JS thread with no way to dismiss them.
    let eval_future = page.evaluate(expression);
    let result = tokio::time::timeout(EVAL_TIMEOUT, eval_future)
        .await
        .map_err(|_| {
            "Command timed out - possibly blocked by a dialog (alert/confirm/prompt)".to_string()
        })?
        .map_err(|e| format!("Evaluation failed: {}", e))?
        .into_value::<serde_json::Value>()
        .map_err(|e| format!("Failed to get result: {}", e))?;

    Ok(result)
}
