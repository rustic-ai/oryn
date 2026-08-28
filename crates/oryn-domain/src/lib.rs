//! Optional compatibility and delegated-user-browser execution domains.
//!
//! This crate is intentionally outside `oryn-native`. It translates a
//! policy-approved handoff into the frozen Oryn1 backend interface while the
//! native kernel remains scanner/CDP/WebDriver-free.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use oryn_common::{
    protocol::{ExecuteRequest, ScannerAction},
    v3::{ExecutionDomain, SupportLevel},
};
use oryn_engine::backend::Backend;
use oryn_native::{
    BrowserContext, CapabilityId, CapabilityLedger, ConfirmationToken, DomainDecision,
    DomainDescriptor, ExecutionDomainAdapter, ExecutionDomainPolicy, ExecutionRequirements,
    HandoffBundle, HandoffOptions, HandoffReceipt, PageId, RuntimeError, select_execution_domain,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainTelemetryRecord {
    pub sequence: u64,
    pub source: ExecutionDomain,
    pub selected: ExecutionDomain,
    pub decision: DomainDecision,
    #[serde(default)]
    pub handoff_lossy: bool,
    #[serde(default)]
    pub destination_revision: Option<u64>,
    #[serde(default)]
    pub failure: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainMetrics {
    pub selections: u64,
    pub handoffs: u64,
    pub failures: u64,
    pub domain_mix: BTreeMap<String, u64>,
    pub reason_mix: BTreeMap<String, u64>,
}

pub struct DomainOrchestrator {
    ledger: CapabilityLedger,
    policy: ExecutionDomainPolicy,
    adapters: Vec<Box<dyn ExecutionDomainAdapter>>,
    telemetry: Vec<DomainTelemetryRecord>,
}

impl DomainOrchestrator {
    pub fn new(ledger: CapabilityLedger, policy: ExecutionDomainPolicy) -> Self {
        Self {
            ledger,
            policy,
            adapters: Vec::new(),
            telemetry: Vec::new(),
        }
    }

    pub fn register(&mut self, adapter: impl ExecutionDomainAdapter + 'static) {
        let descriptor = adapter.descriptor();
        self.ledger.register(descriptor.clone());
        self.adapters
            .retain(|item| item.descriptor().domain != descriptor.domain);
        self.adapters.push(Box::new(adapter));
    }

    pub fn ledger(&self) -> &CapabilityLedger {
        &self.ledger
    }

    pub fn telemetry(&self) -> &[DomainTelemetryRecord] {
        &self.telemetry
    }

    pub fn metrics(&self) -> DomainMetrics {
        let mut metrics = DomainMetrics::default();
        for record in &self.telemetry {
            metrics.selections += 1;
            *metrics
                .domain_mix
                .entry(serialized_key(&record.selected))
                .or_default() += 1;
            *metrics
                .reason_mix
                .entry(serialized_key(&record.decision.reason))
                .or_default() += 1;
            if record.destination_revision.is_some() {
                metrics.handoffs += 1;
            }
            if record.failure.is_some() {
                metrics.failures += 1;
            }
        }
        metrics
    }

    pub fn select(
        &self,
        requirements: &ExecutionRequirements,
    ) -> Result<DomainDecision, RuntimeError> {
        select_execution_domain(&self.ledger, &self.policy, requirements)
            .map_err(|error| RuntimeError::InvalidHandoff(error.to_string()))
    }

    pub async fn handoff_native_page(
        &mut self,
        context: &BrowserContext,
        page_id: PageId,
        requirements: &ExecutionRequirements,
        mut options: HandoffOptions,
        confirmation: Option<ConfirmationToken>,
    ) -> Result<HandoffReceipt, RuntimeError> {
        let decision = self.select(requirements)?;
        context.record_domain_decision(decision.clone());
        if decision.selected == ExecutionDomain::Native {
            return Err(RuntimeError::InvalidHandoff(
                "selected domain is already native".into(),
            ));
        }
        options.allow_lossy |= self.policy.allow_lossy_handoff;
        let bundle = context
            .prepare_handoff(page_id, decision.selected, options, confirmation)
            .await?;
        let handoff_lossy = !bundle.losses.is_empty();
        let adapter = self
            .adapters
            .iter_mut()
            .find(|adapter| adapter.descriptor().domain == decision.selected)
            .ok_or_else(|| {
                RuntimeError::InvalidHandoff(format!(
                    "selected {:?} adapter is not registered",
                    decision.selected
                ))
            })?;
        let sequence = self.telemetry.len() as u64;
        match adapter.accept_handoff(bundle).await {
            Ok(receipt) => {
                self.telemetry.push(DomainTelemetryRecord {
                    sequence,
                    source: ExecutionDomain::Native,
                    selected: decision.selected,
                    decision,
                    handoff_lossy,
                    destination_revision: Some(receipt.destination_revision),
                    failure: None,
                });
                Ok(receipt)
            }
            Err(error) => {
                self.telemetry.push(DomainTelemetryRecord {
                    sequence,
                    source: ExecutionDomain::Native,
                    selected: decision.selected,
                    decision,
                    handoff_lossy,
                    destination_revision: None,
                    failure: Some(error.to_string()),
                });
                Err(error)
            }
        }
    }
}

fn serialized_key(value: &impl Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}

pub struct CompatibilityAdapter<B> {
    backend: B,
    descriptor: DomainDescriptor,
    revision: u64,
}

impl<B> CompatibilityAdapter<B> {
    pub fn new(backend: B, descriptor: DomainDescriptor) -> Self {
        Self {
            backend,
            descriptor,
            revision: 0,
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }
}

#[async_trait]
impl<B: Backend + Send> ExecutionDomainAdapter for CompatibilityAdapter<B> {
    fn descriptor(&self) -> DomainDescriptor {
        self.descriptor.clone()
    }

    async fn accept_handoff(
        &mut self,
        bundle: HandoffBundle,
    ) -> Result<HandoffReceipt, RuntimeError> {
        if bundle.target_domain != self.descriptor.domain {
            return Err(RuntimeError::InvalidHandoff(format!(
                "bundle targets {:?}, adapter owns {:?}",
                bundle.target_domain, self.descriptor.domain
            )));
        }
        if !self.backend.is_ready().await {
            self.backend
                .launch()
                .await
                .map_err(|error| RuntimeError::InvalidHandoff(error.to_string()))?;
        }
        self.backend
            .navigate(&bundle.url)
            .await
            .map_err(|error| RuntimeError::InvalidHandoff(error.to_string()))?;

        let state = serde_json::json!({
            "storage": bundle.storage,
            "forms": bundle.form_values,
            "cookie": bundle.cookie_header.as_ref().map(|cookie| cookie.expose_to_adapter()),
        });
        let state = serde_json::to_string(&state)
            .map_err(|error| RuntimeError::InvalidHandoff(error.to_string()))?;
        let script = format!(
            r#"(()=>{{const state={state};for(const area of state.storage){{for(const [key,value] of area.local)localStorage.setItem(key,value);for(const [key,value] of area.session)sessionStorage.setItem(key,value);}}if(state.cookie){{for(const pair of state.cookie.split(';'))document.cookie=pair.trim();}}for(const field of state.forms){{const node=document.querySelector(field.selector);if(!node)continue;if(field.action==='check'||field.action==='uncheck')node.checked=field.checked;else node.value=field.value;node.dispatchEvent(new Event('input',{{bubbles:true}}));node.dispatchEvent(new Event('change',{{bubbles:true}}));}}return true;}})()"#
        );
        self.backend
            .execute_scanner(ScannerAction::Execute(ExecuteRequest {
                script,
                args: Vec::new(),
            }))
            .await
            .map_err(|error| RuntimeError::InvalidHandoff(error.to_string()))?;
        self.revision = self.revision.saturating_add(1);
        Ok(HandoffReceipt {
            contract_version: oryn_native::execution::EXECUTION_CONTRACT_VERSION,
            source_domain: bundle.source_domain,
            target_domain: bundle.target_domain,
            destination_url: bundle.url,
            destination_revision: self.revision,
            state_applied: vec!["navigation".into(), "storage".into(), "form_values".into()],
            losses: bundle.losses,
        })
    }
}

pub fn native_descriptor() -> DomainDescriptor {
    descriptor(
        ExecutionDomain::Native,
        false,
        false,
        &["dom", "semantic", "causal_trace", "layout", "storage"],
    )
}

pub fn chromium_descriptor() -> DomainDescriptor {
    descriptor(
        ExecutionDomain::Chromium,
        false,
        false,
        &["dom", "layout", "paint", "media", "long_tail_web_api"],
    )
}

pub fn user_browser_descriptor() -> DomainDescriptor {
    descriptor(
        ExecutionDomain::UserBrowser,
        true,
        true,
        &[
            "dom",
            "layout",
            "paint",
            "authenticated_session",
            "extension",
            "user_presence",
        ],
    )
}

fn descriptor(
    domain: ExecutionDomain,
    user_visible: bool,
    owns_authenticated_session: bool,
    capabilities: &[&str],
) -> DomainDescriptor {
    DomainDescriptor {
        domain,
        installed: true,
        canonical_semantics: domain == ExecutionDomain::Native,
        user_visible,
        owns_authenticated_session,
        capabilities: capabilities
            .iter()
            .map(|capability| (CapabilityId::from(*capability), SupportLevel::Supported))
            .collect(),
        known_bad_origins: BTreeSet::new(),
    }
}

#[cfg(feature = "chromium")]
pub fn chromium_adapter(visible: bool) -> CompatibilityAdapter<oryn_h::backend::HeadlessBackend> {
    CompatibilityAdapter::new(
        oryn_h::backend::HeadlessBackend::new_with_visibility(visible),
        chromium_descriptor(),
    )
}

#[cfg(feature = "user-browser")]
pub fn user_browser_adapter(port: u16) -> CompatibilityAdapter<oryn_r::backend::RemoteBackend> {
    CompatibilityAdapter::new(
        oryn_r::backend::RemoteBackend::new(port),
        user_browser_descriptor(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_common::protocol::{ScannerData, ScannerProtocolResponse};
    use oryn_engine::backend::{BackendError, NavigationResult};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct MockBackend {
        scripts: Arc<Mutex<Vec<String>>>,
        ready: bool,
    }

    #[async_trait]
    impl Backend for MockBackend {
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
            Ok(NavigationResult {
                url: url.into(),
                title: String::new(),
                status: 200,
            })
        }
        async fn execute_scanner(
            &mut self,
            command: ScannerAction,
        ) -> Result<ScannerProtocolResponse, BackendError> {
            if let ScannerAction::Execute(request) = command {
                self.scripts.lock().unwrap().push(request.script);
            }
            Ok(ScannerProtocolResponse::Ok {
                data: Box::new(ScannerData::Value(serde_json::Value::Bool(true))),
                warnings: Vec::new(),
            })
        }
        async fn screenshot(&mut self) -> Result<Vec<u8>, BackendError> {
            Err(BackendError::NotSupported("screenshot".into()))
        }
    }

    #[tokio::test]
    async fn compatibility_adapter_reloads_and_applies_transferred_state() {
        let scripts = Arc::new(Mutex::new(Vec::new()));
        let backend = MockBackend {
            scripts: scripts.clone(),
            ready: false,
        };
        let mut adapter = CompatibilityAdapter::new(backend, chromium_descriptor());
        let receipt = adapter
            .accept_handoff(HandoffBundle {
                contract_version: 1,
                source_domain: ExecutionDomain::Native,
                target_domain: ExecutionDomain::Chromium,
                source_page_id: PageId(1),
                url: "https://example.test/app".into(),
                document_generation: 3,
                storage: Vec::new(),
                form_values: Vec::new(),
                cookie_header: None,
                task_metadata: BTreeMap::new(),
                checkpoint: None,
                losses: Vec::new(),
            })
            .await
            .expect("handoff");
        assert_eq!(receipt.destination_revision, 1);
        assert_eq!(scripts.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn orchestrator_handoff_records_domain_and_reason_metrics() {
        let scripts = Arc::new(Mutex::new(Vec::new()));
        let backend = MockBackend {
            scripts: scripts.clone(),
            ready: false,
        };
        let mut ledger = CapabilityLedger::new();
        ledger.register(native_descriptor());
        let mut orchestrator = DomainOrchestrator::new(
            ledger,
            ExecutionDomainPolicy {
                allow_lossy_handoff: true,
                ..ExecutionDomainPolicy::default()
            },
        );
        orchestrator.register(CompatibilityAdapter::new(backend, chromium_descriptor()));
        let context = oryn_native::Browser::in_process_probe().new_context();
        let page = context.new_page();
        page.load_html("https://example.test/app", "<button>Ready</button>")
            .await
            .expect("load native page");
        let receipt = orchestrator
            .handoff_native_page(
                &context,
                page.page_id(),
                &ExecutionRequirements {
                    capabilities: [CapabilityId::from("paint")].into_iter().collect(),
                    ..ExecutionRequirements::default()
                },
                HandoffOptions::default(),
                None,
            )
            .await
            .expect("handoff to chromium");
        assert_eq!(receipt.target_domain, ExecutionDomain::Chromium);
        assert_eq!(receipt.destination_revision, 1);
        let metrics = orchestrator.metrics();
        assert_eq!(metrics.selections, 1);
        assert_eq!(metrics.handoffs, 1);
        assert_eq!(metrics.domain_mix.get("chromium"), Some(&1));
        assert_eq!(
            metrics.reason_mix.get("required_capability_unavailable"),
            Some(&1)
        );
        assert_eq!(scripts.lock().unwrap().len(), 1);
    }
}
