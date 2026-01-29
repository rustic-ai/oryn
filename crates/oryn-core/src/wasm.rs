use oryn_common::protocol::ScanResult;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct OrynCore {
    scan: Option<ScanResult>,
}

#[wasm_bindgen]
impl OrynCore {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        console_error_panic_hook::set_once();
        Self { scan: None }
    }

    /// Update the scan context from JSON
    #[wasm_bindgen(js_name = updateScan)]
    pub fn update_scan(&mut self, scan_json: &str) -> Result<(), JsValue> {
        let scan = serde_json::from_str(scan_json)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse scan: {}", e)))?;
        self.scan = Some(scan);
        Ok(())
    }

    /// Process an OIL command and return the resulting action as JSON (legacy)
    ///
    /// This uses basic resolution without label association or inference.
    /// For advanced features, use `processCommandAdvanced()` instead.
    #[wasm_bindgen(js_name = processCommand)]
    pub fn process_command(&self, oil: &str) -> Result<String, JsValue> {
        let scan = self
            .scan
            .as_ref()
            .ok_or_else(|| JsValue::from_str("No scan loaded. Call updateScan() first."))?;

        let result = crate::api::process_command(oil, scan)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        serde_json::to_string(&result)
            .map_err(|e| JsValue::from_str(&format!("Failed to serialize result: {}", e)))
    }

    /// Process an OIL command with advanced resolution (async)
    ///
    /// This version includes:
    /// - Label association (click "Email" finds associated input)
    /// - Target inference (auto-find submit buttons, dismiss buttons)
    /// - Requirement validation
    ///
    /// Returns a Promise that resolves to the action JSON.
    #[wasm_bindgen(js_name = processCommandAdvanced)]
    pub async fn process_command_advanced(&self, oil: &str) -> Result<String, JsValue> {
        let scan = self
            .scan
            .as_ref()
            .ok_or_else(|| JsValue::from_str("No scan loaded. Call updateScan() first."))?;

        let result = crate::api::process_command_advanced(oil, scan)
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        serde_json::to_string(&result)
            .map_err(|e| JsValue::from_str(&format!("Failed to serialize result: {}", e)))
    }

    /// Get version information
    #[wasm_bindgen(js_name = getVersion)]
    pub fn get_version() -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}
