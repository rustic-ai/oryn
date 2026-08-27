use oryn_common::v2::{Effect, ProjectionProfile, SemanticAction, SemanticRef};
use oryn_native::{
    Browser,
    runtime::{NativeAction, RuntimeError},
};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct ChurnCase {
    id: &'static str,
    expectation: &'static str,
    observed: &'static str,
    passed: bool,
    false_preservation: bool,
}

#[derive(Debug, Serialize)]
struct Ratio {
    numerator: usize,
    denominator: usize,
    value: Option<f64>,
}

#[derive(Debug, Serialize)]
struct RecoveryCase {
    id: &'static str,
    trigger: &'static str,
    attempted: bool,
    recovered: bool,
}

#[derive(Debug, Serialize)]
struct Evidence {
    schema_version: u8,
    suite: &'static str,
    cases: Vec<ChurnCase>,
    reference_survival: Ratio,
    invalidation: Ratio,
    false_preservation: Ratio,
    recovery_cases: Vec<RecoveryCase>,
    recovery: Ratio,
    status: &'static str,
}

fn target(observation: &oryn_common::v2::Observation, name: &str) -> SemanticRef {
    observation
        .nodes
        .iter()
        .find(|node| node.name == name && node.actions.contains(&SemanticAction::Click))
        .unwrap_or_else(|| panic!("missing semantic target {name}"))
        .semantic_ref
}

async fn old_ref_is_invalidated(page: &oryn_native::PageHandle, semantic_ref: SemanticRef) -> bool {
    matches!(
        page.execute(NativeAction {
            target: semantic_ref,
            action: SemanticAction::Click,
            value: None,
        })
        .await,
        Err(RuntimeError::StaleSemanticRef | RuntimeError::WrongDocumentGeneration { .. })
    )
}

