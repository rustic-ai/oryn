//! Explicit diagnostic transport. Each execution owns a fresh browser context.
use oryn_native::{Browser, BrowserConfig, ContextOptions, WorkerLifecycle};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

const MAX_REQUEST_BYTES: usize = 1024 * 1024;

async fn read_frame<R: AsyncBufRead + Unpin>(input: &mut R) -> std::io::Result<Option<Vec<u8>>> {
    let mut frame = Vec::new();
    input
        .take((MAX_REQUEST_BYTES + 1) as u64)
        .read_until(b'\n', &mut frame)
        .await?;
    if frame.is_empty() {
        return Ok(None);
    }
    if frame.len() > MAX_REQUEST_BYTES || frame.last() != Some(&b'\n') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "oversized or truncated WPT frame",
        ));
    }
    Ok(Some(frame))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    command: String,
    id: String,
    url: Option<String>,
    timeout_ms: Option<u64>,
    #[serde(default)]
    uploads: Vec<String>,
}

pub async fn run(lifecycle: WorkerLifecycle) -> Result<(), Box<dyn std::error::Error>> {
    if matches!(lifecycle, WorkerLifecycle::Recycled { .. }) {
        return Err("WPT sessions prohibit recycled workers".into());
    }
    let browser = Browser::try_with_config(BrowserConfig {
        worker_lifecycle: lifecycle,
        ..Default::default()
    })?;
    let start = Instant::now();
    browser.prepare_workers()?;
    let output = Arc::new(tokio::sync::Mutex::new(tokio::io::stdout()));
    emit(
        &output,
        json!({"event":"ready","schema_version":1,"prepare_ms":start.elapsed().as_millis()}),
    )
    .await?;
    let permits = Arc::new(tokio::sync::Semaphore::new(8));
    let mut tasks = tokio::task::JoinSet::new();
    let mut ids = HashSet::new();
    let mut input = BufReader::new(tokio::io::stdin());
    while let Some(line) = read_frame(&mut input).await? {
        let request: Request = serde_json::from_slice(&line)?;
        if !ids.insert(request.id.clone()) {
            return Err("duplicate WPT execution ID".into());
        }
        if request.command == "shutdown" {
            break;
        }
        if request.command != "run" {
            return Err("unknown WPT command".into());
        }
        let url = request.url.ok_or("run requires url")?;
        let timeout = request.timeout_ms.unwrap_or(30_000);
        if !(1..=120_000).contains(&timeout) {
            return Err("invalid WPT deadline".into());
        }
        let queued = Instant::now();
        let permit = permits.clone().acquire_owned().await?;
        let browser = browser.clone();
        let output = output.clone();
        tasks.spawn(async move {
            let queue_ms = queued.elapsed().as_millis();
            let start = Instant::now();
            let mut options = ContextOptions::loopback_test();
            if !request.uploads.is_empty() {
                options.action_policy.rules.insert(
                    oryn_native::ConsequentialAction::FileUpload,
                    oryn_native::PolicyRule::Allow,
                );
            }
            let context = browser.new_audited_context_with_isolated_auxiliaries(options);
            let upload_error = super::import_uploads(&context, &request.uploads)
                .err()
                .map(|e| e.to_string());
            let page = context.new_page();
            let page_id = page.id();
            let result = tokio::time::timeout(Duration::from_millis(timeout), async {
                if let Some(error) = upload_error {
                    return Err(error);
                }
                page.goto(&url).await.map_err(|e| e.to_string())?;
                loop {
                    let raw = page
                        .evaluate("JSON.stringify(globalThis.__oryn_wpt_completion || null)")
                        .await
                        .map_err(|e| e.to_string())?;
                    let mut value: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
                    if !value.is_null() {
                        let raw_trace = page.evaluate("__oryn_native_automation_trace").await.map_err(|e| e.to_string())?;
                        let trace: Value = serde_json::from_str(&raw_trace).map_err(|e| e.to_string())?;
                        if trace["schema_version"] != 1 || trace["scope"] != "page_isolate" || !trace["events"].is_array() {
                            return Err("native automation trace malformed".into());
                        }
                        value["testdriver"] = trace["events"].clone();
                        value["automation_overflow"] = trace["overflow"].clone();
                        value["automation_trace"] = trace;
                        return Ok::<Value, String>(value);
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await;
            let run_ms = start.elapsed().as_millis();
            let trace =
                tokio::time::timeout(Duration::from_secs(5), context.trace_archive(Vec::new()))
                    .await;
            let (policy_events, trace_error) = match trace {
                Ok(Ok(archive)) => {
                    let mut resources = Vec::new();
                    for (page, events) in archive.page_traces {
                        for event in events {
                            let kind = serde_json::to_value(event.kind).unwrap_or(Value::Null);
                            if matches!(
                                kind["kind"].as_str(),
                                Some(
                                    "request_started"
                                        | "response_received"
                                        | "policy_denied"
                                        | "csp_violation"
                                )
                            ) {
                                resources.push(
                                    json!({"page_id":page,"sequence":event.sequence,"event":kind}),
                                );
                            }
                        }
                    }
                    (resources, None)
                }
                _ => (Vec::new(), Some("resource_trace_unavailable")),
            };
            let teardown = Instant::now();
            let mut memory_samples = Vec::new();
            #[cfg(unix)]
            if let Some(audit) = context.resource_audit_snapshot() {
                let pids: std::collections::BTreeSet<u32> = audit.records.iter().filter_map(|record| record.worker_pid).collect();
                for pid in pids {
                    memory_samples.push(json!({"pid":pid,"rss_bytes":oryn_native::worker_client::resident_bytes(pid).ok()}));
                }
            }
            let mut cleanup_errors = Vec::new();
            let cleanup = tokio::time::timeout(Duration::from_secs(15), async {
                for descriptor in context.pages() {
                    if matches!(descriptor.status, oryn_native::PageStatus::Closed) {
                        continue;
                    }
                    if matches!(descriptor.status, oryn_native::PageStatus::Crashed) {
                        cleanup_errors.push(format!("page {:?} crashed before teardown", descriptor.page.page_id));
                        continue;
                    }
                    if let Err(error) = context.close_page(descriptor.page.page_id).await {
                        // A page may close itself between the snapshot and this
                        // call. Only PageNotFound with an observed closed state
                        // resolves that race; crashes and protocol errors fail.
                        let already_closed = matches!(error, oryn_native::RuntimeError::PageNotFound(_))
                            && context.pages().iter().any(|page| page.page.page_id == descriptor.page.page_id
                            && matches!(page.status, oryn_native::PageStatus::Closed));
                        if !already_closed { cleanup_errors.push(error.to_string()); }
                    }
                }
            })
            .await;
            if cleanup.is_err() {
                cleanup_errors.push("teardown_timeout".into());
            }
            match tokio::time::timeout(Duration::from_secs(15), context.retire_workers()).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => cleanup_errors.push(error.to_string()),
                Err(_) => cleanup_errors.push("worker_retirement_timeout".into()),
            }
            let audit = context.resource_audit_snapshot();
            let resource_evidence_complete = audit.as_ref().is_some_and(|audit| audit.complete);
            let resource_error = if audit.is_none() {
                Some("resource_audit_unavailable")
            } else {
                trace_error
            };
            let resources = audit
                .as_ref()
                .map(|audit| audit.records.clone())
                .unwrap_or_default();
            let audit_metadata = audit.as_ref().map(|audit| json!({"schema_version":audit.schema_version,
                "context_id":audit.context_id,"complete":audit.complete,"overflow":audit.overflow,
                "gaps":audit.gaps,"websockets":audit.websockets}));
            drop(page);
            drop(context);
            let teardown_ms = teardown.elapsed().as_millis();
            let (completion, error) = match result {
                Ok(Ok(value)) => (Some(value), None),
                Ok(Err(error)) => (None, Some(error)),
                Err(_) => (None, Some("outer_execution_timeout".into())),
            };
            let replacement_error = browser.prepare_workers().err().map(|e| e.to_string());
            let response = json!({"event":"result","id":request.id,"page_id":page_id,
                "completion":completion,"error":error,"cleanup_errors":cleanup_errors,
                "replacement_error":replacement_error,"queue_ms":queue_ms,"run_ms":run_ms,
                "teardown_ms":teardown_ms,"resources":resources,"resource_error":resource_error,
                "resource_evidence_complete":resource_evidence_complete,"resource_audit":audit_metadata,
                "policy_events":policy_events,"worker_memory":memory_samples});
            let result = emit(&output, response).await;
            drop(permit);
            result
        });
    }
    while let Some(result) = tasks.join_next().await {
        result??;
    }
    emit(&output, json!({"event":"shutdown"})).await?;
    Ok(())
}

async fn emit(
    output: &tokio::sync::Mutex<tokio::io::Stdout>,
    mut value: Value,
) -> std::io::Result<()> {
    let mut frame = serde_json::to_vec(&value)?;
    if frame.len() > 24 * 1024 * 1024 {
        if value["event"] != "result" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "WPT response exceeds limit",
            ));
        }
        value["completion"] = Value::Null;
        value["resources"] = json!([]);
        value["resource_audit"] = Value::Null;
        value["policy_events"] = json!([]);
        value["error"] = json!("result_frame_limit");
        value["resource_error"] = json!("result_frame_limit");
        value["resource_evidence_complete"] = json!(false);
        frame = serde_json::to_vec(&value)?;
    }
    frame.push(b'\n');
    let mut output = output.lock().await;
    output.write_all(&frame).await?;
    output.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn frames_are_bounded_and_require_newlines() {
        let mut input = BufReader::new(&b"{}\n{}\n"[..]);
        assert_eq!(read_frame(&mut input).await.unwrap().unwrap(), b"{}\n");
        assert_eq!(read_frame(&mut input).await.unwrap().unwrap(), b"{}\n");
        assert!(read_frame(&mut input).await.unwrap().is_none());
        let mut input = BufReader::new(&b"{}"[..]);
        assert!(read_frame(&mut input).await.is_err());
        let oversized = vec![b'x'; MAX_REQUEST_BYTES + 2];
        let mut input = BufReader::new(oversized.as_slice());
        assert!(read_frame(&mut input).await.is_err());
    }
    #[test]
    fn requests_reject_unknown_fields_and_bad_types() {
        for source in [
            r#"{"command":"run","id":1}"#,
            r#"{"command":"run","id":"x","secret":"y"}"#,
        ] {
            assert!(serde_json::from_str::<Request>(source).is_err());
        }
    }
}
