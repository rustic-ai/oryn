use std::time::Instant;

use oryn_native::Browser;
use serde::Serialize;

#[derive(Serialize)]
struct BenchmarkResult {
    schema_version: u8,
    pages: usize,
    page_handle_creation_ms: f64,
    startup_to_page_ready_ms: Vec<f64>,
    total_observation_bytes: usize,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pages = std::env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(5);
    let browser = Browser::new();
    let context = browser.new_context();
    let create_started = Instant::now();
    let handles = (0..pages).map(|_| context.new_page()).collect::<Vec<_>>();
    let page_handle_creation_ms = create_started.elapsed().as_secs_f64() * 1_000.0;
    let mut startup_to_page_ready_ms = Vec::with_capacity(pages);
    let mut total_observation_bytes = 0;
    for (index, page) in handles.into_iter().enumerate() {
        let started = Instant::now();
        page.load_html(
            format!("https://example.test/page-{index}"),
            format!("<title>Page {index}</title><button>Ready</button>"),
        )
        .await?;
        let observation = page.observe().await?;
        startup_to_page_ready_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
        total_observation_bytes += serde_json::to_vec(&observation)?.len();
        page.close().await?;
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&BenchmarkResult {
            schema_version: 4,
            pages,
            page_handle_creation_ms,
            startup_to_page_ready_ms,
            total_observation_bytes,
        })?
    );
    Ok(())
}
