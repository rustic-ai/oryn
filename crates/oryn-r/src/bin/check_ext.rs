use chromiumoxide::browser::{Browser, BrowserConfig};
use futures::StreamExt;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ext_path = std::env::current_dir()?.join("extension");
    println!("Loading extension from {:?}", ext_path);

    let config = BrowserConfig::builder()
        .no_sandbox()
        .args(vec![
            format!("--load-extension={}", ext_path.display()),
            format!("--disable-extensions-except={}", ext_path.display()),
        ])
        .build()?;

    let (mut browser, mut handler) = Browser::launch(config).await?;

    tokio::spawn(async move {
        while let Some(h) = handler.next().await {
            if let Err(e) = h {
                eprintln!("Handler error: {}", e);
            }
        }
    });

    println!("Waiting for extension to load...");
    tokio::time::sleep(Duration::from_secs(10)).await;

    let pages = browser.pages().await?;
    println!("Found {} pages", pages.len());
    for page in pages {
        println!("Page: {}", page.url().await?.unwrap_or_default());
    }

    browser.close().await?;
    Ok(())
}
