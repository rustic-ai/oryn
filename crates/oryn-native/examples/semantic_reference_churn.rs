use oryn_common::v3::{Effect, ProjectionProfile, SemanticAction, SemanticRef};
use oryn_native::{
    Browser,
    runtime::{NativeAction, RuntimeError},
};
use serde::Serialize;
use std::{
    io::{Read, Write},
    net::TcpListener,
};

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
    runtime: oryn_native::RuntimeBuildInfo,
    cases: Vec<ChurnCase>,
    reference_survival: Ratio,
    invalidation: Ratio,
    false_preservation: Ratio,
    recovery_cases: Vec<RecoveryCase>,
    recovery: Ratio,
    worker_replacement: WorkerReplacement,
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct WorkerReplacement {
    infinite_loop_terminated: bool,
    parent_survived: bool,
    blank_generation_started: bool,
    old_reference_invalidated: bool,
    replacement_generation: u64,
    termination_latency_ms: f64,
    restart_to_blank_ms: f64,
    status: &'static str,
}

fn target(observation: &oryn_common::v3::Observation, name: &str) -> SemanticRef {
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

async fn survival_case(
    page: &oryn_native::PageHandle,
    id: &'static str,
    html: &str,
    mutation: &str,
) -> Result<ChurnCase, Box<dyn std::error::Error>> {
    page.load_html(format!("https://example.test/{id}"), html)
        .await?;
    let semantic_ref = target(&page.observe().await?, "Stable target");
    page.evaluate(mutation).await?;
    let survived = page
        .execute(NativeAction {
            target: semantic_ref,
            action: SemanticAction::Click,
            value: None,
        })
        .await
        .is_ok();
    Ok(ChurnCase {
        id,
        expectation: "preserved",
        observed: if survived { "preserved" } else { "invalidated" },
        passed: survived,
        false_preservation: false,
    })
}

async fn scripted_invalidation_case(
    page: &oryn_native::PageHandle,
    id: &'static str,
    html: &str,
    mutation: &str,
) -> Result<ChurnCase, Box<dyn std::error::Error>> {
    page.load_html(format!("https://example.test/{id}"), html)
        .await?;
    let semantic_ref = target(&page.observe().await?, "Replace target");
    page.evaluate(mutation).await?;
    Ok(invalidation_case(
        id,
        old_ref_is_invalidated(page, semantic_ref).await,
    ))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = oryn_native::production_runtime_build_info()?;
    let browser = Browser::new();
    let context = browser.new_context_with_options(oryn_native::ContextOptions::loopback_test());
    let page = context.new_page();
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

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let frame_address = listener.local_addr()?;
    let frame_server = std::thread::spawn(move || -> std::io::Result<()> {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept()?;
            let mut request = [0_u8; 2048];
            let length = stream.read(&mut request)?;
            let request = String::from_utf8_lossy(&request[..length]);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/");
            let body = if path == "/frame" {
                "<button>Frame target</button>"
            } else {
                "<iframe id=frame src=/frame></iframe>"
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes())?;
        }
        Ok(())
    });
    page.goto(format!("http://{frame_address}/")).await?;
    let frame = target(
        &page.observe_profile(ProjectionProfile::Full).await?,
        "Frame target",
    );
    page.evaluate(
        "{const old=document.querySelector('#frame');const fresh=document.createElement('iframe');fresh.id='frame';old.parentNode.replaceChild(fresh,old);}'frame replaced'",
    )
    .await?;
    cases.push(invalidation_case(
        "frame_replaced",
        old_ref_is_invalidated(&page, frame).await,
    ));
    frame_server.join().expect("frame fixture server")?;

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
        expectation: "identity_control",
        observed: if survived { "preserved" } else { "invalidated" },
        passed: survived,
        false_preservation: false,
    });

    cases.push(
        survival_case(
            &page,
            "target_attribute_survival",
            "<button id=stable>Stable target</button>",
            "document.querySelector('#stable').setAttribute('data-state','changed');'attribute'",
        )
        .await?,
    );
    cases.push(
        survival_case(
            &page,
            "target_text_survival",
            "<button id=stable aria-label='Stable target'>Before</button>",
            "document.querySelector('#stable').textContent='After';'text'",
        )
        .await?,
    );
    cases.push(
        survival_case(
            &page,
            "form_value_survival",
            "<button id=stable>Stable target</button><input id=value value=before>",
            "document.querySelector('#value').value='after';'value'",
        )
        .await?,
    );
    cases.push(
        survival_case(
            &page,
            "sibling_insertion_survival",
            "<main id=host><button id=stable>Stable target</button></main>",
            "const sibling=document.createElement('span');sibling.textContent='new';document.querySelector('#host').appendChild(sibling);'inserted'",
        )
        .await?,
    );
    cases.push(
        survival_case(
            &page,
            "sibling_removal_survival",
            "<main><span id=sibling>remove</span><button id=stable>Stable target</button></main>",
            "document.querySelector('#sibling').remove();'removed sibling'",
        )
        .await?,
    );
    cases.push(
        survival_case(
            &page,
            "ancestor_attribute_survival",
            "<main id=host><button id=stable>Stable target</button></main>",
            "document.querySelector('#host').className='changed';'ancestor'",
        )
        .await?,
    );
    cases.push(
        survival_case(
            &page,
            "same_document_reparent_survival",
            "<main id=from><button id=stable>Stable target</button></main><aside id=to></aside>",
            "document.querySelector('#to').appendChild(document.querySelector('#stable'));'reparented'",
        )
        .await?,
    );
    cases.push(
        survival_case(
            &page,
            "open_shadow_unrelated_survival",
            "<button id=stable>Stable target</button><div id=host></div>",
            "const root=document.querySelector('#host').attachShadow({mode:'open'});root.innerHTML='<span>shadow change</span>';'shadow'",
        )
        .await?,
    );

    cases.push(
        scripted_invalidation_case(
            &page,
            "inner_html_recreation",
            "<main id=host><button id=target>Replace target</button></main>",
            "document.querySelector('#host').innerHTML='<button id=target>Replace target</button>';'innerHTML'",
        )
        .await?,
    );
    cases.push(
        scripted_invalidation_case(
            &page,
            "react_style_key_replacement",
            "<main id=root><button id=target data-key=old>Replace target</button></main>",
            "const root=document.querySelector('#root');const fresh=document.createElement('button');fresh.id='target';fresh.setAttribute('data-key','new');fresh.textContent='Replace target';root.replaceChild(fresh,document.querySelector('#target'));'react-style replacement'",
        )
        .await?,
    );
    cases.push(
        scripted_invalidation_case(
            &page,
            "vue_style_conditional_replacement",
            "<main id=root><button id=target data-branch=a>Replace target</button></main>",
            "const old=document.querySelector('#target');const fresh=document.createElement('button');fresh.id='target';fresh.setAttribute('data-branch','b');fresh.textContent='Replace target';old.parentNode.replaceChild(fresh,old);'vue-style replacement'",
        )
        .await?,
    );

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

    page.load_html(
        "https://example.test/worker-limit",
        "<button>Pre-crash target</button>",
    )
    .await?;
    let before_limit = page.observe().await?;
    let pre_crash_target = target(&before_limit, "Pre-crash target");
    let termination_started = std::time::Instant::now();
    let infinite_loop_terminated = matches!(
        page.evaluate("for(;;){}").await,
        Err(RuntimeError::WorkerLimitExceeded { .. } | RuntimeError::WorkerCrashed(_))
    );
    let termination_latency_ms = termination_started.elapsed().as_secs_f64() * 1_000.0;
    let restart_started = std::time::Instant::now();
    let blank = page.observe().await?;
    let restart_to_blank_ms = restart_started.elapsed().as_secs_f64() * 1_000.0;
    let parent_survived = infinite_loop_terminated;
    let blank_generation_started = blank.page.document_generation
        > before_limit.page.document_generation
        && blank.nodes.is_empty();
    let old_reference_invalidated = old_ref_is_invalidated(&page, pre_crash_target).await;
    let worker_replacement_passed = infinite_loop_terminated
        && parent_survived
        && blank_generation_started
        && old_reference_invalidated;
    let worker_replacement = WorkerReplacement {
        infinite_loop_terminated,
        parent_survived,
        blank_generation_started,
        old_reference_invalidated,
        replacement_generation: blank.page.document_generation,
        termination_latency_ms,
        restart_to_blank_ms,
        status: if worker_replacement_passed {
            "passed"
        } else {
            "failed"
        },
    };
    cases.push(invalidation_case(
        "worker_replacement",
        old_reference_invalidated,
    ));

    let invalidations = cases
        .iter()
        .filter(|case| case.expectation == "invalidated")
        .collect::<Vec<_>>();
    let invalidation_denominator = invalidations.len();
    let invalidation_numerator = invalidations.iter().filter(|case| case.passed).count();
    let survivals = cases
        .iter()
        .filter(|case| case.expectation == "preserved")
        .collect::<Vec<_>>();
    let survival_denominator = survivals.len();
    let survival_numerator = survivals.iter().filter(|case| case.passed).count();
    let false_preservations = cases.iter().filter(|case| case.false_preservation).count();
    let recovery_attempts = recovery_cases.iter().filter(|case| case.attempted).count();
    let successful_recoveries = recovery_cases
        .iter()
        .filter(|case| case.attempted && case.recovered)
        .count();
    let passed = cases.iter().all(|case| case.passed)
        && survival_denominator == 8
        && survival_numerator == survival_denominator
        && invalidation_denominator == 8
        && recovery_attempts == recovery_cases.len()
        && successful_recoveries == recovery_attempts
        && worker_replacement_passed;
    let ratio = |numerator: usize, denominator: usize| Ratio {
        numerator,
        denominator,
        value: (denominator > 0).then(|| numerator as f64 / denominator as f64),
    };
    let evidence = Evidence {
        schema_version: 5,
        suite: "semantic-reference-churn",
        runtime,
        cases,
        reference_survival: ratio(survival_numerator, survival_denominator),
        invalidation: ratio(invalidation_numerator, invalidation_denominator),
        false_preservation: ratio(false_preservations, invalidation_denominator),
        recovery_cases,
        recovery: ratio(successful_recoveries, recovery_attempts),
        worker_replacement,
        status: if passed { "passed" } else { "failed" },
    };
    println!("{}", serde_json::to_string_pretty(&evidence)?);
    if passed {
        Ok(())
    } else {
        Err("semantic-reference churn suite failed".into())
    }
}
