//! WASM Selector Resolver
//!
//! Provides a selector resolver for WASM builds that uses browser DOM APIs directly,
//! and a no-op stub for native builds.

use super::{SelectorError, SelectorResolver};

/// Selector resolver for WASM builds.
///
/// In WASM, this uses `querySelector` directly via `web_sys` without needing a backend.
/// In native builds, this is a no-op stub that always returns `NotAvailable`.
#[derive(Default)]
pub struct WasmSelectorResolver;

impl WasmSelectorResolver {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(target_arch = "wasm32")]
#[async_trait(?Send)]
impl SelectorResolver for WasmSelectorResolver {
    async fn resolve_selector(&mut self, selector: &str) -> Result<Option<u32>, SelectorError> {
        use wasm_bindgen::JsValue;

        // TODO: Implement actual selector resolution using web_sys.
        // Needs: Oryn.ShadowUtils.querySelectorWithShadow + Oryn.State.elementMap integration.
        // For now, returns None to allow compilation.

        web_sys::console::warn_1(&JsValue::from_str(&format!(
            "[WASM Resolver] Selector resolution not yet implemented: {}",
            selector
        )));

        Ok(None)
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[async_trait::async_trait]
impl SelectorResolver for WasmSelectorResolver {
    async fn resolve_selector(&mut self, _selector: &str) -> Result<Option<u32>, SelectorError> {
        Err(SelectorError::NotAvailable(
            "WasmSelectorResolver only works in WASM builds".to_string(),
        ))
    }
}
