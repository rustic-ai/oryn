//! Shared-parent workflow executor. Fixture/oracle ownership stays in the Python
//! controller; this process supplies genuine OIL outputs and native lifecycle evidence.
use clap::Parser;
use oryn_native::oil::NativeContextOilSession;
use oryn_native::workflow_instrumentation::{FaultKind, FaultPlan, WorkflowAudit};
use oryn_native::{Browser, BrowserConfig, ConsequentialAction, ContextOptions, PolicyRule};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    output: PathBuf,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    schema_version: u32,
    attempt_id: String,
    sessions: Vec<SessionPlan>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionPlan {
    id: String,
    #[serde(default)]
    secrets: Vec<Secret>,
    steps: Vec<Step>,
    #[serde(default)]
    recovery_steps: Vec<Step>,
    #[serde(default)]
    commit_recovery: Option<CommitRecovery>,
    #[serde(default)]
    adversarial_for: Option<String>,
    #[serde(default)]
    uploads: Vec<Upload>,
    fault: Option<FaultPlan>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommitRecovery {
    outcome_url: String,
    resume_after: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    oil: String,
    #[serde(default)]
    capture_reference: Option<CaptureReference>,
    #[serde(default)]
    use_reference: Option<String>,
    #[serde(default)]
    expected_error: Option<String>,
    #[serde(default = "deadline")]
    timeout_ms: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureReference {
    slot: String,
    name: String,
}
async fn wait_commit_gate(
    gate: &oryn_native::workflow_instrumentation::CommitGate,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        match std::fs::read(&gate.marker) {
            Ok(bytes) => {
                if bytes.len() > 4096 {
                    return Err("oversized commit marker".into());
                }
                let value: Value =
                    serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
                if value["schema_version"] != 1
                    || value["token"].as_str() != Some(gate.token.as_str())
                    || value["phase"].as_str() != Some(gate.phase.as_str())
                    || value["started_ns"].as_u64().is_none()
                {
                    return Err("commit marker identity mismatch".into());
                }
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("commit marker deadline".into());
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
async fn release_commit_gate(
    gate: &oryn_native::workflow_instrumentation::CommitGate,
) -> Result<(), String> {
    use std::io::Write;
    let temporary = gate.release.with_extension("publishing");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    let bytes=serde_json::to_vec(&json!({"schema_version":1,"token":gate.token,"phase":gate.phase,"worker_reaped":true,"interrupted_inflight":true})).map_err(|error|error.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| error.to_string())?;
    if gate.release.exists() {
        return Err("release marker already exists".into());
    }
    std::fs::rename(temporary, &gate.release).map_err(|error| error.to_string())?;
    let done = PathBuf::from(format!("{}.done", gate.marker.display()));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(bytes) = std::fs::read(&done)
            && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
        {
            let terminal = if gate.phase == "before_commit" {
                "aborted_before_commit"
            } else {
                "released_after_commit"
            };
            if value["token"].as_str() != Some(gate.token.as_str())
                || value["phase"].as_str() != Some(gate.phase.as_str())
                || value["terminal"] != terminal
            {
                return Err("fixture release acknowledgement mismatch".into());
            }
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("fixture release acknowledgement deadline".into());
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
fn deadline() -> u64 {
    60_000
}
fn step_input(step: &Step, references: &BTreeMap<String, Value>) -> Result<String, String> {
    let Some(slot) = &step.use_reference else {
        return Ok(step.oil.clone());
    };
    let reference = references
        .get(slot)
        .ok_or_else(|| "unknown captured native reference".to_owned())?;
    let request = json!({"contract_version":oryn_common::agent::AGENT_INTENT_CONTRACT_VERSION,"intent":{"intent":"click","target":{"kind":"reference","reference":reference},"force":false}});
    Ok(format!(
        "{}{}",
        oryn_native::oil::TYPED_INTENT_PREFIX,
        request
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Upload {
    label: String,
    path: PathBuf,
    mime: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Secret {
    label: String,
    path: PathBuf,
    origin: String,
}
fn validate(plan: &Plan) -> Result<(), String> {
    if plan.schema_version != 1
        || plan.attempt_id.is_empty()
        || !matches!(
            plan.sessions
                .iter()
                .filter(|session| session.adversarial_for.is_none())
                .count(),
            1 | 5 | 20
        )
        || plan.sessions.len() > 40
    {
        return Err("invalid schema, attempt or context count".into());
    }
    let mut ids = BTreeSet::new();
    for session in &plan.sessions {
        if session.id.is_empty()
            || !ids.insert(&session.id)
            || session.steps.is_empty()
            || session.steps.len() > 512
        {
            return Err("invalid session identity or steps".into());
        }
        for step in session.steps.iter().chain(&session.recovery_steps) {
            if step.oil.trim().is_empty()
                || step.oil.lines().count() != 1
                || !(1..=120_000).contains(&step.timeout_ms)
                || step
                    .expected_error
                    .as_ref()
                    .is_some_and(|value| value.is_empty())
            {
                return Err("invalid OIL step/deadline/error assertion".into());
            }
        }
        if let Some(fault) = &session.fault {
            fault.validate(session.steps.len())?;
        }
        if let Some(recovery) = &session.commit_recovery
            && (recovery
                .outcome_url
                .chars()
                .any(|character| matches!(character, '\n' | '\r' | '"'))
                || !recovery.outcome_url.starts_with("http://127.0.0.1:")
                || recovery.resume_after >= session.steps.len()
                || session
                    .fault
                    .as_ref()
                    .and_then(|fault| fault.commit_gate.as_ref())
                    .is_none())
        {
            return Err("invalid conditional commit recovery".into());
        }
        let mut labels = BTreeSet::new();
        for upload in &session.uploads {
            if upload.label.is_empty() || !labels.insert(&upload.label) || upload.mime.is_empty() {
                return Err("invalid upload authority".into());
            }
        }
    }
    for session in &plan.sessions {
        if let Some(owner) = &session.adversarial_for
            && !plan
                .sessions
                .iter()
                .any(|candidate| candidate.id == *owner && candidate.adversarial_for.is_none())
        {
            return Err("adversarial context requires an explicit cohort owner".into());
        }
    }
    Ok(())
}

fn oil_failure_evidence(error: oryn_native::oil::NativeOilError) -> Value {
    let message = error.to_string();
    let outputs = error.completed_outputs().to_vec();
    match error.into_runtime() {
        Ok(native) => json!({"outputs": outputs, "error": message, "native_error": native}),
        Err(error) => json!({"outputs": outputs, "error": message,
            "evidence_error": format!("native failure serialization failed: {error}")}),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let encoded = std::fs::read(&args.input)?;
    if encoded.len() > 8 * 1024 * 1024 {
        anyhow::bail!("plan exceeds bound");
    }
    let plan: Plan = serde_json::from_slice(&encoded)?;
    validate(&plan).map_err(anyhow::Error::msg)?;
    // Reserve the evidence path before creating any worker or external request.
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args.output)?;
    let lifecycle_audit = oryn_native::worker_lifecycle_audit::WorkerLifecycleAudit::start()
        .map_err(std::io::Error::other)?;
    let runtime = oryn_native::production_runtime_build_info_with_config(
        BrowserConfig::default(),
        ContextOptions::loopback_test(),
    )?;
    let browser = Browser::try_with_config(BrowserConfig::default())?;
    browser.prepare_workers()?;
    let audit = WorkflowAudit::new();
    let barrier = Arc::new(tokio::sync::Barrier::new(plan.sessions.len()));
    let mut tasks = tokio::task::JoinSet::new();
    for session_plan in plan.sessions {
        let browser = browser.clone();
        let audit = audit.clone();
        let barrier = barrier.clone();
        tasks.spawn(async move {
            let mut options=ContextOptions::loopback_test();
            if !session_plan.uploads.is_empty() {options.action_policy.rules.insert(ConsequentialAction::FileUpload,PolicyRule::Allow);}
            let context=browser.new_audited_context_with_isolated_auxiliaries(options);
            let mut errors=Vec::<String>::new();
            for upload in &session_plan.uploads {
                match std::fs::read(&upload.path) {
                    Ok(bytes)=>{let filename=upload.path.file_name().and_then(|name|name.to_str()).unwrap_or("upload.bin");context.import_upload_as(&upload.label,filename,&upload.mime,bytes);},
                    Err(error)=>errors.push(format!("authorized upload unavailable: {error}")),
                }
            }
            for secret in &session_plan.secrets {
                match std::fs::read(&secret.path) {Ok(value)=>{context.import_secret(&secret.origin,&secret.label,value);},Err(error)=>errors.push(format!("secret provisioning failed: {error}"))}
            }
            let secret_labels=context.secret_labels();
            let page=context.new_page();let page_id=page.id();let context_id=context.id();
            let mut session=NativeContextOilSession::new(context.clone());session.set_workflow_audit(audit);
            let mut steps=Vec::new();let mut recovery=Vec::new();let mut references=BTreeMap::new();let mut fault_evidence=Value::Null;let mut cancelled=false;let mut gate_consumed=false;let mut resume_after=None;
            // This barrier coordinates starts; it is never itself counted as native activity.
            barrier.wait().await;
            for (index,step) in session_plan.steps.iter().enumerate() {
                if !errors.is_empty() {break;}
                if resume_after.is_some_and(|after|index<=after) {continue;}
                let fault=session_plan.fault.as_ref().filter(|fault|fault.before_step==index || (fault.commit_gate.is_some() && !gate_consumed && index>=fault.before_step));
                if let Some(FaultPlan{kind:FaultKind::WorkerFailure,commit_gate:None,..})=fault {
                    #[cfg(unix)]
                    match context.terminate_qualification_worker(page.page_id()).await {
                        Ok(evidence)=>{fault_evidence=json!({"fault":"worker_failure","injected":true,"boundary":"before_step_idle_worker","native":evidence});
                            let replacement=context.new_page();
                            if let Err(error)=context.activate_page(replacement.page_id()) {errors.push(error.to_string());break;}
                        },
                        Err(error)=>{errors.push(error.to_string());break;}
                    }
                    #[cfg(not(unix))]
                    {errors.push("worker termination requires Unix process evidence".into());break;}
                }
                // Delayed HTTP responses are armed through the fixture's private
                // control pipe. Only the fixture's persisted release event proves injection.
                if let Some(FaultPlan{kind:FaultKind::DelayedResponse,..})=fault {
                    fault_evidence=json!({"fault":"delayed_response","injected":false,"requires":"sealed fixture response-withholding record"});
                }
                let timeout_ms=match fault {Some(FaultPlan{kind:FaultKind::Cancellation,delay_ms,commit_gate:None,..})=>*delay_ms,_=>step.timeout_ms};
                let input=match step_input(step,&references) {Ok(input)=>input,Err(error)=>{errors.push(error);break;}};
                let operation_started=std::time::Instant::now();
                let gate=fault.and_then(|fault|fault.commit_gate.as_ref());
                let mut gate_observed=false;
                let result=if let Some(gate)=gate {
                    tokio::select! { biased;
                        result=tokio::time::timeout(Duration::from_millis(timeout_ms),session.execute(&input))=>result,
                        marker=wait_commit_gate(gate)=>{
                            match marker {Ok(())=>{gate_observed=true;gate_consumed=true;},Err(error)=>errors.push(error)}
                            // Construct the same timeout outcome without polling
                            // another browser operation. The native cancellation
                            // below must still prove interrupted_inflight.
                            tokio::time::timeout(Duration::ZERO,std::future::pending()).await
                        }
                    }
                } else {tokio::time::timeout(Duration::from_millis(timeout_ms),session.execute(&input)).await};
                let (outputs,error,timed_out,native_error)=match result {
                    Ok(Ok(outputs))=>(serde_json::to_value(outputs).unwrap_or(Value::Null),None,false,Value::Null),
                    Ok(Err(error))=>{
                        let evidence=oil_failure_evidence(error);
                        if let Some(failure)=evidence["evidence_error"].as_str(){errors.push(failure.to_owned());}
                        (evidence["outputs"].clone(),evidence["error"].as_str().map(str::to_owned),false,evidence["native_error"].clone())
                    },
                    Err(_)=>(json!([]),Some("native operation deadline exceeded".into()),true,Value::Null),
                };
                if let Some(capture)=&step.capture_reference {
                    let matches:Vec<_>=outputs.as_array().into_iter().flatten()
                        .filter(|output|output["kind"]=="observation")
                        .flat_map(|output|output["observation"]["nodes"].as_array().into_iter().flatten())
                        .filter(|node|node["name"].as_str()==Some(capture.name.as_str()))
                        .filter_map(|node|node.get("semantic_ref").cloned()).collect();
                    if matches.len()!=1 || capture.slot.is_empty() {errors.push("capture requires exactly one real observed native reference".into());}
                    else {references.insert(capture.slot.clone(),matches[0].clone());}
                }
                let cancellation=gate_observed || (gate.is_none() && matches!(fault,Some(FaultPlan{kind:FaultKind::Cancellation,..}))&&timed_out);
                if cancellation {cancelled=true;fault_evidence=json!({"fault":"cancellation","injected":false,"operation_dropped":true,"requires":"worker retirement completion"});}
                let matched=if gate_observed {true} else {match &step.expected_error {Some(expected)=>error.as_ref().is_some_and(|actual|actual.contains(expected)),None=>error.is_none()}};
                if !matched {errors.push(format!("step {index}: unexpected result: {error:?}"));}
                steps.push(json!({"index":index,"outputs":outputs,"error":error,"native_error":native_error,"timed_out":timed_out,"expected_error_matched":matched,"operation_elapsed_ns":operation_started.elapsed().as_nanos()}));
                if cancellation {
                    #[cfg(unix)]
                    match context.cancel_qualification_operation(page.page_id()).await {
                        Ok(native)=>{let interrupted=native["interrupted_inflight"]==true;
                            fault_evidence=json!({"fault":fault.map(|fault|&fault.kind),"injected":interrupted,"native":native,"commit_gate":gate});
                            if let Some(gate)=gate.filter(|_|gate_observed && interrupted && native["reaped"]==true)
                                && let Err(error)=release_commit_gate(gate).await {errors.push(error);fault_evidence["injected"]=json!(false);}
                            if !interrupted {errors.push("cancellation did not interrupt an inflight native operation".into());}
                        },
                        Err(error)=>errors.push(error.to_string()),
                    }
                    #[cfg(not(unix))]
                    errors.push("native cancellation requires Unix exit evidence".into());
                    if gate_observed {
                        if errors.is_empty() {
                            if let Some(policy)=&session_plan.commit_recovery {
                                let replacement=context.new_page();
                                if let Err(error)=context.activate_page(replacement.page_id()) {errors.push(error.to_string());}
                                let inspect=[format!("goto \"{}\"",policy.outcome_url),"extract text css(#commit-state)".to_owned()];
                                let mut state=None;
                                for input in inspect {
                                    match tokio::time::timeout(Duration::from_secs(30),session.execute(&input)).await {
                                        Ok(Ok(outputs))=>{
                                            let values=serde_json::to_value(&outputs).unwrap_or(Value::Null);
                                            for output in values.as_array().into_iter().flatten() {
                                                if output["kind"]=="structured" {state=output["value"].as_str().map(str::to_owned);}
                                            }
                                            recovery.push(json!({"stage":"inspect_commit","outputs":outputs,"error":null}));
                                        },
                                        result=>{errors.push(format!("commit inspection failed: {result:?}"));break;},
                                    }
                                }
                                let command=match state.as_deref() {
                                    Some("not_committed")=>Some("click \"Retry pending request\""),
                                    Some("committed")=>Some("click \"Continue committed request\""),
                                    _=>{errors.push("durable outcome did not establish commit state".into());None},
                                };
                                if errors.is_empty()
                                    && let Some(command)=command {
                                        match tokio::time::timeout(Duration::from_secs(30),session.execute(command)).await {
                                            Ok(Ok(outputs))=>recovery.push(json!({"stage":"resolve_commit","state":state,"outputs":outputs,"error":null})),
                                            result=>errors.push(format!("commit recovery action failed: {result:?}")),
                                        }
                                    }
                                resume_after=Some(policy.resume_after);
                            } else {errors.push("commit interruption has no conditional recovery policy".into());}
                        }
                        continue;
                    }
                    if !session_plan.recovery_steps.is_empty() {
                        let replacement=context.new_page();
                        if let Err(error)=context.activate_page(replacement.page_id()) {errors.push(error.to_string());}
                        for (recovery_index,recovery_step) in session_plan.recovery_steps.iter().enumerate() {
                            let result=tokio::time::timeout(Duration::from_millis(recovery_step.timeout_ms),session.execute(&recovery_step.oil)).await;
                            match result {
                                Ok(Ok(outputs))=>recovery.push(json!({"index":recovery_index,"outputs":outputs,"error":null})),
                                Ok(Err(error))=>{errors.push(format!("recovery failed: {error}"));let mut evidence=oil_failure_evidence(error);evidence["index"]=json!(recovery_index);recovery.push(evidence);break;},
                                Err(_)=>{errors.push("recovery operation deadline".into());break;},
                            }
                        }
                    }
                    break;
                }
                if matches!(fault,Some(FaultPlan{kind:FaultKind::DuplicateDelivery,..})) {
                    let replay=tokio::time::timeout(Duration::from_millis(step.timeout_ms),session.execute(&input)).await;
                    let replay=match replay {Ok(Ok(outputs))=>json!({"outputs":outputs,"error":null}),Ok(Err(error))=>oil_failure_evidence(error),Err(_)=>json!({"outputs":[],"error":"duplicate delivery timeout"})};
                    let delivered=replay["error"].is_null() && replay["outputs"].as_array().is_some_and(|outputs|outputs.iter().any(|output|output["kind"]=="action"));
                    fault_evidence=json!({"fault":"duplicate_delivery","injected":delivered,"mechanism":"actual OIL replay","original_step":index,"replay":replay});
                }
            }
            let mut cleanup_errors=Vec::new();
            for descriptor in context.pages() {
                if matches!(descriptor.status,oryn_native::PageStatus::Closed|oryn_native::PageStatus::Crashed) {continue;}
                match tokio::time::timeout(Duration::from_secs(15),context.close_page(descriptor.page.page_id)).await {
                    Ok(Ok(_))=>{},Ok(Err(error))=>cleanup_errors.push(error.to_string()),Err(_)=>cleanup_errors.push("page close deadline".into()),
                }
            }
            let retired=match tokio::time::timeout(Duration::from_secs(15),context.retire_workers()).await {
                Ok(Ok(()))=>true,Ok(Err(error))=>{cleanup_errors.push(error.to_string());false},Err(_)=>{cleanup_errors.push("worker retirement deadline".into());false},
            };
            if cancelled {fault_evidence["workers_retired"]=json!(retired);if !retired {fault_evidence["injected"]=json!(false);}}
            let resources=context.resource_audit_snapshot();
            json!({"id":session_plan.id,"adversarial_for":session_plan.adversarial_for,"context_id":context_id,"page_id":page_id,"steps":steps,"recovery":recovery,
                "fault_evidence":fault_evidence,"errors":errors,"cleanup_errors":cleanup_errors,
                "workers_retired":retired,"secret_labels":secret_labels,"resource_audit":resources})
        });
    }
    let mut sessions = Vec::new();
    let mut infrastructure = Vec::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(result) => sessions.push(result),
            Err(error) => infrastructure.push(error.to_string()),
        }
    }
    drop(browser);
    let evidence = json!({"schema_version":1,"attempt_id":plan.attempt_id,"parent_pid":std::process::id(),
        "runtime":runtime,"worker_lifecycle":lifecycle_audit.snapshot(),"sessions":sessions,"native_activity":audit.snapshot(),"infrastructure_errors":infrastructure,
        "family_qualified":false});
    use std::io::Write;
    serde_json::to_writer_pretty(&mut output, &evidence)?;
    output.write_all(b"\n")?;
    output.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plan() -> Plan {
        serde_json::from_value(json!({"schema_version":1,"attempt_id":"one","sessions":[{"id":"a","steps":[{"oil":"observe"}],"fault":null}]})).unwrap()
    }
    #[test]
    fn duplicate_sessions_and_unsupported_context_counts_are_rejected() {
        let mut p = plan();
        p.sessions.push(plan().sessions.remove(0));
        assert!(validate(&p).is_err());
    }
    #[test]
    fn expected_errors_must_be_explicit_and_steps_cannot_hide_extra_commands() {
        let mut p = plan();
        p.sessions[0].steps[0].oil = "observe\nclick 1".into();
        assert!(validate(&p).is_err());
        p.sessions[0].steps[0].oil = "observe".into();
        p.sessions[0].steps[0].expected_error = Some(String::new());
        assert!(validate(&p).is_err());
    }
    #[test]
    fn fault_plan_requires_an_existing_step() {
        let mut p = plan();
        p.sessions[0].fault = Some(FaultPlan {
            kind: FaultKind::WorkerFailure,
            before_step: 1,
            delay_ms: 1,
            commit_gate: None,
        });
        assert!(validate(&p).is_err());
    }
    #[test]
    fn captured_reference_must_come_from_a_real_previous_observation() {
        let step: Step =
            serde_json::from_value(json!({"oil":"click saved","use_reference":"old"})).unwrap();
        assert!(step_input(&step, &BTreeMap::new()).is_err());
        let references = BTreeMap::from([(
            "old".into(),
            json!({"page_id":7,"document_generation":2,"node":3}),
        )]);
        let input = step_input(&step, &references).unwrap();
        let request: oryn_common::agent::IntentRequest = serde_json::from_str(
            input
                .strip_prefix(oryn_native::oil::TYPED_INTENT_PREFIX)
                .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            request.intent,
            oryn_common::agent::Intent::Click {
                target: oryn_common::agent::Target::Reference { .. },
                ..
            }
        ));
    }
    #[test]
    fn adversarial_context_requires_explicit_cohort_owner() {
        let mut p = plan();
        let mut probe = plan().sessions.remove(0);
        probe.id = "probe".into();
        probe.adversarial_for = Some("a".into());
        p.sessions.push(probe);
        assert!(validate(&p).is_ok());
        p.sessions[1].adversarial_for = Some("foreign".into());
        assert!(validate(&p).is_err());
    }

    #[test]
    fn commit_recovery_requires_bound_fault_and_native_route() {
        let mut p = plan();
        p.sessions[0].commit_recovery = Some(CommitRecovery {
            outcome_url: "http://127.0.0.1:1234/outcome".into(),
            resume_after: 0,
        });
        assert!(validate(&p).is_err());
    }
    #[tokio::test]
    async fn commit_marker_rejects_foreign_token_before_interrupting() {
        let root = std::env::temp_dir().join(format!("oryn-commit-marker-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let marker = root.join("marker");
        std::fs::write(
            &marker,
            br#"{"schema_version":1,"token":"foreign","phase":"before_commit","started_ns":1}"#,
        )
        .unwrap();
        let gate = oryn_native::workflow_instrumentation::CommitGate {
            token: "owner".into(),
            phase: "before_commit".into(),
            marker: marker.clone(),
            release: root.join("release"),
        };
        assert!(wait_commit_gate(&gate).await.is_err());
        std::fs::remove_file(marker).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn native_failure_evidence_retains_partial_receipts_and_action_identity() {
        let error = oryn_native::oil::NativeOilError::ScriptExecutionFailed {
            completed: vec![oryn_native::oil::NativeOilOutput::Text {
                value: "done".into(),
            }],
            failed_index: 1,
            cause: Box::new(oryn_native::oil::NativeOilError::Runtime(
                oryn_native::RuntimeError::ActionExecutionFailed {
                    action_id: oryn_common::v3::ActionId(42),
                    cause: Box::new(oryn_native::RuntimeError::Closed),
                },
            )),
        };
        let evidence = oil_failure_evidence(error);
        assert_eq!(evidence["outputs"][0]["value"], "done");
        assert_eq!(
            evidence["native_error"]["OilExecutionFailed"]["failed_index"],
            1
        );
        assert_eq!(
            evidence["native_error"]["OilExecutionFailed"]["cause"]["ActionExecutionFailed"]["action_id"],
            42
        );
    }
}
