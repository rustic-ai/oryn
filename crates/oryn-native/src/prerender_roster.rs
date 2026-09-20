//! One page actor owns all nonactive parser drivers. Candidate turns lend the
//! same isolate host; neither a pending IPC nor a failed driver owns that host.

/// Inputs retained together for poll prerender roster.
pub struct PrerenderPollInput<'a> {
    pub snapshot: Option<crate::host::PrerenderSourceSnapshot>,
    pub host: &'a mut Option<crate::host::RawV8Host>,
    pub client: SharedContextClient,
    pub page: PageId,
    pub broker: SharedNetworkBroker,
    pub source_url: &'a str,
    pub policy: crate::referrer::ReferrerPolicy,
    pub config: &'a BrowserConfig,
}
use super::*;
use prerender_load::{CandidateDocument, PendingCandidate, PendingReservation};
use std::task::Poll;
#[cfg(test)]
thread_local! { pub(super) static FAIL_AFTER_CONSTRUCTION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
type Key = (u64, usize);
type Source = (oryn_js::realm::RealmId, (oryn_dom::NodeId, u64, u64, u64));
#[derive(Default)]
pub(super) struct Roster {
    source: Option<Source>,
    jobs: BTreeMap<Key, Job>,
    retiring: VecDeque<oryn_js::realm::RealmId>,
    retry: Option<Instant>,
    interest_pending: bool,
    accepted_blank: bool,
    #[cfg(test)]
    diagnostic_failure: Option<String>,
}
struct Job {
    rule: oryn_js::speculation_rules::Candidate,
    deadline: Instant,
    phase: Phase,
}
enum Phase {
    Reserve(Box<PendingReservation>),
    Fetch(Box<PendingCandidate>),
    Document {
        document: Box<CandidateDocument>,
        ready: bool,
        ready_ack: Option<Box<navigation_fetch::PendingPolicy>>,
        loaded: bool,
        ack: Option<Box<navigation_fetch::PendingPolicy>>,
        next: Option<Instant>,
    },
    Failed,
}
fn is_blank(rule: &oryn_js::speculation_rules::Candidate) -> bool {
    rule.target == Some(oryn_js::speculation_rules::TargetHint::Blank)
}
impl Roster {
    #[cfg(test)]
    fn retain_diagnostic_failure(&mut self, error: &impl std::fmt::Display) {
        if self.diagnostic_failure.is_none() {
            self.diagnostic_failure = Some(repair44_bounded_diagnostic(format_args!("{error}")));
        }
    }
    #[cfg(test)]
    pub(super) fn loaded_for_test(&self) -> bool {
        self.jobs
            .values()
            .any(|job| matches!(&job.phase, Phase::Document { loaded: true, .. }))
    }
    #[cfg(all(test, feature = "in-process-probe"))]
    pub(super) fn diagnostic_for_test(&self) -> String {
        // Fixed-size counters and native identity only; no URLs, response bodies,
        // policy payloads or driver/V8 calls. Called only on fixture failure.
        let mut phases = [0usize; 8];
        for job in self.jobs.values() {
            match &job.phase {
                Phase::Reserve(_) => phases[0] += 1,
                Phase::Fetch(_) => phases[1] += 1,
                Phase::Failed => phases[2] += 1,
                Phase::Document {
                    ready,
                    ready_ack,
                    loaded,
                    ack,
                    next,
                    ..
                } => {
                    phases[3] += 1;
                    phases[4] += usize::from(*ready);
                    phases[5] += usize::from(ready_ack.is_some());
                    phases[6] += usize::from(*loaded);
                    phases[7] += usize::from(ack.is_some() || next.is_some());
                }
            }
        }
        format!(
            "source={:?} jobs={} phases[reserve,fetch,failed,document,ready,ready_ack,loaded,load_ack_or_next]={phases:?} retiring={} retry_pending={} interest_pending={} accepted_blank={} first_failure={:?}",
            self.source,
            self.jobs.len(),
            self.retiring.len(),
            self.retry.is_some(),
            self.interest_pending,
            self.accepted_blank,
            self.diagnostic_failure
        )
    }
    /// Consume the exact source record after native navigation admission. The
    /// selected driver leaves roster cancellation ownership before departure.
    pub fn matching_pending(
        &self,
        source: &crate::host::PrerenderSourceSnapshot,
        url: &str,
    ) -> bool {
        self.matching_target_pending(source, url, false)
    }
    #[cfg(test)]
    pub fn matching_blank_pending(
        &self,
        source: &crate::host::PrerenderSourceSnapshot,
        url: &str,
    ) -> bool {
        self.matching_target_pending(source, url, true)
    }
    fn matching_target_pending(
        &self,
        source: &crate::host::PrerenderSourceSnapshot,
        url: &str,
        blank: bool,
    ) -> bool {
        source.candidates.iter().any(|candidate| {
            let rule = &candidate.candidate;
            if !candidate.eligible || rule.url != url || is_blank(rule) != blank {
                return false;
            }
            self.source != Some((source.realm, source.identity))
                || self
                    .jobs
                    .get(&(candidate.rule_source, candidate.ordinal))
                    .is_none_or(|job| job.rule == *rule && !matches!(&job.phase, Phase::Failed))
        })
    }
    pub fn take_for_activation(
        &mut self,
        source: &crate::host::PrerenderSourceSnapshot,
        url: &str,
    ) -> Option<Box<CandidateDocument>> {
        self.take_for_target(source, url, false)
    }
    pub fn take_for_blank_activation(
        &mut self,
        source: &crate::host::PrerenderSourceSnapshot,
        url: &str,
    ) -> Option<Box<CandidateDocument>> {
        self.take_for_target(source, url, true)
    }
    fn take_for_target(
        &mut self,
        source: &crate::host::PrerenderSourceSnapshot,
        url: &str,
        blank: bool,
    ) -> Option<Box<CandidateDocument>> {
        if self.source != Some((source.realm, source.identity)) {
            return None;
        }
        let key=self.jobs.iter().find(|(key,job)|job.rule.url==url&&is_blank(&job.rule)==blank
            &&matches!(&job.phase,Phase::Document{document,ready:true,..} if document.activation_available())
            &&source.candidates.iter().any(|candidate|candidate.eligible&&candidate.rule_source==key.0&&candidate.ordinal==key.1&&candidate.candidate==job.rule)).map(|(key,_)|*key)?;
        let job = self.jobs.get_mut(&key)?;
        let Phase::Document { document, .. } = std::mem::replace(&mut job.phase, Phase::Failed)
        else {
            unreachable!("selected ready candidate")
        };
        Some(document)
    }
    /// Transfer an accepted blank job before source retirement can cancel it.
    pub fn detach_blank(
        &mut self,
        source: &mut crate::host::PrerenderSourceSnapshot,
        receipt: &crate::context_service::BlankPrerenderActivation,
    ) -> Option<Self> {
        if self.source != Some((source.realm, source.identity)) {
            return None;
        }
        let key = self
            .jobs
            .iter()
            .find(|(_, job)| {
                is_blank(&job.rule)
                    && job.rule.url == receipt.url
                    && !matches!(job.phase, Phase::Failed)
            })
            .map(|(key, _)| *key)?;
        let job = self.jobs.remove(&key)?;
        source
            .candidates
            .retain(|candidate| candidate.rule_source == key.0 && candidate.ordinal == key.1);
        Some(Self {
            source: self.source,
            jobs: BTreeMap::from([(key, job)]),
            accepted_blank: true,
            ..Self::default()
        })
    }
    pub fn delay(&self) -> Option<Duration> {
        if self
            .jobs
            .values()
            .all(|job| matches!(job.phase, Phase::Failed))
            && self.retiring.is_empty()
            && !self.interest_pending
        {
            return None;
        }
        Some(
            self.retry
                .map(|time| time.saturating_duration_since(Instant::now()))
                .unwrap_or(Duration::ZERO),
        )
    }
    fn dispose(&mut self, job: Job) {
        if let Phase::Document { document, .. } = job.phase {
            self.retiring
                .push_back(document.executable.agent_root_realm());
            drop(document);
        }
    }
    pub fn poll(&mut self, params: PrerenderPollInput<'_>) -> (u64, Vec<String>) {
        let PrerenderPollInput {
            snapshot,
            host,
            client,
            page,
            broker,
            source_url,
            policy,
            config,
        } = params;

        let mut diagnostics = Vec::new();
        let mut high = 0;
        // A source navigation in the same author turn cannot revoke a blank
        // open which the parent already accepted. The actor takes this job
        // immediately after the turn; retain its frozen source until then.
        let snapshot = self
            .source
            .and_then(|(realm, _)| host.as_ref()?.pending_blank_prerender_source(realm))
            .or(snapshot);

        let identity = snapshot
            .as_ref()
            .map(|source| (source.realm, source.identity));
        if identity != self.source {
            let jobs = std::mem::take(&mut self.jobs);
            for (_, job) in jobs {
                self.dispose(job)
            }
            self.source = identity;
            #[cfg(test)]
            {
                self.diagnostic_failure = None;
            }
        }
        self.interest_pending = snapshot.as_ref().is_some_and(|source| {
            source
                .candidates
                .iter()
                .any(|candidate| !candidate.eligible && candidate.eligibility_delay.is_some())
        });
        let wanted = snapshot
            .as_ref()
            .map(|source| {
                source
                    .candidates
                    .iter()
                    .map(|candidate| {
                        (
                            (candidate.rule_source, candidate.ordinal),
                            candidate.candidate.clone(),
                        )
                    })
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        let removed = self
            .jobs
            .iter()
            .filter(|(key, job)| wanted.get(key) != Some(&job.rule))
            .map(|(key, _)| *key)
            .collect::<Vec<_>>();
        for key in removed {
            if let Some(job) = self.jobs.remove(&key) {
                self.dispose(job)
            }
        }
        let Some(native) = host.as_mut() else {
            return (high, vec!["prerender actor host unavailable".into()]);
        };
        let count = self.retiring.len();
        for _ in 0..count {
            let realm = self.retiring.pop_front().unwrap();
            if let Err(error) = native.retire_agent_root(realm) {
                self.retiring.push_back(realm);
                #[cfg(test)]
                self.retain_diagnostic_failure(&error);
                diagnostics.push(error.to_string());
            }
        }
        let Some(source) = snapshot else {
            self.retry =
                (!self.retiring.is_empty()).then(|| Instant::now() + Duration::from_millis(25));
            return (high, diagnostics);
        };
        for (key, rule) in wanted {
            if self.jobs.contains_key(&key) || self.jobs.len() >= 16 {
                continue;
            }
            if !source.candidates.iter().any(|candidate| {
                candidate.rule_source == key.0 && candidate.ordinal == key.1 && candidate.eligible
            }) {
                continue;
            }
            let Some(deadline) = Instant::now().checked_add(config.navigation_timeout) else {
                diagnostics.push("prerender deadline overflow".into());
                continue;
            };
            let phase = match PendingReservation::start(
                client.clone(),
                page,
                source.creator.clone(),
                rule.url.clone(),
                rule.target,
                deadline,
            ) {
                Ok(pending) => Phase::Reserve(Box::new(pending)),
                Err(error) => {
                    #[cfg(test)]
                    self.retain_diagnostic_failure(&error);
                    diagnostics.push(error.to_string());
                    Phase::Failed
                }
            };
            self.jobs.insert(
                key,
                Job {
                    rule,
                    deadline,
                    phase,
                },
            );
        }
        let keys = self.jobs.keys().copied().collect::<Vec<_>>();
        for key in keys {
            // Every author turn can remove its originating rule or replace the
            // source. Validate again before lending the host to the next job.
            let current = if self.accepted_blank {
                Some(source.clone())
            } else {
                host.as_ref()
                    .and_then(|host| host.prerender_source_snapshot(source.realm).ok().flatten())
            };
            if current
                .as_ref()
                .is_none_or(|current| current.identity != source.identity)
            {
                break;
            }
            let Some(mut job) = self.jobs.remove(&key) else {
                continue;
            };
            if !current.unwrap().candidates.iter().any(|candidate| {
                candidate.rule_source == key.0
                    && candidate.ordinal == key.1
                    && candidate.candidate == job.rule
            }) {
                self.dispose(job);
                continue;
            }
            let phase = std::mem::replace(&mut job.phase, Phase::Failed);
            let document_realm = match &phase {
                Phase::Document { document, .. } => Some(document.executable.agent_root_realm()),
                _ => None,
            };
            let result: Result<Phase, RuntimeError> = (|| {
                Ok(match phase {
                    Phase::Reserve(mut pending) => match pending.poll() {
                        Poll::Pending => Phase::Reserve(pending),
                        Poll::Ready(Err(error)) => return Err(error),
                        Poll::Ready(Ok(reservation)) => {
                            if reservation.page_id == page {
                                high = high.max(reservation.generation);
                            }
                            let url = Url::parse(&job.rule.url).map_err(|error| {
                                RuntimeError::ProtocolViolation(error.to_string())
                            })?;
                            let selected_policy = job
                                .rule
                                .referrer_policy
                                .as_deref()
                                .and_then(crate::referrer::ReferrerPolicy::parse_attribute)
                                .unwrap_or(policy);
                            Phase::Fetch(Box::new(PendingCandidate::start(
                                reservation,
                                client.clone(),
                                page,
                                source.creator.clone(),
                                broker.clone(),
                                url,
                                crate::cookies::CookieRequestContext::navigation(Some(
                                    source_url.into(),
                                )),
                                Url::parse(source_url).ok(),
                                selected_policy,
                                config.max_redirects,
                                job.deadline,
                            )?))
                        }
                    },
                    Phase::Fetch(mut pending) => match pending.poll() {
                        Poll::Pending => Phase::Fetch(pending),
                        Poll::Ready(Err(error)) => return Err(error),
                        Poll::Ready(Ok(prepared)) => {
                            let mut provisional = None;
                            let (destination_client, destination_page) = prepared.endpoint();
                            let result =
                                prepared.construct(prerender_load::PrerenderConstructionInput {
                                    previous: source.realm,
                                    client: destination_client,
                                    page: destination_page,
                                    max_heap: config.max_v8_heap_bytes,
                                    deadline: job.deadline,
                                    host,
                                    provisional: &mut provisional,
                                });
                            if let Some(realm) = provisional {
                                self.retiring.push_back(realm)
                            }
                            let document = result?;
                            #[cfg(test)]
                            if FAIL_AFTER_CONSTRUCTION.with(|fail| fail.replace(false)) {
                                self.retiring
                                    .push_back(document.executable.agent_root_realm());
                                return Err(RuntimeError::ProtocolViolation(
                                    "injected candidate construction failure".into(),
                                ));
                            }
                            Phase::Document {
                                document,
                                ready: false,
                                ready_ack: None,
                                loaded: false,
                                ack: None,
                                next: None,
                            }
                        }
                    },
                    Phase::Document {
                        mut document,
                        mut ready,
                        mut ready_ack,
                        mut loaded,
                        mut ack,
                        mut next,
                    } => {
                        let realm = document.executable.agent_root_realm();
                        let live = host
                            .as_mut()
                            .ok_or(RuntimeError::EvaluationUnavailable)?
                            .prerender_root_current(realm)
                            .map_err(map_host_error)?;
                        if !live || (!loaded && Instant::now() >= job.deadline) {
                            self.retiring.push_back(realm);
                            return Err(RuntimeError::NavigationCancelled);
                        }
                        if !ready {
                            if ready_ack.is_none() {
                                ready_ack=Some(Box::new(navigation_fetch::PendingPolicy::start_prerender_response(document.reservation.client.clone(),crate::context_service::ContextServiceRequest::MarkPrerenderReady{page_id:document.reservation.page_id,token:document.reservation.token.clone(),generation:document.reservation.generation,epoch:document.reservation.epoch},Some(job.deadline))?))
                            }
                            match ready_ack.as_mut().unwrap().poll() {
                                Poll::Pending => {}
                                Poll::Ready(Ok(
                                    crate::context_service::ContextServiceResponse::PrerenderReady,
                                )) => {
                                    ready = true;
                                    ready_ack = None
                                }
                                Poll::Ready(result) => {
                                    return Err(result.err().unwrap_or_else(|| {
                                        RuntimeError::ProtocolViolation(
                                            "prerender readiness ACK mismatch".into(),
                                        )
                                    }));
                                }
                            }
                        }
                        // Loaded documents still own dynamic scripts, frame parsers and
                        // context-event continuations. The load ACK is a milestone,
                        // not permission to abandon their executable task driver.
                        if next.is_none_or(|next| Instant::now() >= next) {
                            if let Err(error) = document.poll_with_host(host) {
                                self.retiring.push_back(realm);
                                return Err(error);
                            }
                            next = Some(Instant::now() + Duration::from_millis(25));
                        }
                        if !loaded && document.load_complete() {
                            if ack.is_none() {
                                ack=Some(Box::new(navigation_fetch::PendingPolicy::start_prerender_response(document.reservation.client.clone(),crate::context_service::ContextServiceRequest::MarkPrerenderLoaded{page_id:document.reservation.page_id,token:document.reservation.token.clone(),generation:document.reservation.generation,epoch:document.reservation.epoch},Some(job.deadline))?))
                            }
                            match ack.as_mut().unwrap().poll() {
                                Poll::Pending => {}
                                Poll::Ready(Ok(
                                    crate::context_service::ContextServiceResponse::PrerenderLoaded,
                                )) => {
                                    loaded = true;
                                    ack = None
                                }
                                Poll::Ready(result) => {
                                    self.retiring.push_back(realm);
                                    return Err(result.err().unwrap_or_else(|| {
                                        RuntimeError::ProtocolViolation(
                                            "prerender load ACK mismatch".into(),
                                        )
                                    }));
                                }
                            }
                        }
                        Phase::Document {
                            document,
                            ready,
                            ready_ack,
                            loaded,
                            ack,
                            next,
                        }
                    }
                    Phase::Failed => Phase::Failed,
                })
            })();
            job.phase = match result {
                Ok(phase) => phase,
                Err(error) => {
                    if let Some(realm) = document_realm
                        && !self.retiring.contains(&realm)
                    {
                        self.retiring.push_back(realm)
                    }
                    #[cfg(test)]
                    self.retain_diagnostic_failure(&error);
                    diagnostics.push(error.to_string());
                    Phase::Failed
                }
            };
            self.jobs.insert(key, job);
        }
        if let Some(host) = host.as_mut() {
            // A failed construction/turn can revoke parent authority while
            // retaining its local root. Retire that root before returning the
            // shared host to any incumbent/global task poll.
            let count = self.retiring.len();
            for _ in 0..count {
                let realm = self.retiring.pop_front().unwrap();
                if let Err(error) = host.retire_agent_root(realm) {
                    self.retiring.push_back(realm);
                    #[cfg(test)]
                    self.retain_diagnostic_failure(&error);
                    diagnostics.push(error.to_string());
                }
            }
            if let Err(error) = host.activate_realm(source.realm) {
                #[cfg(test)]
                self.retain_diagnostic_failure(&error);
                diagnostics.push(error.to_string())
            }
        }
        self.retry = Some(Instant::now() + Duration::from_millis(25));
        (high, diagnostics)
    }
}

#[cfg(test)]
mod retained_phase_size_tests {
    #[test]
    fn phase_holds_custody_handles_instead_of_inline_document_payloads() {
        assert!(std::mem::size_of::<super::Phase>() <= 128);
        assert!(std::mem::size_of::<Result<super::Phase, super::RuntimeError>>() < 2048);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_roster_failure_is_bounded_and_later_errors_do_not_replace_it() {
        let mut roster = Roster::default();
        roster.retain_diagnostic_failure(&format!("first:{}", "é".repeat(4096)));
        let first = roster.diagnostic_failure.clone().unwrap();
        assert!(first.len() <= 1024);
        assert!(first.starts_with("first:"));
        assert!(first.ends_with(" [truncated]"));
        roster.retain_diagnostic_failure(&"later retry");
        assert_eq!(roster.diagnostic_failure.as_deref(), Some(first.as_str()));
    }
    #[test]
    fn failed_rule_tombstone_does_not_suppress_reused_rule_in_new_document() {
        let rule = oryn_js::speculation_rules::Candidate {
            url: "https://example.test/next".into(),
            eagerness: oryn_js::speculation_rules::Eagerness::Immediate,
            target: None,
            referrer_policy: None,
            anonymous_cross_origin: false,
            tag: None,
        };
        let mut source = crate::host::PrerenderSourceSnapshot {
            realm: oryn_js::realm::RealmId(1),
            identity: (
                oryn_dom::NodeId {
                    slot: 1,
                    generation: 1,
                },
                1,
                0,
                1,
            ),
            creator: oryn_js::worker_wire::Creator::Window {
                target: oryn_js::message_wire::WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 0,
            },
            candidates: vec![oryn_js::v8_bindings::NativePrerenderCandidate {
                rule_source: 1,
                ordinal: 0,
                candidate: rule.clone(),
                eligible: true,
                eligibility_delay: None,
            }],
        };
        let mut roster = Roster {
            source: Some((source.realm, source.identity)),
            ..Default::default()
        };
        roster.jobs.insert(
            (1, 0),
            Job {
                rule,
                deadline: Instant::now(),
                phase: Phase::Failed,
            },
        );
        assert!(!roster.matching_pending(&source, "https://example.test/next"));
        source.identity.2 += 1;
        assert!(roster.matching_pending(&source, "https://example.test/next"));
        source.candidates.clear();
        assert!(!roster.matching_pending(&source, "https://example.test/next"));
    }
    #[test]
    fn gesture_eligibility_and_target_hint_control_candidate_selection() {
        let rule = oryn_js::speculation_rules::Candidate {
            url: "https://example.test/next".into(),
            eagerness: oryn_js::speculation_rules::Eagerness::Conservative,
            target: Some(oryn_js::speculation_rules::TargetHint::Blank),
            referrer_policy: None,
            anonymous_cross_origin: false,
            tag: None,
        };
        let mut source = crate::host::PrerenderSourceSnapshot {
            realm: oryn_js::realm::RealmId(1),
            identity: (
                oryn_dom::NodeId {
                    slot: 1,
                    generation: 1,
                },
                1,
                0,
                1,
            ),
            creator: oryn_js::worker_wire::Creator::Window {
                target: oryn_js::message_wire::WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 0,
            },
            candidates: vec![oryn_js::v8_bindings::NativePrerenderCandidate {
                rule_source: 1,
                ordinal: 0,
                candidate: rule,
                eligible: false,
                eligibility_delay: None,
            }],
        };
        let roster = Roster {
            source: Some((source.realm, source.identity)),
            ..Default::default()
        };
        assert!(!roster.matching_pending(&source, "https://example.test/next"));
        assert!(!roster.matching_blank_pending(&source, "https://example.test/next"));
        source.candidates[0].eligible = true;
        assert!(roster.matching_blank_pending(&source, "https://example.test/next"));
        assert!(!roster.matching_pending(&source, "https://example.test/next"));
        source.candidates[0].candidate.target = None;
        assert!(!roster.matching_blank_pending(&source, "https://example.test/next"));
        assert!(roster.matching_pending(&source, "https://example.test/next"));
        source.candidates.clear();
        assert!(!roster.matching_pending(&source, "https://example.test/next"));
    }
    #[test]
    fn pending_hover_keeps_actor_wake_without_starting_dormant_candidates() {
        let mut roster = Roster::default();
        assert!(roster.delay().is_none());
        roster.interest_pending = true;
        roster.retry = Some(Instant::now());
        assert_eq!(roster.delay(), Some(Duration::ZERO));
        roster.interest_pending = false;
        assert!(roster.delay().is_none());
    }
    #[test]
    fn terminal_admission_records_are_quiescent_and_bounded_by_source_reconciliation() {
        let mut roster = Roster::default();
        assert!(roster.delay().is_none());
        let rule = oryn_js::speculation_rules::Candidate {
            url: "https://example.test/next".into(),
            eagerness: oryn_js::speculation_rules::Eagerness::Immediate,
            target: None,
            referrer_policy: None,
            anonymous_cross_origin: false,
            tag: None,
        };
        roster.jobs.insert(
            (1, 0),
            Job {
                rule,
                deadline: Instant::now(),
                phase: Phase::Failed,
            },
        );
        roster.retry = Some(Instant::now());
        assert!(
            roster.delay().is_none(),
            "a failed optional candidate must not spin the active actor"
        );
        roster.retiring.push_back(oryn_js::realm::RealmId(8));
        assert_eq!(roster.delay(), Some(Duration::ZERO));
        roster.retiring.clear();
        assert!(roster.delay().is_none());
    }
}
