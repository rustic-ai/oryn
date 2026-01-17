use fantoccini::{Client, ClientBuilder};

pub struct WebDriverClient {
    pub client: Client,
}

impl WebDriverClient {
    pub async fn connect(
        url: &str,
        capabilities: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut caps = serde_json::Map::new();

        // Default capabilities (W3C standard)
        // If user provided caps, merge them
        if let Some(user_caps) = capabilities {
            for (k, v) in user_caps {
                caps.insert(k, v);
            }
        }

        // Connect
        let client = ClientBuilder::native()
            .capabilities(caps)
            .connect(url)
            .await
            .map_err(|e| format!("Failed to connect to WebDriver at {}: {}", url, e))?;

        Ok(Self { client })
    }

    pub async fn close(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.client
            .close()
            .await
            .map_err(|e| format!("Failed to close session: {}", e))?;
        Ok(())
    }
}
