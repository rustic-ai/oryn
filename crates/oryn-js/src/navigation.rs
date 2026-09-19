//! Owned navigation data shared by the JS boundary and native loaders.

/// Parent-authenticated destination identity, separate from load authorization.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NavigationTargetReceipt(pub Vec<u8>);
impl std::fmt::Debug for NavigationTargetReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NavigationTargetReceipt")
            .field("bytes", &self.0.len())
            .finish()
    }
}

/// Noopener turns a named auxiliary target into a fresh unnamed context.
/// Empty target follows the normal self-target default.
pub fn effective_navigation_target(target: &str, noopener: bool) -> &str {
    if noopener
        && !target.is_empty()
        && !["_self", "_parent", "_top"]
            .iter()
            .any(|keyword| target.eq_ignore_ascii_case(keyword))
    {
        "_blank"
    } else {
        target
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PostResource {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

impl PostResource {
    /// Validate at each privileged loader boundary, including redirects.
    pub fn valid_for(&self, url: &url::Url) -> bool {
        matches!(url.scheme(), "http" | "https")
            && self.bytes.len() <= 16 * 1024 * 1024
            && !self.content_type.contains(['\r', '\n'])
    }
    /// Fetch redirect semantics: 307/308 retain a POST resource, 301/302/303 do not.
    pub fn redirect(resource: &mut Option<Self>, status: u16) {
        if matches!(status, 301..=303) {
            *resource = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noopener_does_not_reuse_names_but_preserves_reserved_existing_targets() {
        assert_eq!(effective_navigation_target("receipt", true), "_blank");
        assert_eq!(effective_navigation_target("receipt", false), "receipt");
        for target in ["", "_self", "_PARENT", "_top"] {
            assert_eq!(effective_navigation_target(target, true), target);
        }
    }

    #[test]
    fn post_resources_reject_non_http_and_header_injection() {
        let mut resource = PostResource {
            bytes: vec![1],
            content_type: "text/plain".into(),
        };
        assert!(resource.valid_for(&url::Url::parse("https://example.test/").unwrap()));
        for url in [
            "data:text/plain,ignored",
            "file:///tmp/ignored",
            "about:blank",
        ] {
            assert!(!resource.valid_for(&url::Url::parse(url).unwrap()));
        }
        resource.content_type = "text/plain\r\nx-injected: yes".into();
        assert!(!resource.valid_for(&url::Url::parse("https://example.test/").unwrap()));
    }
}

/// Parent-authenticated original form policy. This is provenance, not an action
/// approval; consumers still enforce consequential-action policy independently.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FormNavigationAuthority(pub Vec<u8>);
impl std::fmt::Debug for FormNavigationAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FormNavigationAuthority([native policy])")
    }
}

/// Parent-authenticated fetch metadata for a form's original source document.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct FormNavigationSource {
    /// Native transient activation frozen when this navigation was accepted.
    #[serde(default)]
    pub user_activation: bool,
    #[serde(default)]
    pub source_domain_key: Option<String>,
    /// Authenticated origin identity, including opaque-origin identity. The
    /// serialized URL alone cannot determine Navigation event disclosure.
    pub origin_key: String,
    pub url: String,
    pub referrer_policy: String,
    pub top_level_url: String,
    pub ancestor_urls: Vec<String>,
    pub opaque_origin: bool,
}

/// Parent-sealed source metadata for one accepted script navigation. This is
/// distinct from form authority: it cannot bypass form-action or form sandbox
/// checks by being supplied where a form token is required.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScriptNavigationAuthority(pub Vec<u8>);
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScriptNavigationAdmission {
    pub authority: ScriptNavigationAuthority,
    pub target: crate::worker_wire::Creator,
}
impl std::fmt::Debug for ScriptNavigationAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ScriptNavigationAuthority([native source])")
    }
}

/// Parent-sealed service worker source metadata for one target navigation.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ServiceNavigationAuthority(pub Vec<u8>);
impl std::fmt::Debug for ServiceNavigationAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServiceNavigationAuthority([native source])")
    }
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ServiceNavigationAdmission {
    pub authority: ServiceNavigationAuthority,
    pub target: crate::worker_wire::Creator,
}

/// Parent-minted execution correlation; never constructed from author JSON.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct NavigationExecutionToken(pub Vec<u8>);
impl std::fmt::Debug for NavigationExecutionToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NavigationExecutionToken")
            .field("bytes", &self.0.len())
            .finish()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NavigationExecutionOutcome {
    Committed {
        document_generation: u64,
    },
    /// A same-document or intercepted operation finished without a new Document.
    Completed,
    Canceled,
    Failed {
        message: String,
    },
}

