use serde_json::json;

/// Standard port for COG WebDriver/Remote Automation
pub const DEFAULT_COG_PORT: u16 = 8080;

/// Returns the default WebDriver URL for a local COG instance
pub fn default_cog_url() -> String {
    format!("http://localhost:{}", DEFAULT_COG_PORT)
}

/// Returns standard capabilities required for WPE/COG
pub fn wpe_capabilities() -> serde_json::Map<String, serde_json::Value> {
    let mut caps = serde_json::Map::new();
    // WPE often uses 'browserName': 'wpe' or 'Cog'
    caps.insert("browserName".to_string(), json!("wpe"));
    caps.insert(
        "wpe:browserOptions".to_string(),
        json!({
            "binary": "/usr/bin/cog",
            "args": ["--platform=fdo"]
        }),
    );
    caps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_url() {
        assert_eq!(default_cog_url(), "http://localhost:8080");
    }

    #[test]
    fn test_wpe_capabilities() {
        let caps = wpe_capabilities();
        assert_eq!(caps.get("browserName").unwrap(), "wpe");

        let opts = caps.get("wpe:browserOptions").unwrap();
        assert!(opts.is_object());

        let path = opts.get("binary").unwrap().as_str().unwrap();
        assert_eq!(path, "/usr/bin/cog");
    }
}
