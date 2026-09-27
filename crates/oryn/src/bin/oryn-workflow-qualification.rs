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
    response_gate: Option<ResponseGate>,
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
    #[serde(default)]
    bootstrap_url: Option<String>,
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
fn validate_commit_bootstrap(policy: &CommitRecovery, token: &str) -> Result<(), String> {
    let outcome = url::Url::parse(&policy.outcome_url).map_err(|error| error.to_string())?;
    if outcome.scheme() != "http"
        || outcome.host_str() != Some("127.0.0.1")
        || !outcome.username().is_empty()
        || outcome.password().is_some()
        || outcome.fragment().is_some()
        || !outcome.path().ends_with("/workflow-outcome")
        || outcome
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<Vec<_>>()
            != vec![("token".to_owned(), token.to_owned())]
    {
        return Err(
            "commit outcome must be the exact loopback route bound to the retained gate token"
                .into(),
        );
    }
    let Some(bootstrap) = &policy.bootstrap_url else {
        return Ok(());
    };
    let bootstrap = url::Url::parse(bootstrap).map_err(|error| error.to_string())?;
    let segments: Vec<_> = outcome
        .path_segments()
        .ok_or("missing outcome path")?
        .collect();
    if segments.len() != 3
        || segments[0] != "saas"
        || segments[1].is_empty()
        || segments[2] != "workflow-outcome"
        || bootstrap.origin() != outcome.origin()
        || !bootstrap.username().is_empty()
        || bootstrap.password().is_some()
        || bootstrap.query().is_some()
        || bootstrap.fragment().is_some()
        || bootstrap.path() != format!("/saas/{}/resume", segments[1])
        || policy
            .bootstrap_url
            .as_ref()
            .is_some_and(|value| value.chars().any(|c| matches!(c, '\n' | '\r' | '"')))
    {
        return Err(
            "commit bootstrap must be the same-origin same-case public SaaS resume route".into(),
        );
    }
    Ok(())
}

const COMMIT_STATE_EXTRACT: &str = "extract text --selector \"#commit-state\"";

#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct ResponseGate {
    token: String,
    marker: PathBuf,
    route: String,
}
impl ResponseGate {
    fn validate(&self) -> Result<(), String> {
        if self.token.is_empty()
            || self.token.len() > 128
            || !self.marker.is_absolute()
            || self.marker.exists()
            || self.route.is_empty()
            || self.route.len() > 2048
            || self
                .route
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
            || !(self.route.starts_with('/')
                || self.route.split('/').all(|segment| {
                    !segment.is_empty()
                        && segment
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                }))
        {
            return Err(
                "response gate requires a token, fresh absolute marker and exact route".into(),
            );
        }
        Ok(())
    }
}

enum ResponseGateOutcome<T> {
    Completed(T),
    Entered(Value),
    Deadline,
    Invalid(String),
}

async fn wait_response_gate(
    gate: &ResponseGate,
    deadline: tokio::time::Instant,
) -> Result<Value, String> {
    use std::io::Read;
    loop {
        // Check the original operation deadline before accepting any witness.
        if tokio::time::Instant::now() >= deadline {
            return Err("response marker deadline".into());
        }
        match std::fs::File::open(&gate.marker) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(4097)
                    .read_to_end(&mut bytes)
                    .map_err(|error| error.to_string())?;
                if bytes.len() > 4096 {
                    return Err("oversized response marker".into());
                }
                let value: Value =
                    serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
                if value["schema_version"] != 1
                    || value["token"].as_str() != Some(gate.token.as_str())
                    || value["phase"].as_str() != Some("response_entered")
                    || value["route"].as_str() != Some(gate.route.as_str())
                    || !value["started_ns"].as_u64().is_some_and(|value| value > 0)
                {
                    return Err("response marker identity mismatch".into());
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err("response marker deadline".into());
                }
                return Ok(value);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
        tokio::time::sleep_until(
            deadline.min(tokio::time::Instant::now() + Duration::from_millis(5)),
        )
        .await;
    }
}

