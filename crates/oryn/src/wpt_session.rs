//! Explicit diagnostic transport. Each execution owns a fresh browser context.
use oryn_native::{Browser, BrowserConfig, ContextOptions, WorkerLifecycle};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_DIAGNOSTIC_EVENTS: usize = 256;
const MAX_DIAGNOSTIC_BYTES: usize = 1024 * 1024;

#[derive(Default)]
struct TraceExport {
    expected_live_pages: BTreeSet<String>,
    captured_pages: BTreeSet<String>,
    policy_events: Vec<Value>,
    diagnostic_events: Vec<Value>,
    diagnostic_bytes: usize,
    dropped_events: u64,
    dropped_bytes: u64,
    source_overflow: bool,
    collection_error: Option<String>,
}

impl TraceExport {
    fn collect(
        &mut self,
        pages: impl IntoIterator<Item = (String, Vec<oryn_native::trace::TraceEvent>)>,
    ) {
        for (page, events) in pages {
            self.captured_pages.insert(page.clone());
            for event in events {
                let diagnostic = matches!(
                    &event.kind,
                    oryn_native::trace::TraceEventKind::ScriptDiagnostic { .. }
                ) || matches!(&event.kind,
                        oryn_native::trace::TraceEventKind::Console { level, .. } if level == "error");
                self.source_overflow |= matches!(&event.kind,
                    oryn_native::trace::TraceEventKind::ScriptDiagnostic { capability, .. }
                        if capability == "diagnostics.overflow");
                if let oryn_native::trace::TraceEventKind::ScriptDiagnostic { capability, detail } =
                    &event.kind
                    && capability == "diagnostics.collection"
                {
                    self.collection_error.get_or_insert_with(|| detail.clone());
                }
                let kind = match serde_json::to_value(event.kind) {
                    Ok(kind) => kind,
                    Err(error) => {
                        self.collection_error =
                            Some(format!("trace serialization failed: {error}"));
                        continue;
                    }
                };
                let record = json!({"page_id":page,"sequence":event.sequence,"event":kind});
                if matches!(
                    record["event"]["kind"].as_str(),
                    Some(
                        "request_started" | "response_received" | "policy_denied" | "csp_violation"
                    )
                ) {
                    self.policy_events.push(record);
                } else if diagnostic {
                    let bytes = match serde_json::to_vec(&record) {
                        Ok(bytes) => bytes.len(),
                        Err(error) => {
                            self.collection_error =
                                Some(format!("diagnostic serialization failed: {error}"));
                            continue;
                        }
                    };
                    if self.diagnostic_events.len() >= MAX_DIAGNOSTIC_EVENTS
                        || bytes > MAX_DIAGNOSTIC_BYTES.saturating_sub(self.diagnostic_bytes)
                    {
                        // A whole original error is either preserved or explicitly
                        // accounted for as dropped. Never present truncation as complete.
                        self.dropped_events = self.dropped_events.saturating_add(1);
                        self.dropped_bytes = self.dropped_bytes.saturating_add(bytes as u64);
                    } else {
                        self.diagnostic_bytes += bytes;
                        self.diagnostic_events.push(record);
                    }
                }
            }
        }
    }

