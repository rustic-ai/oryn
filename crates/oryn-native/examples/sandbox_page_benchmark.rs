use std::time::Instant;

use oryn_common::v3::SemanticAction;
use oryn_native::{Browser, BrowserConfig, NativeAction, WorkerLifecycle};
use serde::Serialize;

#[derive(Serialize)]
struct BenchmarkResult {
    schema_version: u8,
    pages: usize,
    page_handle_creation_ms: f64,
    first_cold_verification_launch_handshake_ms: f64,
    cached_isolated_launch_ms: Vec<f64>,
    warm_action_count: usize,
    warm_actions_per_second: f64,
    worker_lifecycle: String,
    reset_or_close_ms: Vec<f64>,
    recycled_javascript_state_erased: bool,
    total_observation_bytes: usize,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pages = std::env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(5);
    let warm_action_count = std::env::args()
        .nth(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(10_000);
    let worker_lifecycle = match std::env::var("ORYN_BENCH_WORKER_MODE").as_deref() {
        Ok("recycled") => WorkerLifecycle::Recycled {
            capacity: 1,
            max_leases: pages.max(1),
        },
        Ok("isolated-prewarmed") => WorkerLifecycle::IsolatedPrewarmed { capacity: pages },
        _ => WorkerLifecycle::IsolatedOnDemand,
    };
    let browser = Browser::try_with_config(BrowserConfig {
        worker_lifecycle,
        ..BrowserConfig::default()
    })?;
    browser.prepare_workers()?;
    let context = browser.new_context();
    let create_started = Instant::now();
    let handles = (0..pages).map(|_| context.new_page()).collect::<Vec<_>>();
    let page_handle_creation_ms = create_started.elapsed().as_secs_f64() * 1_000.0;
    let mut startup_to_page_ready_ms = Vec::with_capacity(pages);
    let mut total_observation_bytes = 0;
    let mut warm_actions_per_second = 0.0;
    let mut reset_or_close_ms = Vec::with_capacity(pages);
    let mut recycled_javascript_state_erased = true;
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
        if index > 0 && matches!(worker_lifecycle, WorkerLifecycle::Recycled { .. }) {
            recycled_javascript_state_erased &=
                page.evaluate("typeof globalThis.__orynChurn").await? == "undefined";
        }
        if index == 0 {
            let target = observation
                .nodes
                .iter()
                .find(|node| node.name == "Ready")
                .map(|node| node.semantic_ref)
                .ok_or("benchmark button was not observed")?;
            let actions = std::iter::repeat_n(
                NativeAction {
                    target,
                    action: SemanticAction::Click,
                    value: None,
                },
                warm_action_count,
            )
            .collect();
            let warm_started = Instant::now();
            let outputs = page.execute_batch(actions).await?;
            warm_actions_per_second = outputs.len() as f64 / warm_started.elapsed().as_secs_f64();
            page.evaluate("globalThis.__orynChurn = 'set'; 'ok'")
                .await?;
        }
        let close_started = Instant::now();
        page.close().await?;
        reset_or_close_ms.push(close_started.elapsed().as_secs_f64() * 1_000.0);
    }
    let first_cold_verification_launch_handshake_ms = startup_to_page_ready_ms
        .first()
        .copied()
        .unwrap_or_default();
    let cached_isolated_launch_ms = startup_to_page_ready_ms.into_iter().skip(1).collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&BenchmarkResult {
            schema_version: 5,
            pages,
            page_handle_creation_ms,
            first_cold_verification_launch_handshake_ms,
            cached_isolated_launch_ms,
            warm_action_count,
            warm_actions_per_second,
            worker_lifecycle: worker_lifecycle.name().into(),
            reset_or_close_ms,
            recycled_javascript_state_erased,
            total_observation_bytes,
        })?
    );
    Ok(())
}
