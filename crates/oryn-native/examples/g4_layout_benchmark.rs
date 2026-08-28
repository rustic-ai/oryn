use std::time::Instant;

use oryn_common::v3::ProjectionProfile;
use oryn_common::v4::{ObservationOptions, ViewportState};
use oryn_native::Browser;
use serde::Serialize;

#[derive(Serialize)]
struct LayoutBenchmark {
    schema_version: u8,
    nodes: usize,
    samples: usize,
    cold_page_to_geometry_ms: f64,
    reflow_p50_ms: f64,
    reflow_p95_ms: f64,
    observation_bytes: usize,
    passed: bool,
}

fn percentile(values: &mut [f64], fraction: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    let index = ((values.len() - 1) as f64 * fraction).floor() as usize;
    values[index]
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let nodes = std::env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(400usize);
    let samples = std::env::args()
        .nth(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(20usize)
        .max(1);
    let cards = (0..nodes)
        .map(|index| format!("<button class=card>Card {index}</button>"))
        .collect::<String>();
    let html = format!(
        "<style>.grid{{display:grid;grid-template-columns:repeat(4,1fr);gap:4px;width:1200px}}.card{{height:28px}}</style><main class=grid>{cards}</main>"
    );
    // Keep the context owner alive for the duration of the benchmark. Page
    // workers intentionally hold only a weak context reference so dropping a
    // context closes its pages and parent-owned broker services.
    let context = Browser::new().new_context();
    let page = context.new_page();
    let cold = Instant::now();
    page.load_html("https://g4.example.test/benchmark", html)
        .await?;
    let first = page
        .observe_with(ObservationOptions {
            profile: ProjectionProfile::Full,
            include_geometry: true,
            viewport_only: false,
            include_hidden: true,
        })
        .await?;
    let cold_page_to_geometry_ms = cold.elapsed().as_secs_f64() * 1_000.0;
    let observation_bytes = serde_json::to_vec(&first)?.len();
    let semantic_cards = first
        .nodes
        .iter()
        .filter(|node| node.semantic.role == "button" && node.semantic.name.starts_with("Card "))
        .count();
    let mut timings = Vec::with_capacity(samples);
    for sample in 0..samples {
        let started = Instant::now();
        page.set_viewport(ViewportState {
            width: if sample % 2 == 0 { 1280.0 } else { 960.0 },
            height: 720.0,
            device_pixel_ratio: 1.0,
        })
        .await?;
        let observation = page
            .observe_with(ObservationOptions {
                profile: ProjectionProfile::Compact,
                include_geometry: true,
                viewport_only: false,
                include_hidden: true,
            })
            .await?;
        if observation.nodes.is_empty() {
            return Err("layout observation unexpectedly empty".into());
        }
        timings.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let mut p50_values = timings.clone();
    let reflow_p50_ms = percentile(&mut p50_values, 0.50);
    let reflow_p95_ms = percentile(&mut timings, 0.95);
    println!(
        "{}",
        serde_json::to_string_pretty(&LayoutBenchmark {
            schema_version: 1,
            nodes,
            samples,
            cold_page_to_geometry_ms,
            reflow_p50_ms,
            reflow_p95_ms,
            observation_bytes,
            passed: semantic_cards == nodes,
        })?
    );
    Ok(())
}
