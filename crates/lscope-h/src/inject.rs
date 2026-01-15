use chromiumoxide::Page;
use std::error::Error;

const SCANNER_JS: &str = include_str!("../../lscope-scanner/src/scanner.js");

pub async fn inject_scanner(page: &Page) -> Result<(), Box<dyn Error + Send + Sync>> {
    // Check if loaded
    let is_loaded: bool = page
        .evaluate("typeof window.Lemmascope !== 'undefined'")
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

    // Construct JS call: window.Lemmascope.process(action, params)
    // We need to pass params safely. serialized JSON string is easiest.
    let params_json = serde_json::to_string(&params)?;

    // Evaluate returns a RemoteObject. We want the value.
    // NOTE: chromiumoxide evaluate returns generic types.
    // We can evaluate an expression that returns a JSON string, then parse it?
    // Or let serialization handle it.

    // We pass params_json as the single argument, which contains "action" field.
    let expression = format!("window.Lemmascope.process({})", params_json);
    tracing::info!("Evaluating script: {}", expression);

    let result = page
        .evaluate(expression)
        .await
        .map_err(|e| format!("Evaluation failed: {}", e))?
        .into_value::<serde_json::Value>()
        .map_err(|e| format!("Failed to get result: {}", e))?;

    Ok(result)
}