    fn audit(&self) -> Value {
        let overflow = self.dropped_events != 0;
        let missing_pages: Vec<_> = self
            .expected_live_pages
            .difference(&self.captured_pages)
            .collect();
        json!({"schema_version":1,"scope":"pre_cleanup_live_pages",
            "coverage_snapshot":"before_archive",
            "expected_live_pages":self.expected_live_pages,"captured_pages":self.captured_pages,
            "missing_pages":missing_pages,
            "complete": !overflow && !self.source_overflow && self.collection_error.is_none() && missing_pages.is_empty(),
            "overflow":overflow,"source_overflow":self.source_overflow,
            "recorded_events":self.diagnostic_events.len(),"encoded_bytes":self.diagnostic_bytes,
            "dropped_events":self.dropped_events,"dropped_bytes":self.dropped_bytes,
            "event_limit":MAX_DIAGNOSTIC_EVENTS,"byte_limit":MAX_DIAGNOSTIC_BYTES,
            "collection_error":self.collection_error})
    }
}

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
            // trace_archive omits retired pages and a missing live handle. Keep
            // the expected population at the capture boundary so omission cannot
            // be reported as complete diagnostic evidence.
            let mut trace_export = TraceExport {
                expected_live_pages: context.pages().into_iter().filter(|descriptor|
                    !matches!(descriptor.status, oryn_native::PageStatus::Closed | oryn_native::PageStatus::Crashed))
                    .map(|descriptor| descriptor.page.page_id.0.to_string()).collect(),
                ..Default::default()
            };
            let trace =
                tokio::time::timeout(Duration::from_secs(5), context.trace_archive(Vec::new()))
                    .await;
            let trace_error = match trace {
                Ok(Ok(archive)) => {
                    trace_export.collect(archive.page_traces);
                    None
                }
                Ok(Err(error)) => {
                    trace_export.collection_error = Some(error.to_string());
                    Some("resource_trace_unavailable")
                }
                Err(error) => {
                    trace_export.collection_error = Some(error.to_string());
                    Some("resource_trace_unavailable")
                }
            };
            // Own the exported values before closing pages or retiring workers.
            // Cleanup must not replace an earlier scheduler/navigation failure.
            let diagnostic_audit = trace_export.audit();
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
                "policy_events":trace_export.policy_events,
                "diagnostic_events":trace_export.diagnostic_events,"diagnostic_audit":diagnostic_audit,
                "worker_memory":memory_samples});
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
    fn trace_event(
        page: u64,
        sequence: u64,
        kind: oryn_native::trace::TraceEventKind,
    ) -> oryn_native::trace::TraceEvent {
        oryn_native::trace::TraceEvent {
            sequence,
            logical_time: sequence,
            page_id: oryn_common::v3::PageId(page),
            frame_id: None,
            revision: oryn_common::v3::Revision(3),
            action_id: None,
            causality: Default::default(),
            kind,
        }
    }

    #[test]
    fn diagnostic_export_preserves_child_scheduler_and_unclaimed_navigation_errors() {
        use oryn_native::trace::TraceEventKind as Kind;
        let scheduler = "scheduled child failed: original λ\nsecond line";
        let navigation = "navigation: original broker denied child source";
        let mut exported = TraceExport::default();
        exported.collect([
            (
                "9".into(),
                vec![trace_event(
                    9,
                    17,
                    Kind::ScriptDiagnostic {
                        capability: "scheduler.page_task".into(),
                        detail: scheduler.into(),
                    },
                )],
            ),
            (
                "10".into(),
                vec![
                    trace_event(
                        10,
                        8,
                        Kind::Console {
                            level: "error".into(),
                            message: navigation.into(),
                        },
                    ),
                    trace_event(
                        10,
                        9,
                        Kind::RequestStarted {
                            method: "GET".into(),
                            url: "https://child.test/".into(),
                        },
                    ),
                    trace_event(
                        10,
                        10,
                        Kind::Console {
                            level: "log".into(),
                            message: "ordinary author log".into(),
                        },
                    ),
                    trace_event(
                        10,
                        11,
                        Kind::ScriptDiagnostic {
                            capability: "paint.cleanup".into(),
                            detail: "original cleanup join failure".into(),
                        },
                    ),
                ],
            ),
        ]);
        // The owned export remains unchanged when cleanup happens afterward.
        let original = exported.diagnostic_events.clone();
        let response = json!({"error":"primary timeout", "cleanup_errors":["close failed"],
            "diagnostic_events":exported.diagnostic_events,"diagnostic_audit":exported.audit(),
            "policy_events":exported.policy_events});
        assert_eq!(response["diagnostic_events"], json!(original));
        assert_eq!(response["diagnostic_events"][0]["page_id"], "9");
        assert_eq!(response["diagnostic_events"][0]["sequence"], 17);
        assert_eq!(
            response["diagnostic_events"][0]["event"]["detail"],
            scheduler
        );
        assert_eq!(
            response["diagnostic_events"][1]["event"]["message"],
            navigation
        );
        assert_eq!(
            response["diagnostic_events"][2]["event"]["capability"],
            "paint.cleanup"
        );
        assert_eq!(
            response["diagnostic_events"][2]["event"]["detail"],
            "original cleanup join failure"
        );
        assert_eq!(
            response["diagnostic_audit"]["scope"],
            "pre_cleanup_live_pages"
        );
        assert_eq!(response["diagnostic_audit"]["complete"], true);
        assert_eq!(response["policy_events"].as_array().unwrap().len(), 1);
        assert_eq!(
            response["policy_events"][0]["event"]["kind"],
            "request_started"
        );
    }

    #[test]
    fn diagnostic_export_bounds_count_and_bytes_without_fabricating_complete_evidence() {
        use oryn_native::trace::TraceEventKind as Kind;
        let mut exported = TraceExport::default();
        exported.collect([(
            "1".into(),
            (0..MAX_DIAGNOSTIC_EVENTS + 1)
                .map(|sequence| {
                    trace_event(
                        1,
                        sequence as u64,
                        Kind::Console {
                            level: "error".into(),
                            message: "whole original".into(),
                        },
                    )
                })
                .collect(),
        )]);
        assert_eq!(exported.diagnostic_events.len(), MAX_DIAGNOSTIC_EVENTS);
        assert_eq!(exported.audit()["dropped_events"], 1);
        assert_eq!(exported.audit()["complete"], false);
        let mut large = TraceExport::default();
        large.collect([(
            "2".into(),
            vec![trace_event(
                2,
                0,
                Kind::Console {
                    level: "error".into(),
                    message: "x".repeat(MAX_DIAGNOSTIC_BYTES + 1),
                },
            )],
        )]);
        assert!(large.diagnostic_events.is_empty());
        assert_eq!(large.audit()["overflow"], true);
        let mut source = TraceExport::default();
        source.collect([(
            "3".into(),
            vec![trace_event(
                3,
                0,
                Kind::ScriptDiagnostic {
                    capability: "diagnostics.overflow".into(),
                    detail: "original overflow".into(),
                },
            )],
        )]);
        assert_eq!(source.audit()["source_overflow"], true);
        assert_eq!(source.audit()["complete"], false);
        let empty = TraceExport::default();
        assert_eq!(empty.audit()["complete"], true);
        source.collection_error = Some("worker trace channel closed".into());
        assert_eq!(
            source.audit()["collection_error"],
            "worker trace channel closed"
        );
    }

    #[test]
    fn diagnostic_export_rejects_partial_console_collection() {
        let mut exported = TraceExport::default();
        exported.collect([(
            "1".into(),
            vec![trace_event(
                1,
                1,
                oryn_native::trace::TraceEventKind::ScriptDiagnostic {
                    capability: "diagnostics.collection".into(),
                    detail: "child drain failed".into(),
                },
            )],
        )]);
        assert_eq!(exported.audit()["complete"], false);
        assert_eq!(exported.audit()["collection_error"], "child drain failed");
        assert_eq!(exported.diagnostic_events.len(), 1);
    }

    #[test]
    fn diagnostic_export_reports_missing_live_pages_even_when_captured_pages_have_no_errors() {
        let mut exported = TraceExport {
            expected_live_pages: ["9".into(), "10".into()].into_iter().collect(),
            ..Default::default()
        };
        exported.collect([("9".into(), Vec::new())]);
        let audit = exported.audit();
        assert_eq!(audit["coverage_snapshot"], "before_archive");
        assert_eq!(audit["captured_pages"], json!(["9"]));
        assert_eq!(audit["missing_pages"], json!(["10"]));
        assert_eq!(audit["complete"], false);
        assert_eq!(audit["recorded_events"], 0);
        exported.collect([("10".into(), Vec::new())]);
        assert_eq!(exported.audit()["missing_pages"], json!([]));
        assert_eq!(exported.audit()["complete"], true);
    }
}
