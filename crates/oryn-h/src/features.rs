use chromiumoxide::Page;
use futures::StreamExt;
use std::error::Error;
use std::path::Path;

pub async fn generate_pdf(
    page: &Page,
    output_path: &Path,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let pdf_data = page
        .pdf(chromiumoxide::cdp::browser_protocol::page::PrintToPdfParams::default())
        .await
        .map_err(|e| format!("PDF generation failed: {}", e))?;

    tokio::fs::write(output_path, pdf_data)
        .await
        .map_err(|e| format!("Failed to write PDF to file: {}", e))?;

    Ok(())
}

pub async fn enable_network_logging(page: &Page) -> Result<(), Box<dyn Error + Send + Sync>> {
    // Enable network domain?
    // chromiumoxide automatically handles some domains?
    // We want to log RequestWillBeSent.

    let mut request_events = page
        .event_listener::<chromiumoxide::cdp::browser_protocol::network::EventRequestWillBeSent>()
        .await
        .map_err(|e| format!("Failed to subscribe to network events: {}", e))?;

    tokio::spawn(async move {
        while let Some(event) = request_events.next().await {
            tracing::info!(
                "Network Request: [{}] {}",
                event.request.method,
                event.request.url
            );
        }
    });

    Ok(())
}
