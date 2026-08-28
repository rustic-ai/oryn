use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use oryn_common::{
    protocol::{ScannerAction, ScannerData, ScannerProtocolResponse},
    v3::ExecutionDomain,
};
use oryn_domain::{
    CompatibilityAdapter, DomainOrchestrator, chromium_descriptor, native_descriptor,
    user_browser_descriptor,
};
use oryn_engine::backend::{Backend, BackendError, NavigationResult};
use oryn_native::{
    Browser, CapabilityId, CapabilityLedger, ConsequenceLevel, ExecutionDomainPolicy,
    ExecutionRequirements, HandoffOptions,
};

#[derive(Default)]
struct ProbeBackend {
    ready: bool,
    navigations: Vec<String>,
    state_applications: u64,
}

#[async_trait]
impl Backend for ProbeBackend {
    async fn launch(&mut self) -> Result<(), BackendError> {
        self.ready = true;
        Ok(())
    }
    async fn close(&mut self) -> Result<(), BackendError> {
        self.ready = false;
        Ok(())
    }
    async fn is_ready(&self) -> bool {
        self.ready
    }
    async fn navigate(&mut self, url: &str) -> Result<NavigationResult, BackendError> {
        self.navigations.push(url.into());
        Ok(NavigationResult {
            url: url.into(),
            title: "G6 destination".into(),
            status: 200,
        })
    }
    async fn execute_scanner(
        &mut self,
        command: ScannerAction,
    ) -> Result<ScannerProtocolResponse, BackendError> {
        if matches!(command, ScannerAction::Execute(_)) {
            self.state_applications += 1;
        }
        Ok(ScannerProtocolResponse::Ok {
            data: Box::new(ScannerData::Value(serde_json::Value::Bool(true))),
            warnings: Vec::new(),
        })
    }
    async fn screenshot(&mut self) -> Result<Vec<u8>, BackendError> {
        Err(BackendError::NotSupported("probe".into()))
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut ledger = CapabilityLedger::new();
    ledger.register(native_descriptor());
    ledger.register(chromium_descriptor());
    ledger.register(user_browser_descriptor());
    let policy = ExecutionDomainPolicy {
        allow_lossy_handoff: true,
        ..ExecutionDomainPolicy::default()
    };
    let mut orchestrator = DomainOrchestrator::new(ledger, policy);
    orchestrator.register(CompatibilityAdapter::new(
        ProbeBackend::default(),
        chromium_descriptor(),
    ));

    let native = orchestrator.select(&ExecutionRequirements::default())?;
    let paint = orchestrator.select(&ExecutionRequirements {
        capabilities: BTreeSet::from([CapabilityId::from("paint")]),
        ..ExecutionRequirements::default()
    })?;
    let auth = orchestrator.select(&ExecutionRequirements {
        requires_authenticated_session: true,
        consequence: ConsequenceLevel::Consequential,
        ..ExecutionRequirements::default()
    })?;
    let challenge = orchestrator.select(&ExecutionRequirements {
        site_challenged: true,
        ..ExecutionRequirements::default()
    })?;

    let context = Browser::new().new_context();
    let page = context.new_page();
    page.load_html(
        "https://example.test/g6",
        "<input id=name value=Oryn><button>Continue</button>",
    )
    .await?;
    let receipt = orchestrator
        .handoff_native_page(
            &context,
            page.page_id(),
            &ExecutionRequirements {
                capabilities: BTreeSet::from([CapabilityId::from("paint")]),
                ..ExecutionRequirements::default()
            },
            HandoffOptions {
                task_metadata: BTreeMap::from([("task".into(), "g6-probe".into())]),
                checkpoint: Some("g6-checkpoint".into()),
                ..HandoffOptions::default()
            },
            None,
        )
        .await?;
    let metrics = orchestrator.metrics();
    let trace = context.trace_archive(Vec::new()).await?;
    let passed = native.selected == ExecutionDomain::Native
        && paint.selected == ExecutionDomain::Chromium
        && auth.selected == ExecutionDomain::UserBrowser
        && challenge.selected == ExecutionDomain::Chromium
        && receipt.destination_revision > 0
        && !receipt.losses.is_empty()
        && metrics.handoffs == 1
        && trace.domain_decisions.len() == 1;
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "schema_version": 1,
            "suite": "oryn2-g6-probe",
            "decisions": {"native": native, "paint": paint, "auth": auth, "challenge": challenge},
            "handoff": receipt,
            "metrics": metrics,
            "trace_domain_decisions": trace.domain_decisions,
            "outcome": {"status": if passed { "passed" } else { "failed" }}
        }))?
    );
    if !passed {
        return Err("G6 probe failed".into());
    }
    Ok(())
}
