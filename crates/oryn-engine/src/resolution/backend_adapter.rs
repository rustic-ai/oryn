//! Backend Adapter for Selector Resolution
//!
//! This module provides an adapter that implements the SelectorResolver trait
//! using a Backend instance. This allows the core resolution engine to work
//! with native backends while remaining backend-agnostic.

use async_trait::async_trait;
use oryn_common::protocol::{ExecuteRequest, ScannerAction, ScannerData, ScannerProtocolResponse};
use oryn_core::resolution::{SelectorError, SelectorResolver};

use crate::backend::Backend;

/// Adapter that implements SelectorResolver using a Backend.
///
/// This adapter wraps a backend reference and translates SelectorResolver
/// trait calls into Backend::execute_scanner() calls with appropriate JavaScript.
pub struct BackendSelectorResolver<'a, B: Backend + ?Sized> {
    backend: &'a mut B,
}

impl<'a, B: Backend + ?Sized> BackendSelectorResolver<'a, B> {
    /// Create a new backend adapter.
    pub fn new(backend: &'a mut B) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl<'a, B: Backend + ?Sized + Send + Sync> SelectorResolver for BackendSelectorResolver<'a, B> {
    async fn resolve_selector(&mut self, selector: &str) -> Result<Option<u32>, SelectorError> {
        let escaped = selector.replace('\\', "\\\\").replace('"', "\\\"");

        // Only try advanced fallback strategies if the selector is a simple word
        // (no special CSS characters like [ ] > + ~)
        let is_simple = !selector
            .chars()
            .any(|c| matches!(c, '[' | ']' | '>' | '+' | '~' | ' ' | ':'));

        // Shared JS to map a found element to its Oryn ID (or assign a new one)
        let map_element_js = r#"
                if (!el) return { found: false };
                var id = Oryn.State.inverseMap.get(el);
                if (id !== undefined && Oryn.State.elementMap.has(id)) return { found: true, id: id };
                id = Oryn.State.nextId++;
                Oryn.State.inverseMap.set(el, id);
                Oryn.State.elementMap.set(id, el);
                return { found: true, id: id };
        "#;

        let script = if is_simple {
            // For simple selectors, try multiple strategies
            format!(
                r###"
                var selectors = [
                    '{0}',
                    '#{0}',
                    '[name="{0}"]',
                    '[placeholder*="{0}"]',
                    '[aria-label*="{0}"]'
                ];
                var el = null;
                for (var i = 0; i < selectors.length; i++) {{
                    try {{
                        el = Oryn.ShadowUtils.querySelectorWithShadow(document.body, selectors[i]);
                        if (el) break;
                    }} catch (e) {{}}
                }}
                {1}
                "###,
                escaped, map_element_js
            )
        } else {
            // For complex selectors, use directly
            format!(
                r###"
                var el = null;
                try {{
                    el = Oryn.ShadowUtils.querySelectorWithShadow(document.body, "{0}");
                }} catch (e) {{}}
                {1}
                "###,
                escaped, map_element_js
            )
        };

        let req = ScannerAction::Execute(ExecuteRequest {
            script,
            args: vec![],
        });

        match self.backend.execute_scanner(req).await {
            Ok(resp) => parse_selector_response(&resp),
            Err(e) => Err(SelectorError::Backend(e.to_string())),
        }
    }
}

/// Parse the scanner response from a selector query.
fn parse_selector_response(resp: &ScannerProtocolResponse) -> Result<Option<u32>, SelectorError> {
    match resp {
        ScannerProtocolResponse::Ok { data, .. } => {
            if let ScannerData::Value(result) = data.as_ref()
                && let Some(inner) = result.get("result")
                && let Some(obj) = inner.as_object()
            {
                if obj.get("found").and_then(|v| v.as_bool()) == Some(true)
                    && let Some(id) = obj.get("id").and_then(|v| v.as_u64())
                {
                    return Ok(Some(id as u32));
                }
                return Ok(None);
            }
            Err(SelectorError::Backend(
                "Unexpected response format".to_string(),
            ))
        }
        ScannerProtocolResponse::Error { message, .. } => {
            Err(SelectorError::Backend(message.clone()))
        }
    }
}
