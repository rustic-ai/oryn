use std::{collections::BTreeSet, fs};

use oryn_common::v3::{SemanticAction, SemanticRef};
use oryn_native::{
    Browser, NativeAction,
    trace::{CausalRelation, TraceEventKind},
};
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
    html: String,
    target: String,
    expected: Vec<Expected>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Expected {
    label_id: String,
    relation: String,
    kind: String,
}

#[derive(Debug, Serialize)]
struct Observed {
    sequence: u64,
    action_id: u64,
    relation: String,
    kind: String,
    matched_label_id: Option<String>,
    source_relation: String,
}

#[derive(Debug, Serialize)]
struct LedgerCase {
    case_id: String,
    expected: Vec<Expected>,
    observed: Vec<Observed>,
    passed: bool,
}

fn relation(value: CausalRelation) -> &'static str {
    match value {
        CausalRelation::Direct => "direct",
        CausalRelation::Derived => "derived",
        CausalRelation::Correlated => "correlated",
        CausalRelation::Unattributed => "unattributed",
    }
}

fn kind(value: &TraceEventKind) -> &'static str {
    match value {
        TraceEventKind::EventDispatched { .. } => "event_dispatched",
        TraceEventKind::Mutation { .. } => "mutation",
        TraceEventKind::Console { .. } => "console",
        _ => "other",
    }
}

fn target(observation: &oryn_common::v3::Observation, name: &str) -> SemanticRef {
    observation
        .nodes
        .iter()
        .find(|node| node.name == name && node.actions.contains(&SemanticAction::Click))
        .expect("controlled causal target")
        .semantic_ref
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "benchmarks/conformance/semantic-causality-v1.json".into());
    let manifest: Manifest = serde_json::from_str(&fs::read_to_string(path)?)?;
    if manifest.schema_version != 1 {
        return Err("unsupported causal manifest".into());
    }
    let browser = Browser::in_process_probe();
    let context = browser.new_context();
    let page = context.new_page();
    let mut ledger = Vec::new();
    for case in manifest.cases {
        page.load_html(format!("https://causal.test/{}", case.id), case.html)
            .await?;
        let before_sequence = page
            .trace()
            .await?
            .last()
            .map_or(0, |event| event.sequence + 1);
        let semantic_ref = target(&page.observe().await?, &case.target);
        let result = page
            .execute(NativeAction {
                target: semantic_ref,
                action: SemanticAction::Click,
                value: None,
            })
            .await?;
        let expected_keys = case
            .expected
            .iter()
            .map(|item| {
                (
                    item.relation.as_str(),
                    item.kind.as_str(),
                    item.label_id.as_str(),
                )
            })
            .collect::<Vec<_>>();
        let observed = page
            .trace()
            .await?
            .into_iter()
            .filter(|event| {
                event.sequence >= before_sequence && event.action_id == Some(result.action_id)
            })
            .filter(|event| {
                matches!(
                    event.kind,
                    TraceEventKind::EventDispatched { .. }
                        | TraceEventKind::Mutation { .. }
                        | TraceEventKind::Console { .. }
                )
            })
            .map(|event| {
                let observed_relation = relation(event.causality);
                let observed_kind = kind(&event.kind);
                let matched_label_id = expected_keys
                    .iter()
                    .find(|(expected_relation, expected_kind, _)| {
                        *expected_relation == observed_relation && *expected_kind == observed_kind
                    })
                    .map(|(_, _, label)| (*label).to_string());
                Observed {
                    sequence: event.sequence,
                    action_id: result.action_id.0,
                    relation: observed_relation.into(),
                    kind: observed_kind.into(),
                    matched_label_id,
                    source_relation: observed_relation.into(),
                }
            })
            .collect::<Vec<_>>();
        let matched = observed
            .iter()
            .filter_map(|item| item.matched_label_id.clone())
            .collect::<BTreeSet<_>>();
        let expected = case
            .expected
            .iter()
            .map(|item| item.label_id.clone())
            .collect::<BTreeSet<_>>();
        let passed = matched == expected && observed.len() == expected.len();
        ledger.push(LedgerCase {
            case_id: case.id,
            expected: case.expected,
            observed,
            passed,
        });
    }
    let passed = ledger.iter().all(|case| case.passed);
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "suite": manifest.suite,
            "ledger": ledger,
            "status": if passed { "passed" } else { "failed" }
        }))?
    );
    if passed {
        Ok(())
    } else {
        Err("causal evidence mismatch".into())
    }
}
