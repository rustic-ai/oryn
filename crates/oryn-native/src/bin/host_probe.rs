use std::time::Instant;

use oryn_native::host::{JavaScriptHost, RawV8Host};
use serde::Serialize;

#[derive(Serialize)]
struct ProbeResult {
    schema_version: u8,
    host: &'static str,
    iterations: usize,
    startup_ms: f64,
    execution_ms: f64,
}

fn main() {
    let iterations = std::env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(100);
    let started = Instant::now();
    let mut host = RawV8Host::new();
    let startup_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let execution_started = Instant::now();
    for _ in 0..iterations {
        host.execute("globalThis.__oryn = (globalThis.__oryn ?? 0) + 1")
            .expect("probe script must execute");
    }
    let result = ProbeResult {
        schema_version: 1,
        host: "raw_v8",
        iterations,
        startup_ms,
        execution_ms: execution_started.elapsed().as_secs_f64() * 1_000.0,
    };
    println!(
        "{}",
        serde_json::to_string(&result).expect("serialize result")
    );
}
