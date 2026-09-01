use std::time::Instant;

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
    let mut runtime = deno_core::JsRuntime::new(deno_core::RuntimeOptions::default());
    let startup_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let execution_started = Instant::now();
    for _ in 0..iterations {
        runtime
            .execute_script(
                "oryn:probe",
                "globalThis.__oryn = (globalThis.__oryn ?? 0) + 1".to_string(),
            )
            .expect("probe script must execute");
    }
    let result = ProbeResult {
        schema_version: 1,
        host: "deno_core",
        iterations,
        startup_ms,
        execution_ms: execution_started.elapsed().as_secs_f64() * 1_000.0,
    };
    println!(
        "{}",
        serde_json::to_string(&result).expect("serialize result")
    );
}
