use std::{env, time::Instant};

use oryn_native::Browser;
use serde::Serialize;
use tokio::task::JoinSet;

const DOCUMENT: &str = r#"<!doctype html><title>Probe</title><main><label>Email<input aria-label="Email"></label><button>Save</button></main>"#;

#[derive(Serialize)]
struct Measurement {
    schema_version: u8,
    iterations: usize,
    page_count: usize,
    page_creation_ms: f64,
    load_observe_ms: f64,
    observation_bytes: usize,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let iterations = argument(1, 10);
    let page_count = argument(2, 20);
    let mut measurements = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        measurements.push(measure(page_count).await);
    }
    println!(
        "{}",
        serde_json::to_string(&measurements).expect("serialize native probe")
    );
}

async fn measure(page_count: usize) -> Measurement {
    let browser = Browser::in_process_probe();
    let context = browser.new_context();
    let create_started = Instant::now();
    let pages: Vec<_> = (0..page_count).map(|_| context.new_page()).collect();
    let page_creation_ms = create_started.elapsed().as_secs_f64() * 1000.0;

    let execution_started = Instant::now();
    let mut tasks = JoinSet::new();
    for page in pages {
        tasks.spawn(async move {
            page.load_html("https://example.test/probe", DOCUMENT)
                .await
                .expect("load probe document");
            let observation = page.observe().await.expect("observe probe document");
            page.close().await.expect("close probe page");
            serde_json::to_vec(&observation)
                .expect("serialize observation")
                .len()
        });
    }
    let mut observation_bytes = 0;
    while let Some(result) = tasks.join_next().await {
        observation_bytes += result.expect("page task");
    }

    Measurement {
        schema_version: 1,
        iterations: 1,
        page_count,
        page_creation_ms,
        load_observe_ms: execution_started.elapsed().as_secs_f64() * 1000.0,
        observation_bytes,
    }
}

fn argument(index: usize, default: usize) -> usize {
    env::args()
        .nth(index)
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