async fn execute_until_response_entered<F: std::future::Future>(
    operation: F,
    gate: &ResponseGate,
    deadline: tokio::time::Instant,
) -> ResponseGateOutcome<F::Output> {
    tokio::select! { biased;
        // Completion/error wins when both branches are ready. Never label a
        // completed action an interrupted operation and never replay it.
        result = tokio::time::timeout_at(deadline, operation) => match result {
            Ok(result) => ResponseGateOutcome::Completed(result),
            Err(_) => ResponseGateOutcome::Deadline,
        },
        marker = wait_response_gate(gate, deadline) => match marker {
            Ok(marker) => ResponseGateOutcome::Entered(marker),
            Err(error) => ResponseGateOutcome::Invalid(error),
        }
    }
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
        let response_cancellation = session.fault.as_ref().is_some_and(|fault| {
            matches!(fault.kind, FaultKind::Cancellation) && fault.commit_gate.is_none()
        });
        if response_cancellation != session.response_gate.is_some() {
            return Err("ordinary cancellation requires exactly one response-entry gate".into());
        }
        if let Some(gate) = &session.response_gate {
            gate.validate()?;
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
        if let Some(policy) = &session.commit_recovery {
            let gate = session
                .fault
                .as_ref()
                .and_then(|fault| fault.commit_gate.as_ref())
                .ok_or("missing retained commit gate")?;
            validate_commit_bootstrap(policy, &gate.token)?;
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
                let timeout_ms=step.timeout_ms;
                let input=match step_input(step,&references) {Ok(input)=>input,Err(error)=>{errors.push(error);break;}};
                let operation_started=std::time::Instant::now();
                let gate=fault.and_then(|fault|fault.commit_gate.as_ref());
                let mut gate_observed=false;
                let response_gate=fault.filter(|fault|matches!(fault.kind,FaultKind::Cancellation) && fault.commit_gate.is_none()).and(session_plan.response_gate.as_ref());
                let mut response_marker=None;
                let result=if let Some(response_gate)=response_gate {
                    let deadline=tokio::time::Instant::now()+Duration::from_millis(timeout_ms);
                    match execute_until_response_entered(session.execute(&input),response_gate,deadline).await {
                        ResponseGateOutcome::Completed(result)=>{
                            errors.push("cancellation response was not entered before operation completed".into());
                            Ok(result)
                        },
                        outcome=>{
                            match outcome {
                                ResponseGateOutcome::Entered(marker)=>response_marker=Some(marker),
                                ResponseGateOutcome::Deadline=>errors.push("response marker not observed within original operation deadline".into()),
                                ResponseGateOutcome::Invalid(error)=>errors.push(error),
                                ResponseGateOutcome::Completed(_)=>unreachable!(),
                            }
                            tokio::time::timeout(Duration::ZERO,std::future::pending()).await
                        }
                    }
                } else if let Some(gate)=gate {
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
                let cancellation=gate_observed || response_marker.is_some();
                if cancellation {cancelled=true;fault_evidence=json!({"fault":"cancellation","injected":false,"operation_dropped":true,"requires":"worker retirement completion"});}
                let matched=if gate_observed {true} else {match &step.expected_error {Some(expected)=>error.as_ref().is_some_and(|actual|actual.contains(expected)),None=>error.is_none()}};
                if !matched {errors.push(format!("step {index}: unexpected result: {error:?}"));}
                steps.push(json!({"index":index,"outputs":outputs,"error":error,"native_error":native_error,"timed_out":timed_out,"expected_error_matched":matched,"operation_elapsed_ns":operation_started.elapsed().as_nanos()}));
                if cancellation {
                    #[cfg(unix)]
                    match context.cancel_qualification_operation(page.page_id()).await {
                        Ok(native)=>{let interrupted=native["interrupted_inflight"]==true;
                            fault_evidence=json!({"fault":fault.map(|fault|&fault.kind),"injected":interrupted,"native":native,"commit_gate":gate,"response_gate":response_gate,"response_entered":response_marker.is_some(),"response_marker":response_marker});
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
                                // Preserve the previous two30s inspection budget overall,
                                // including the optional side-effect-free real navigation.
                                let inspect_deadline=tokio::time::Instant::now()+Duration::from_secs(60);
                                if errors.is_empty() && let Some(bootstrap)=&policy.bootstrap_url {
                                    let deadline=inspect_deadline.min(tokio::time::Instant::now()+Duration::from_secs(30));
                                    match tokio::time::timeout_at(deadline,session.execute(&format!("goto \"{}\"",bootstrap))).await {
                                        Ok(Ok(outputs))=>recovery.push(json!({"stage":"bootstrap_commit","outputs":outputs,"error":null})),
                                        result=>errors.push(format!("commit bootstrap failed: {result:?}")),
                                    }
                                }
                                let inspect=[format!("goto \"{}\"",policy.outcome_url),COMMIT_STATE_EXTRACT.to_owned()];
                                let mut state=None;
                                for input in inspect {
                                    if !errors.is_empty() {break;}
                                    let deadline=inspect_deadline.min(tokio::time::Instant::now()+Duration::from_secs(30));
                                    match tokio::time::timeout_at(deadline,session.execute(&input)).await {
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
                    if errors.is_empty() && !session_plan.recovery_steps.is_empty() {
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
            bootstrap_url: None,
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
    fn response_gate_fixture(label: &str) -> (PathBuf, ResponseGate) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let root = std::env::temp_dir().join(format!(
            "oryn-response-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let gate = ResponseGate {
            token: "owner".into(),
            marker: root.join("marker"),
            route: "/held-response".into(),
        };
        (root, gate)
    }
    fn publish_response(gate: &ResponseGate) {
        std::fs::write(&gate.marker, serde_json::to_vec(&json!({"schema_version":1,"token":gate.token,"phase":"response_entered","started_ns":1,"route":gate.route})).unwrap()).unwrap();
    }
    struct CountedOperation(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl Drop for CountedOperation {
        fn drop(&mut self) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    #[test]
    fn response_gate_plan_requires_cancellation_and_fresh_bound_route() {
        let (root, gate) = response_gate_fixture("plan");
        let mut p = plan();
        p.sessions[0].fault = Some(FaultPlan {
            kind: FaultKind::Cancellation,
            before_step: 0,
            delay_ms: 50,
            commit_gate: None,
        });
        assert!(validate(&p).is_err());
        p.sessions[0].response_gate = Some(gate);
        assert!(validate(&p).is_ok());
        p.sessions[0].fault.as_mut().unwrap().kind = FaultKind::WorkerFailure;
        assert!(validate(&p).is_err());
        p.sessions[0].fault.as_mut().unwrap().kind = FaultKind::Cancellation;
        publish_response(p.sessions[0].response_gate.as_ref().unwrap());
        assert!(validate(&p).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn response_gate_waits_past_old_delay_then_drops_original_once() {
        let (root, gate) = response_gate_fixture("slow");
        let drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (entered, receiving) = tokio::sync::oneshot::channel();
        let operation = async {
            let _guard = CountedOperation(drops.clone());
            entered.send(()).unwrap();
            std::future::pending::<()>().await;
        };
        let fixture = async {
            receiving.await.unwrap();
            tokio::time::sleep(Duration::from_millis(75)).await;
            assert_eq!(drops.load(std::sync::atomic::Ordering::SeqCst), 0);
            publish_response(&gate);
        };
        let (result, ()) = tokio::join!(
            execute_until_response_entered(
                operation,
                &gate,
                tokio::time::Instant::now() + Duration::from_secs(2)
            ),
            fixture
        );
        assert!(matches!(result, ResponseGateOutcome::Entered(_)));
        assert_eq!(drops.load(std::sync::atomic::Ordering::SeqCst), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn response_gate_completion_and_error_win_without_replay() {
        let (root, gate) = response_gate_fixture("complete");
        publish_response(&gate);
        for result in [Ok(7), Err("original failure")] {
            let got = execute_until_response_entered(
                std::future::ready(result),
                &gate,
                tokio::time::Instant::now() + Duration::from_secs(1),
            )
            .await;
            assert!(matches!(got, ResponseGateOutcome::Completed(value) if value==result));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn response_gate_missing_marker_obeys_original_deadline() {
        let (root, gate) = response_gate_fixture("deadline");
        let deadline = tokio::time::Instant::now() + Duration::from_millis(10);
        let result =
            execute_until_response_entered(std::future::pending::<()>(), &gate, deadline).await;
        assert!(matches!(
            result,
            ResponseGateOutcome::Deadline | ResponseGateOutcome::Invalid(_)
        ));
        publish_response(&gate);
        assert!(wait_response_gate(&gate, deadline).await.is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn response_gate_rejects_foreign_route_token_phase_and_oversize() {
        let (root, gate) = response_gate_fixture("identity");
        for (key, wrong) in [
            ("token", json!("foreign")),
            ("route", json!("/other")),
            ("phase", json!("released")),
            ("started_ns", json!(0)),
        ] {
            let mut marker = json!({"schema_version":1,"token":gate.token,"phase":"response_entered","started_ns":1,"route":gate.route});
            marker[key] = wrong;
            std::fs::write(&gate.marker, serde_json::to_vec(&marker).unwrap()).unwrap();
            assert!(
                wait_response_gate(&gate, tokio::time::Instant::now() + Duration::from_secs(1))
                    .await
                    .is_err()
            );
        }
        std::fs::write(&gate.marker, vec![b' '; 4097]).unwrap();
        assert!(
            wait_response_gate(&gate, tokio::time::Instant::now() + Duration::from_secs(1))
                .await
                .is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn response_gate_outer_cancellation_drops_pending_operation_once() {
        let (root, gate) = response_gate_fixture("cancel");
        let drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (entered, receiving) = tokio::sync::oneshot::channel();
        {
            let operation = async {
                let _guard = CountedOperation(drops.clone());
                entered.send(()).unwrap();
                std::future::pending::<()>().await
            };
            tokio::select! { biased;
                _ = execute_until_response_entered(operation,&gate,tokio::time::Instant::now()+Duration::from_secs(1)) => panic!("gate settled before cancellation"),
                _ = receiving => {},
            }
        }
        assert_eq!(drops.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!gate.marker.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn commit_state_extract_uses_real_native_grammar_and_selector() {
        let browser = Browser::try_with_config(BrowserConfig::default()).unwrap();
        let context = browser.new_context_with_options(ContextOptions::loopback_test());
        let page = context.new_page();
        let loaded = page
            .load_html(
                "http://127.0.0.1/commit-grammar",
                "<p id=commit-state>committed</p><p>not_committed</p>",
            )
            .await;
        let mut oil = oryn_native::oil::NativeOilSession::new(page.clone());
        let invalid = oil.execute("extract text css(#commit-state)").await;
        let extracted = oil.execute(COMMIT_STATE_EXTRACT).await;
        let close = context.close_page(page.page_id()).await;
        let retired = context.retire_workers().await;
        loaded.unwrap();
        close.unwrap();
        retired.unwrap();
        assert!(matches!(
            invalid,
            Err(oryn_native::oil::NativeOilError::Parse(_))
        ));
        let outputs = extracted.unwrap();
        assert!(
            matches!(outputs.as_slice(),[oryn_native::oil::NativeOilOutput::Structured {value}] if value.as_str().is_some_and(|text|text.trim()=="committed"))
        );
    }

    #[test]
    fn commit_bootstrap_is_same_origin_same_case_and_has_no_authority_parameters() {
        let mut policy = CommitRecovery {
            outcome_url: "http://127.0.0.1:1234/saas/owned/workflow-outcome?token=bound".into(),
            bootstrap_url: Some("http://127.0.0.1:1234/saas/owned/resume".into()),
            resume_after: 0,
        };
        assert!(validate_commit_bootstrap(&policy, "bound").is_ok());
        for forbidden in [
            "http://127.0.0.1:1234@foreign.test/saas/owned/workflow-outcome?token=bound",
            "http://127.0.0.1:1234/saas/owned/workflow-outcome?token=foreign",
            "http://127.0.0.1:1234/saas/owned/workflow-outcome?token=bound&token=bound",
            "http://127.0.0.1:1234/saas/owned/workflow-outcome?token=bound#fragment",
        ] {
            let invalid = CommitRecovery {
                outcome_url: forbidden.into(),
                bootstrap_url: None,
                resume_after: 0,
            };
            assert!(
                validate_commit_bootstrap(&invalid, "bound").is_err(),
                "{forbidden}"
            );
        }
        for forbidden in [
            "http://127.0.0.1:1235/saas/owned/resume",
            "http://127.0.0.1:1234/saas/foreign/resume",
            "http://127.0.0.1:1234/saas/owned/resume?token=bound",
            "http://user@127.0.0.1:1234/saas/owned/resume",
            "http://127.0.0.1:1234/saas/owned/resume#fragment",
        ] {
            policy.bootstrap_url = Some(forbidden.into());
            assert!(
                validate_commit_bootstrap(&policy, "bound").is_err(),
                "{forbidden}"
            );
        }
    }
    #[cfg(unix)]
    struct StrictCookieFixture {
        base: String,
        stop: Arc<std::sync::atomic::AtomicBool>,
        requests: Arc<std::sync::Mutex<Vec<(String, bool, bool)>>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    #[cfg(unix)]
    impl StrictCookieFixture {
        fn start() -> Self {
            use std::io::{Read, Write};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
            let stopped = stop.clone();
            let recorded = requests.clone();
            let thread = std::thread::spawn(move || {
                while !stopped.load(std::sync::atomic::Ordering::SeqCst) {
                    let (mut stream, _) = match listener.accept() {
                        Ok(value) => value,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        Err(_) => break,
                    };
                    if stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .is_err()
                        || stream
                            .set_write_timeout(Some(Duration::from_secs(1)))
                            .is_err()
                    {
                        continue;
                    }
                    let mut bytes = Vec::new();
                    let mut buffer = [0; 1024];
                    let read_deadline = std::time::Instant::now() + Duration::from_secs(1);
                    while bytes.len() < 16384
                        && !bytes.windows(4).any(|part| part == b"\r\n\r\n")
                        && std::time::Instant::now() < read_deadline
                        && !stopped.load(std::sync::atomic::Ordering::SeqCst)
                    {
                        match stream.read(&mut buffer) {
                            Ok(0) | Err(_) => break,
                            Ok(size) => bytes.extend_from_slice(&buffer[..size]),
                        }
                    }
                    let request = String::from_utf8_lossy(&bytes);
                    let mut first = request.lines().next().unwrap_or("").split_whitespace();
                    let method = first.next().unwrap_or("");
                    let path = first.next().unwrap_or("");
                    let cookie = request.lines().any(|line| {
                        line.split_once(':').is_some_and(|(name, value)| {
                            name.eq_ignore_ascii_case("cookie")
                                && value
                                    .split(';')
                                    .any(|part| part.trim() == "session=retained")
                        })
                    });
                    let seed = path == "/saas/owned/seed" && method == "GET";
                    recorded
                        .lock()
                        .unwrap()
                        .push((format!("{method} {path}"), cookie, seed));
                    let (status, body) = match (method, path) {
                        ("GET", "/saas/owned/seed") => ("200 OK", "<p>seeded</p>"),
                        ("GET", "/saas/owned/resume") => ("200 OK", "<p>public resume</p>"),
                        ("GET", "/saas/owned/workflow-outcome?token=bound") if cookie => {
                            ("200 OK", "<p id=commit-state>committed</p>")
                        }
                        ("GET", "/saas/owned/workflow-outcome?token=bound") => {
                            ("403 Forbidden", "<p>unauthorized</p>")
                        }
                        _ => ("404 Forbidden", "<p>not found</p>"),
                    };
                    let set_cookie = if seed {
                        "Set-Cookie: session=retained; Path=/; HttpOnly; SameSite=Strict\r\n"
                    } else {
                        ""
                    };
                    let reply = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: text/html\r\n{set_cookie}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(reply.as_bytes());
                }
            });
            Self {
                base,
                stop,
                requests,
                thread: Some(thread),
            }
        }
    }
    #[cfg(unix)]
    impl Drop for StrictCookieFixture {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn commit_bootstrap_restores_strict_cookie_on_real_fresh_native_page() {
        let fixture = StrictCookieFixture::start();
        let browser = Browser::try_with_config(BrowserConfig::default()).unwrap();
        let context = browser.new_context_with_options(ContextOptions::loopback_test());
        let seed = context.new_page();
        let blank = context.new_page();
        let resumed = context.new_page();
        let outcome = format!("{}/saas/owned/workflow-outcome?token=bound", fixture.base);
        let bootstrap = format!("{}/saas/owned/resume", fixture.base);
        let run = async {
            let mut seed_oil = oryn_native::oil::NativeOilSession::new(seed.clone());
            seed_oil
                .execute(&format!("goto \"{}/saas/owned/seed\"", fixture.base))
                .await
                .map_err(|error| error.to_string())?;
            // This independent blank page must not acquire an authenticated response.
            let mut blank_oil = oryn_native::oil::NativeOilSession::new(blank.clone());
            let _ = blank_oil.execute(&format!("goto \"{outcome}\"")).await;
            // A different fresh page follows exactly the production GET bootstrap sequence.
            let mut resumed_oil = oryn_native::oil::NativeOilSession::new(resumed.clone());
            resumed_oil
                .execute(&format!("goto \"{bootstrap}\""))
                .await
                .map_err(|error| error.to_string())?;
            resumed_oil
                .execute(&format!("goto \"{outcome}\""))
                .await
                .map_err(|error| error.to_string())?;
            resumed_oil
                .execute(COMMIT_STATE_EXTRACT)
                .await
                .map_err(|error| error.to_string())
        };
        let result = tokio::time::timeout(Duration::from_secs(30), run).await;
        let mut cleanup = Vec::new();
        for page in [&seed, &blank, &resumed] {
            cleanup.push(
                tokio::time::timeout(Duration::from_secs(15), context.close_page(page.page_id()))
                    .await,
            );
        }
        let retired = tokio::time::timeout(Duration::from_secs(15), context.retire_workers()).await;
        let requests = fixture.requests.lock().unwrap().clone();
        drop(fixture);
        for result in cleanup {
            result.unwrap().unwrap();
        }
        retired.unwrap().unwrap();
        let outputs = result.unwrap().unwrap();
        assert!(
            matches!(outputs.as_slice(),[oryn_native::oil::NativeOilOutput::Structured {value}] if value.as_str()==Some("committed"))
        );
        let protected: Vec<_> = requests
            .iter()
            .filter(|(path, _, _)| path == "GET /saas/owned/workflow-outcome?token=bound")
            .collect();
        assert_eq!(protected.len(), 2);
        assert!(
            !protected[0].1,
            "Strict cookie must be withheld from the blank-page request"
        );
        assert!(
            protected[1].1,
            "real same-origin bootstrap must retain Strict cookie authority"
        );
        assert_eq!(
            requests.iter().filter(|(_, _, seed)| *seed).count(),
            1,
            "bootstrap cannot mint authentication"
        );
        assert_eq!(
            requests
                .iter()
                .filter(|(path, _, _)| path == "GET /saas/owned/resume")
                .count(),
            1
        );
        assert!(
            requests.iter().all(|(path, _, _)| path.starts_with("GET ")),
            "no mutation or commit replay"
        );
    }
    #[test]
    fn response_gate_accepts_opaque_fixture_identifiers_and_absolute_routes() {
        let (root, mut gate) = response_gate_fixture("route-contract");
        for route in [
            "login",
            "receipt",
            "order",
            "confirmation",
            "case_9-v2",
            "article/beta",
            "/article/beta",
            "/held-response",
            "/legacy/path?literal=1",
        ] {
            gate.route = route.into();
            assert!(gate.validate().is_ok(), "{route}");
            let encoded = serde_json::to_value(&gate).unwrap();
            assert_eq!(
                encoded["route"], route,
                "route identity must not be normalized"
            );
        }
        for route in [
            "",
            "two words",
            "login\n",
            "/route\r",
            "/bad\0",
            "\tlogin",
            "foo//bar",
            "foo/",
            "foo/../bar",
            "foo?bar",
            "https://example.test/route",
            "réceipt",
            "/nonbreaking\u{a0}space",
            "/delete\u{7f}",
        ] {
            gate.route = route.into();
            assert!(gate.validate().is_err(), "{route:?}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn response_gate_opaque_route_still_requires_exact_marker_binding() {
        let (root, mut gate) = response_gate_fixture("opaque-binding");
        gate.route = "receipt".into();
        gate.validate().unwrap();
        publish_response(&gate);
        let marker =
            wait_response_gate(&gate, tokio::time::Instant::now() + Duration::from_secs(1))
                .await
                .unwrap();
        assert_eq!(marker["route"], "receipt");
        for field in ["route", "token"] {
            let mut wrong = marker.clone();
            wrong[field] = json!("foreign");
            std::fs::write(&gate.marker, serde_json::to_vec(&wrong).unwrap()).unwrap();
            assert!(
                wait_response_gate(&gate, tokio::time::Instant::now() + Duration::from_secs(1))
                    .await
                    .is_err()
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn response_gate_deserializes_actual_fixture_exported_cancellation_plans() {
        let path = std::env::var_os("ORYN_RESPONSE_GATE_FIXTURE_PLANS").expect(
            "generate the reviewed fixture-plan exporter output before this targeted regression",
        );
        let encoded = std::fs::read(path).unwrap();
        let envelope: Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(envelope["schema_version"], 1);
        let plans = envelope["plans"].as_array().unwrap();
        let arms = envelope["arms"].as_array().unwrap();
        assert_eq!(
            plans.len(),
            10,
            "all ten failed cancellation contracts must be covered"
        );
        assert_eq!(arms.len(), plans.len());
        let mut identities = BTreeSet::new();
        for value in plans {
            let plan: Plan = serde_json::from_value(value.clone()).unwrap();
            assert!(
                identities.insert(plan.attempt_id.clone()),
                "duplicate exported original"
            );
            validate(&plan).unwrap_or_else(|error| panic!("{}: {error}", plan.attempt_id));
            assert_eq!(plan.sessions.len(), 1);
            let session = &plan.sessions[0];
            let fault = session.fault.as_ref().unwrap();
            assert!(matches!(fault.kind, FaultKind::Cancellation));
            assert!(fault.commit_gate.is_none());
            let gate = session.response_gate.as_ref().unwrap();
            let matching: Vec<_> = arms
                .iter()
                .filter(|arm| arm["attempt_id"].as_str() == Some(plan.attempt_id.as_str()))
                .collect();
            assert_eq!(
                matching.len(),
                1,
                "each actual fixture arm must have exactly one plan"
            );
            let request = &matching[0]["request"];
            assert_eq!(request["command"], "arm_response_fault");
            assert_eq!(
                request["response_gate"],
                serde_json::to_value(gate).unwrap()
            );
            assert_eq!(request["token"].as_str(), Some(gate.token.as_str()));
            assert_eq!(request["route"].as_str(), Some(gate.route.as_str()));
            assert!(
                !gate.marker.exists(),
                "exporter must not fabricate a response-entry witness"
            );
        }
    }
}