fn invalidation_case(id: &'static str, invalidated: bool) -> ChurnCase {
    ChurnCase {
        id,
        expectation: "invalidated",
        observed: if invalidated {
            "invalidated"
        } else {
            "preserved"
        },
        passed: invalidated,
        false_preservation: !invalidated,
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let page = Browser::new().new_context().new_page();
    let mut cases = Vec::new();

    page.load_html(
        "https://example.test/removed",
        "<button id=target>Removed target</button>",
    )
    .await?;
    let removed = target(&page.observe().await?, "Removed target");
    page.evaluate("document.querySelector('#target').remove();'removed'")
        .await?;
    cases.push(invalidation_case(
        "removed_node",
        old_ref_is_invalidated(&page, removed).await,
    ));

    page.load_html(
        "https://example.test/replaced",
        "<main id=host><button id=target>Replaced target</button></main>",
    )
    .await?;
    let replaced = target(&page.observe().await?, "Replaced target");
    page.evaluate(
        "const old=document.querySelector('#target');const fresh=document.createElement('button');fresh.id='target';fresh.textContent='Fresh target';old.parentNode.replaceChild(fresh,old);'replaced'",
    )
    .await?;
    cases.push(invalidation_case(
        "replaced_node",
        old_ref_is_invalidated(&page, replaced).await,
    ));

    page.load_html(
        "https://example.test/frame",
        r#"<iframe id=frame></iframe><script>
          const frame=document.querySelector('#frame');
          frame.contentDocument=document.createDocumentFragment();
          const action=document.createElement('button');action.textContent='Frame target';
          frame.contentDocument.appendChild(action);
        </script>"#,
    )
    .await?;
    let frame = target(
        &page.observe_profile(ProjectionProfile::Full).await?,
        "Frame target",
    );
    page.evaluate(
        "{const replacementFrame=document.querySelector('#frame');replacementFrame.contentDocument=document.createDocumentFragment();const freshFrameAction=document.createElement('button');freshFrameAction.textContent='Fresh frame target';replacementFrame.contentDocument.appendChild(freshFrameAction);}'frame replaced'",
    )
    .await?;
    cases.push(invalidation_case(
        "frame_replaced",
        old_ref_is_invalidated(&page, frame).await,
    ));

    page.load_html(
        "https://example.test/closed-shadow",
        r#"<open-box></open-box><closed-box></closed-box><script>
          class OpenBox extends HTMLElement { connectedCallback(){this.attachShadow({mode:'open'}).innerHTML='<button>Open target</button>';} }
          class ClosedBox extends HTMLElement { connectedCallback(){this.attachShadow({mode:'closed'}).innerHTML='<button>Closed target</button>';} }
          customElements.define('open-box',OpenBox);customElements.define('closed-box',ClosedBox);
        </script>"#,
    )
    .await?;
    let shadow = page.observe_profile(ProjectionProfile::Full).await?;
    let closed_isolated = shadow.nodes.iter().any(|node| node.name == "Open target")
        && !shadow.nodes.iter().any(|node| node.name == "Closed target");
    cases.push(ChurnCase {
        id: "closed_shadow_boundary",
        expectation: "isolated",
        observed: if closed_isolated {
            "isolated"
        } else {
            "exposed"
        },
        passed: closed_isolated,
        false_preservation: !closed_isolated,
    });

    page.load_html(
        "https://example.test/navigation-a",
        "<button>Navigation target</button>",
    )
    .await?;
    let navigation = target(&page.observe().await?, "Navigation target");
    page.load_html(
        "https://example.test/navigation-b",
        "<button>New document target</button>",
    )
    .await?;
    cases.push(invalidation_case(
        "navigation_generation",
        old_ref_is_invalidated(&page, navigation).await,
    ));

    page.load_html(
        "https://example.test/survival",
        "<button id=stable>Stable target</button><output id=status>Before</output>",
    )
    .await?;
    let stable = target(&page.observe().await?, "Stable target");
    page.evaluate("document.querySelector('#status').textContent='After';'mutated sibling'")
        .await?;
    let survived = page
        .execute(NativeAction {
            target: stable,
            action: SemanticAction::Click,
            value: None,
        })
        .await
        .is_ok();
    cases.push(ChurnCase {
        id: "unrelated_mutation_survival",
        expectation: "preserved",
        observed: if survived { "preserved" } else { "invalidated" },
        passed: survived,
        false_preservation: false,
    });

    page.load_html(
        "https://example.test/stale-recovery",
        r#"<button id=target onclick="document.querySelector('#status').textContent='Recovered stale reference'">Recovery target</button><output id=status>Pending</output>"#,
    )
    .await?;
    let stale_target = target(&page.observe().await?, "Recovery target");
    page.evaluate(
        "const old=document.querySelector('#target');const fresh=document.createElement('button');fresh.id='target';fresh.textContent='Recovery target';fresh.onclick=()=>document.querySelector('#status').textContent='Recovered stale reference';old.parentNode.replaceChild(fresh,old);'replaced'",
    )
    .await?;
    let stale_detected = old_ref_is_invalidated(&page, stale_target).await;
    let fresh_target = target(&page.observe().await?, "Recovery target");
    let stale_recovered = stale_detected
        && page
            .execute(NativeAction {
                target: fresh_target,
                action: SemanticAction::Click,
                value: None,
            })
            .await
            .is_ok()
        && page
            .extract(oryn_native::runtime::ContentKind::Text, None)
            .await?
            .as_str()
            .is_some_and(|text| text.contains("Recovered stale reference"));

    page.load_html(
        "https://example.test/no-progress-recovery",
        r#"<button id=noop>No progress</button><button id=recover onclick="document.querySelector('#status').textContent='Recovered no progress'">Recovery action</button><output id=status>Pending</output>"#,
    )
    .await?;
    let observation = page.observe().await?;
    let noop = target(&observation, "No progress");
    let recover = target(&observation, "Recovery action");
    page.execute(NativeAction {
        target: noop,
        action: SemanticAction::Focus,
        value: None,
    })
    .await?;
    let no_progress = page
        .execute(NativeAction {
            target: noop,
            action: SemanticAction::Click,
            value: None,
        })
        .await?;
    let no_progress_detected = no_progress
        .delta
        .as_ref()
        .is_some_and(|delta| delta.upserted.is_empty() && delta.removed.is_empty())
        && no_progress
            .effects
            .iter()
            .all(|effect| matches!(effect, Effect::Event { .. }));
    let no_progress_recovered = no_progress_detected
        && page
            .execute(NativeAction {
                target: recover,
                action: SemanticAction::Click,
                value: None,
            })
            .await
            .is_ok()
        && page
            .extract(oryn_native::runtime::ContentKind::Text, None)
            .await?
            .as_str()
            .is_some_and(|text| text.contains("Recovered no progress"));
    let recovery_cases = vec![
        RecoveryCase {
            id: "stale_reference_recovery",
            trigger: "stale_reference",
            attempted: stale_detected,
            recovered: stale_recovered,
        },
        RecoveryCase {
            id: "no_progress_recovery",
            trigger: "event_only_no_progress",
            attempted: no_progress_detected,
            recovered: no_progress_recovered,
        },
    ];

    let invalidations = cases
        .iter()
        .filter(|case| case.expectation == "invalidated")
        .collect::<Vec<_>>();
    let invalidation_denominator = invalidations.len();
    let invalidation_numerator = invalidations.iter().filter(|case| case.passed).count();
    let false_preservations = cases.iter().filter(|case| case.false_preservation).count();
    let recovery_attempts = recovery_cases.iter().filter(|case| case.attempted).count();
    let successful_recoveries = recovery_cases
        .iter()
        .filter(|case| case.attempted && case.recovered)
        .count();
    let passed = cases.iter().all(|case| case.passed)
        && recovery_attempts == recovery_cases.len()
        && successful_recoveries == recovery_attempts;
    let ratio = |numerator: usize, denominator: usize| Ratio {
        numerator,
        denominator,
        value: (denominator > 0).then(|| numerator as f64 / denominator as f64),
    };
    let evidence = Evidence {
        schema_version: 3,
        suite: "g2r-semantic-reference-churn",
        cases,
        reference_survival: ratio(usize::from(survived), 1),
        invalidation: ratio(invalidation_numerator, invalidation_denominator),
        false_preservation: ratio(false_preservations, invalidation_denominator + 1),
        recovery_cases,
        recovery: ratio(successful_recoveries, recovery_attempts),
        status: if passed { "passed" } else { "failed" },
    };
    println!("{}", serde_json::to_string_pretty(&evidence)?);
    if passed {
        Ok(())
    } else {
        Err("semantic-reference churn suite failed".into())
    }
}
