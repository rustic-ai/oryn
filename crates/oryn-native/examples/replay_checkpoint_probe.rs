use std::collections::BTreeMap;

use oryn_native::oil::NativeContextOilSession;
use oryn_native::{
    Browser, ContextOptions, DeterminismConfig, FailureArtifact, FailureArtifactPayload,
    TRACE_ARCHIVE_VERSION,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let determinism = DeterminismConfig {
        unix_time_ms: 1_700_000_000_123,
        random_seed: 73,
    };
    let browser = Browser::new();
    let context = browser.new_recording_context(
        ContextOptions::default().with_determinism(determinism),
        true,
    );
    context
        .new_page()
        .load_html("about:blank", "<!doctype html><title>Replay root</title>")
        .await?;
    let checkpoint = context
        .checkpoint(
            "replay-root",
            BTreeMap::from([("task".into(), "replay-probe".into())]),
            Vec::new(),
        )
        .await?;
    let mut oil = NativeContextOilSession::new(context.clone());
    oil.execute("page current").await?;
    let trace = context.trace_archive(Vec::new()).await?;
    let (_replayed, replay) = browser
        .replay_task_trace(ContextOptions::default(), &checkpoint, &trace)
        .await?;
    let (_branch_context, branch) = browser
        .branch_from_checkpoint(ContextOptions::default(), &checkpoint, "alternate")
        .await?;
    let failure = FailureArtifact::seal(FailureArtifactPayload {
        schema_version: TRACE_ARCHIVE_VERSION,
        failure_id: "replay-controlled-failure".into(),
        checkpoint: checkpoint.clone(),
        trace: trace.clone(),
        error: "controlled fixture failure".into(),
    })?;
    failure.verify()?;
    let passed = checkpoint.verify().is_ok()
        && replay.attempted_intents == 1
        && replay.replayed_intents == 1
        && replay.divergences.is_empty()
        && branch.parent_checkpoint_sha256 == checkpoint.sha256
        && failure.verify().is_ok();
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "schema_version": 1,
            "suite": "oryn2-replay-probe",
            "checkpoint_sha256": checkpoint.sha256,
            "trace": {
                "intents": trace.intents.len(),
                "pages": trace.page_traces.len(),
                "network_exchanges": trace.network.exchanges.len(),
                "nondeterministic_regions": trace.nondeterministic_regions
            },
            "replay": replay,
            "branch": branch,
            "failure_artifact_sha256": failure.sha256,
            "outcome": {"status": if passed { "passed" } else { "failed" }}
        }))?
    );
    if !passed {
        return Err("replay probe failed".into());
    }
    Ok(())
}
