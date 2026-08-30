use std::fs;

use oryn_native::challenge::{ChallengeDisposition, assess_challenge};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct Manifest {
    schema_version: u8,
    suite: String,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    id: String,
    status: u16,
    headers: Vec<(String, String)>,
    html: String,
    expected: String,
    provider: Option<String>,
}

#[derive(Debug, Serialize)]
struct LedgerCase {
    id: String,
    expected: String,
    observed: String,
    expected_provider: Option<String>,
    observed_provider: Option<String>,
    signals: Vec<String>,
    passed: bool,
}

fn disposition(value: ChallengeDisposition) -> &'static str {
    match value {
        ChallengeDisposition::NotDetected => "not_detected",
        ChallengeDisposition::Suspected => "suspected",
        ChallengeDisposition::Confirmed => "confirmed",
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "benchmarks/conformance/challenge-detection-v1.json".into());
    let manifest: Manifest = serde_json::from_str(&fs::read_to_string(path)?)?;
    if manifest.schema_version != 1 {
        return Err("unsupported challenge manifest".into());
    }
    let ledger = manifest
        .cases
        .into_iter()
        .map(|case| {
            let assessment = assess_challenge(case.status, &case.headers, &case.html);
            let observed = disposition(assessment.disposition).to_string();
            let passed = observed == case.expected && assessment.provider == case.provider;
            LedgerCase {
                id: case.id,
                expected: case.expected,
                observed,
                expected_provider: case.provider,
                observed_provider: assessment.provider,
                signals: assessment
                    .signals
                    .into_iter()
                    .map(|signal| signal.marker)
                    .collect(),
                passed,
            }
        })
        .collect::<Vec<_>>();
    let passed = ledger.iter().all(|case| case.passed);
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "suite": manifest.suite,
            "ledger": ledger,
            "confirmed_precision": {
                "numerator": ledger.iter().filter(|case| case.observed == "confirmed" && case.expected == "confirmed").count(),
                "denominator": ledger.iter().filter(|case| case.observed == "confirmed").count()
            },
            "confirmed_recall": {
                "numerator": ledger.iter().filter(|case| case.observed == "confirmed" && case.expected == "confirmed").count(),
                "denominator": ledger.iter().filter(|case| case.expected == "confirmed").count()
            },
            "status": if passed { "passed" } else { "failed" }
        }))?
    );
    if passed {
        Ok(())
    } else {
        Err("challenge evidence mismatch".into())
    }
}
