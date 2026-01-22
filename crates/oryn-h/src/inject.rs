use chromiumoxide::Page;
use std::error::Error;
use std::future::Future;
use std::time::Duration;

const SCANNER_JS: &str = include_str!("../../oryn-scanner/src/scanner.js");

/// Default timeout for JavaScript evaluation (10 seconds).
/// This prevents hanging when dialogs (alert/confirm/prompt) block the JS thread.
const EVAL_TIMEOUT: Duration = Duration::from_secs(10);

/// Maximum retries for context errors during page navigation.
const MAX_CONTEXT_RETRIES: u32 = 10;

/// Delay between retries when context is not found (page navigating).
const CONTEXT_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Check if an error indicates the page context is unavailable (e.g., during navigation).
fn is_context_error(err: &str) -> bool {
    err.contains("Cannot find context")
        || err.contains("Execution context was destroyed")
        || err.contains("-32000")
}

/// Retry an async operation that may fail due to context errors during page navigation.
/// Returns immediately on success or non-context errors; retries only on context errors.
async fn retry_on_context_error<T, E, F, Fut>(
    operation_name: &str,
    mut operation: F,
) -> Result<T, Box<dyn Error + Send + Sync>>
where
    E: std::fmt::Display,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut last_error = None;

    for attempt in 0..MAX_CONTEXT_RETRIES {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(e) => {
                let err_str = e.to_string();
                if is_context_error(&err_str) {
                    tracing::debug!(
                        "{} context error (attempt {}/{}), retrying...",
                        operation_name,
                        attempt + 1,
                        MAX_CONTEXT_RETRIES
                    );
                    last_error = Some(err_str);
                    tokio::time::sleep(CONTEXT_RETRY_DELAY).await;
                    continue;
                }
                return Err(err_str.into());
            }
        }
    }

    Err(last_error
        .unwrap_or_else(|| format!("{} failed after retries", operation_name))
        .into())
}

pub async fn inject_scanner(page: &Page) -> Result<(), Box<dyn Error + Send + Sync>> {
    retry_on_context_error("Scanner injection", || try_inject_scanner(page)).await
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
    let params_json = serde_json::to_string(&params)?;
    let expression = format!("window.Oryn.process({})", params_json);

    tracing::info!("Evaluating script: {}", expression);

    let mut last_error = None;

    for attempt in 0..MAX_CONTEXT_RETRIES {
        inject_scanner(page).await?;

        match evaluate_with_timeout(page, &expression).await {
            Ok(value) => return Ok(value),
            Err(EvalError::Timeout) => {
                return Err(
                    "Command timed out - possibly blocked by a dialog (alert/confirm/prompt)"
                        .into(),
                );
            }
            Err(EvalError::Context(err_str)) => {
                tracing::debug!(
                    "Context error during command (attempt {}/{}), retrying...",
                    attempt + 1,
                    MAX_CONTEXT_RETRIES
                );
                last_error = Some(err_str);
                tokio::time::sleep(CONTEXT_RETRY_DELAY).await;
            }
            Err(EvalError::Other(err_str)) => {
                return Err(format!("Evaluation failed: {}", err_str).into());
            }
        }
    }

    Err(last_error
        .unwrap_or_else(|| "Failed to execute command after retries".to_string())
        .into())
}

enum EvalError {
    Timeout,
    Context(String),
    Other(String),
}

async fn evaluate_with_timeout(
    page: &Page,
    expression: &str,
) -> Result<serde_json::Value, EvalError> {
    let eval_result = tokio::time::timeout(EVAL_TIMEOUT, page.evaluate(expression)).await;

    match eval_result {
        Err(_) => Err(EvalError::Timeout),
        Ok(Err(e)) => {
            let err_str = e.to_string();
            if is_context_error(&err_str) {
                Err(EvalError::Context(err_str))
            } else {
                Err(EvalError::Other(err_str))
            }
        }
        Ok(Ok(remote_object)) => remote_object
            .into_value::<serde_json::Value>()
            .map_err(|e| EvalError::Other(format!("Failed to get result: {}", e))),
    }
}