/// Thread-safe native custody shared by JS admission and the eventual loader.
/// The sink must retain its publication until the parent acknowledges it.
#[derive(Clone)]
pub struct NavigationExecutionObserver(std::sync::Arc<NavigationExecutionObserverInner>);
struct NavigationExecutionObserverInner {
    publication: std::sync::Mutex<()>,
    settled: std::sync::atomic::AtomicBool,
    loader_owned: std::sync::atomic::AtomicBool,
    admitted: std::sync::atomic::AtomicBool,
    admission_sink: Box<dyn Fn() + Send + Sync>,
    sink: Box<dyn Fn(NavigationExecutionOutcome) + Send + Sync>,
}
impl std::fmt::Debug for NavigationExecutionObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NavigationExecutionObserver([native custody])")
    }
}
impl NavigationExecutionObserver {
    pub fn new(sink: impl Fn(NavigationExecutionOutcome) + Send + Sync + 'static) -> Self {
        Self::with_admission(|| {}, sink)
    }
    /// Admission is distinct from both queued dispatch and terminal settlement.
    pub fn with_admission(
        admission: impl Fn() + Send + Sync + 'static,
        sink: impl Fn(NavigationExecutionOutcome) + Send + Sync + 'static,
    ) -> Self {
        Self(std::sync::Arc::new(NavigationExecutionObserverInner {
            publication: std::sync::Mutex::new(()),
            settled: std::sync::atomic::AtomicBool::new(false),
            loader_owned: std::sync::atomic::AtomicBool::new(false),
            admitted: std::sync::atomic::AtomicBool::new(false),
            admission_sink: Box::new(admission),
            sink: Box::new(sink),
        }))
    }
    /// Called on the admitting realm before handing custody to a loader, or
    /// before starting an accepted same-document/intercepted commit.
    pub fn admitted(&self) {
        let _publication = self
            .0
            .publication
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !self.0.settled.load(std::sync::atomic::Ordering::Acquire)
            && !self
                .0
                .admitted
                .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            (self.0.admission_sink)();
        }
    }
    pub fn loader_owned(&self) {
        self.0
            .loader_owned
            .store(true, std::sync::atomic::Ordering::Release);
    }
    pub fn complete(&self, outcome: NavigationExecutionOutcome) {
        let _publication = self
            .0
            .publication
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !self
            .0
            .settled
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            (self.0.sink)(outcome);
        }
    }
    pub fn complete_document_operation(&self, outcome: NavigationExecutionOutcome) {
        if !self
            .0
            .loader_owned
            .load(std::sync::atomic::Ordering::Acquire)
        {
            self.complete(outcome);
        }
    }
}
impl Drop for NavigationExecutionObserverInner {
    fn drop(&mut self) {
        if !self.settled.swap(true, std::sync::atomic::Ordering::AcqRel) {
            (self.sink)(NavigationExecutionOutcome::Failed {
                message: "navigation execution owner dropped before terminal settlement".into(),
            });
        }
    }
}

#[cfg(test)]
mod execution_observer_tests {
    use super::*;
    #[test]
    fn loader_transfer_ignores_source_retirement_and_publishes_once() {
        let values = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = values.clone();
        let observer =
            NavigationExecutionObserver::new(move |value| sink.lock().unwrap().push(value));
        let loader = observer.clone();
        observer.loader_owned();
        observer.complete_document_operation(NavigationExecutionOutcome::Canceled);
        drop(observer);
        assert!(values.lock().unwrap().is_empty());
        loader.complete(NavigationExecutionOutcome::Committed {
            document_generation: 7,
        });
        loader.complete(NavigationExecutionOutcome::Canceled);
        drop(loader);
        assert_eq!(
            *values.lock().unwrap(),
            vec![NavigationExecutionOutcome::Committed {
                document_generation: 7
            }]
        );
    }
    #[test]
    fn acceptance_is_one_shot_and_never_inferred_from_terminal_success() {
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let accepted = events.clone();
        let terminal = events.clone();
        let observer = NavigationExecutionObserver::with_admission(
            move || accepted.lock().unwrap().push("accepted"),
            move |_| terminal.lock().unwrap().push("terminal"),
        );
        observer.admitted();
        observer.admitted();
        observer.complete(NavigationExecutionOutcome::Completed);
        observer.admitted();
        drop(observer);
        assert_eq!(*events.lock().unwrap(), vec!["accepted", "terminal"]);
        events.lock().unwrap().clear();
        let accepted = events.clone();
        let terminal = events.clone();
        let observer = NavigationExecutionObserver::with_admission(
            move || accepted.lock().unwrap().push("accepted"),
            move |_| terminal.lock().unwrap().push("terminal"),
        );
        observer.complete(NavigationExecutionOutcome::Canceled);
        observer.admitted();
        drop(observer);
        assert_eq!(*events.lock().unwrap(), vec!["terminal"]);
    }
    #[test]
    fn abandoned_native_execution_never_becomes_success() {
        let values = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = values.clone();
        let observer =
            NavigationExecutionObserver::new(move |value| sink.lock().unwrap().push(value));
        let retained = observer.clone();
        drop(observer);
        assert!(values.lock().unwrap().is_empty());
        drop(retained);
        assert!(matches!(
            values.lock().unwrap().as_slice(),
            [NavigationExecutionOutcome::Failed { .. }]
        ));
    }
}

/// Native one-shot hyperlink auditing capability. Never exposed to author JS.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HyperlinkAuditToken {
    pub page_id: u64,
    pub capability: Vec<u8>,
}
impl std::fmt::Debug for HyperlinkAuditToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HyperlinkAuditToken([native capability])")
    }
}
pub trait HyperlinkAudit: Send + Sync {
    fn token(&self) -> HyperlinkAuditToken;
    fn accepted(&self);
    fn discarded(&self);
    /// Transfer cancellation ownership to the parent destination receipt.
    fn transferred(&self);
}
