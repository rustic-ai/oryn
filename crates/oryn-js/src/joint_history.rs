//! Native joint-session history steps. Route names are opaque native navigable
//! identities; importing a timeline never grants authority to a Document.
use crate::navigation_api::NavigationHistorySnapshot;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_STEPS: usize = 4096;
pub const MAX_ROUTES: usize = 1024;
pub const MAX_RETAINED_BYTES: usize = 64 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub step: u64,
    pub snapshot: NavigationHistorySnapshot,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Timeline {
    revision: u64,
    next_step: u64,
    current: u64,
    routes: BTreeMap<String, Vec<Entry>>,
    #[serde(default)]
    executions: BTreeMap<String, BTreeMap<String, PendingCommit>>,
    #[serde(default)]
    execution_origins: BTreeMap<String, (u64, u64, u64)>,
    #[serde(skip)]
    completion_leases: BTreeMap<String, NativeJointTraversalReceipt>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct PendingCommit {
    entry: String,
    requires_load: bool,
    committed: bool,
    #[serde(default)]
    published: bool,
    #[serde(default)]
    loaded: bool,
    #[serde(default)]
    interactive: bool,
}
#[derive(Clone, Debug)]
pub struct Traversal {
    pub revision: u64,
    pub from: u64,
    pub to: u64,
    pub destinations: Vec<(String, NavigationHistorySnapshot)>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JointTraversalFailure {
    Canceled,
    Retired,
    Failed(String),
}
impl std::fmt::Display for JointTraversalFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Canceled => f.write_str("joint traversal canceled"),
            Self::Retired => f.write_str("joint traversal retired"),
            Self::Failed(message) => f.write_str(message),
        }
    }
}
#[derive(Debug, Default)]
struct CompletionState {
    terminal: Option<Result<(), JointTraversalFailure>>,
    interactive: bool,
}
type Completion = std::sync::Mutex<CompletionState>;
fn completions() -> &'static std::sync::Mutex<BTreeMap<String, std::sync::Weak<Completion>>> {
    static REGISTRY: std::sync::OnceLock<
        std::sync::Mutex<BTreeMap<String, std::sync::Weak<Completion>>>,
    > = std::sync::OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}
/// Native receipt identity is separate from a history entry or Document id.
/// The weak registry bridges new isolates without retaining completed pages.
#[derive(Clone, Debug)]
pub struct NativeJointTraversalReceipt {
    token: String,
    completion: std::sync::Arc<Completion>,
}
impl NativeJointTraversalReceipt {
    pub(crate) fn token(&self) -> &str {
        &self.token
    }
    pub fn poll(&self) -> Option<Result<(), JointTraversalFailure>> {
        self.completion
            .lock()
            .map(|value| value.terminal.clone())
            .unwrap_or_else(|_| {
                Some(Err(JointTraversalFailure::Failed(
                    "joint completion lock poisoned".into(),
                )))
            })
    }
    pub fn poll_interactive(&self) -> Option<Result<(), JointTraversalFailure>> {
        self.completion
            .lock()
            .map(|value| {
                value
                    .terminal
                    .clone()
                    .or_else(|| value.interactive.then_some(Ok(())))
            })
            .unwrap_or_else(|_| {
                Some(Err(JointTraversalFailure::Failed(
                    "joint completion lock poisoned".into(),
                )))
            })
    }
    pub fn cancel(&self) {
        settle_completion(&self.token, Err(JointTraversalFailure::Canceled));
    }
    pub fn retire(&self) {
        settle_completion(&self.token, Err(JointTraversalFailure::Retired));
    }
}
fn settle_completion(token: &str, result: Result<(), JointTraversalFailure>) {
    let completion = completions()
        .lock()
        .ok()
        .and_then(|registry| registry.get(token).and_then(std::sync::Weak::upgrade));
    if let Some(completion) = completion {
        if let Ok(mut value) = completion.lock() {
            if value.terminal.is_none() {
                if result.is_ok() {
                    value.interactive = true;
                }
                value.terminal = Some(result);
            }
        }
    }
}
impl Timeline {
    pub fn begin_execution(
        &mut self,
        plan: &Traversal,
    ) -> Result<NativeJointTraversalReceipt, &'static str> {
        self.reconcile_executions();
        self.executions.retain(|token, _| {
            completions()
                .lock()
                .ok()
                .and_then(|registry| registry.get(token).and_then(std::sync::Weak::upgrade))
                .is_some_and(|completion| {
                    completion
                        .lock()
                        .is_ok_and(|state| state.terminal.is_none())
                })
        });
        if self.executions.len() >= 128 {
            return Err("joint traversal execution capacity");
        }
        let mut retained = self
            .retained_bytes()?
            .checked_add(128 + std::mem::size_of::<(u64, u64, u64)>())
            .ok_or("joint execution bytes")?;
        for (route, entry) in &plan.destinations {
            retained = retained
                .checked_add(route.len())
                .and_then(|bytes| bytes.checked_add(entry.id.len()))
                .and_then(|bytes| bytes.checked_add(std::mem::size_of::<PendingCommit>()))
                .ok_or("joint execution bytes")?;
        }
        if retained > MAX_RETAINED_BYTES {
            return Err("joint execution bytes");
        }
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| "joint traversal entropy")?;
        let token = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let completion = std::sync::Arc::new(std::sync::Mutex::new(CompletionState::default()));
        let mut registry = completions()
            .lock()
            .map_err(|_| "joint completion lock poisoned")?;
        registry.retain(|_, value| value.strong_count() != 0);
        if registry.len() >= 4096 || registry.contains_key(&token) {
            return Err("joint traversal receipt capacity");
        }
        registry.insert(token.clone(), std::sync::Arc::downgrade(&completion));
        drop(registry);
        let routes = plan
            .destinations
            .iter()
            .map(|(route, entry)| {
                (
                    route.clone(),
                    PendingCommit {
                        entry: entry.id.clone(),
                        requires_load: self.at(route, plan.from).is_none_or(|previous| {
                            previous.snapshot.document_id != entry.document_id
                        }),
                        committed: false,
                        published: false,
                        loaded: false,
                        interactive: false,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        if routes.is_empty() {
            settle_completion(&token, Ok(()));
        } else {
            self.execution_origins
                .insert(token.clone(), (plan.from, plan.to, self.next_revision()?));
            self.executions.insert(token.clone(), routes);
        }
        let receipt = NativeJointTraversalReceipt { token, completion };
        if self.executions.contains_key(receipt.token()) {
            self.completion_leases
                .insert(receipt.token.clone(), receipt.clone());
        }
        Ok(receipt)
    }
    pub fn rebind_execution_leases(&mut self) {
        if let Ok(registry) = completions().lock() {
            for token in self.executions.keys() {
                if let Some(completion) = registry.get(token).and_then(std::sync::Weak::upgrade) {
                    self.completion_leases.insert(
                        token.clone(),
                        NativeJointTraversalReceipt {
                            token: token.clone(),
                            completion,
                        },
                    );
                }
            }
        }
    }
    fn publish_interactive_milestones(&self) {
        for (token, remaining) in &self.executions {
            if remaining.values().all(|entry| {
                entry.committed
                    && if entry.requires_load {
                        entry.interactive || entry.loaded
                    } else {
                        entry.published
                    }
            }) {
                if let Some(completion) = completions()
                    .lock()
                    .ok()
                    .and_then(|registry| registry.get(token).and_then(std::sync::Weak::upgrade))
                {
                    if let Ok(mut value) = completion.lock() {
                        if value.terminal.is_none() {
                            value.interactive = true;
                        }
                    }
                }
            }
        }
    }
    fn release_finished_custody(&mut self) {
        self.publish_interactive_milestones();
        self.execution_origins
            .retain(|token, _| self.executions.contains_key(token));
        self.completion_leases
            .retain(|token, _| self.executions.contains_key(token));
    }
    pub fn record_commit(&mut self, route: &str, id: &str) {
        for (token, remaining) in &self.executions {
            if remaining
                .get(route)
                .is_some_and(|expected| expected.entry == id)
            {
                self.execution_origins.remove(token);
            }
        }
        self.executions.retain(|token, remaining| {
            if let Some(expected) = remaining
                .get_mut(route)
                .filter(|expected| expected.entry == id)
            {
                expected.committed = true;
                if expected.published && (!expected.requires_load || expected.loaded) {
                    remaining.remove(route);
                }
            }
            if remaining.is_empty() {
                settle_completion(token, Ok(()));
                false
            } else {
                true
            }
        });
        self.release_finished_custody();
    }
    pub fn record_publication(&mut self, route: &str, id: &str) {
        self.record_readiness(route, id, 0);
    }
    pub fn record_load(&mut self, route: &str, id: &str) {
        self.record_readiness(route, id, 2);
    }
    pub fn record_interactive(&mut self, route: &str, id: &str) {
        self.record_readiness(route, id, 1);
    }
    fn record_readiness(&mut self, route: &str, id: &str, milestone: u8) {
        self.executions.retain(|token, remaining| {
            if let Some(expected) = remaining
                .get_mut(route)
                .filter(|expected| expected.entry == id)
            {
                match milestone {
                    0 => expected.published = true,
                    1 => expected.interactive = true,
                    _ => {
                        expected.loaded = true;
                        expected.interactive = true;
                    }
                }
                if expected.committed
                    && expected.published
                    && (!expected.requires_load || expected.loaded)
                {
                    remaining.remove(route);
                }
            }
            if remaining.is_empty() {
                settle_completion(token, Ok(()));
                false
            } else {
                true
            }
        });
        self.release_finished_custody();
    }
    fn rollback_uncommitted(&mut self, token: &str) {
        if let Some((from, to, revision)) = self.execution_origins.remove(token) {
            if self.current == to && self.revision == revision && self.steps().contains(&from) {
                if let Ok(revision) = self.next_revision() {
                    self.current = from;
                    self.revision = revision;
                }
            }
        }
    }
    pub fn reconcile_executions(&mut self) {
        let terminal = self
            .executions
            .keys()
            .filter(|token| {
                completions()
                    .lock()
                    .ok()
                    .and_then(|registry| registry.get(*token).and_then(std::sync::Weak::upgrade))
                    .is_none_or(|completion| {
                        completion
                            .lock()
                            .is_ok_and(|state| state.terminal.is_some())
                    })
            })
            .cloned()
            .collect::<Vec<_>>();
        for token in terminal {
            self.rollback_uncommitted(&token);
            self.executions.remove(&token);
        }
        self.release_finished_custody();
    }
    pub fn fail_route(&mut self, route: &str, id: &str, failure: JointTraversalFailure) {
        let tokens = self
            .executions
            .iter()
            .filter(|(_, remaining)| {
                remaining
                    .get(route)
                    .is_some_and(|expected| expected.entry == id)
            })
            .map(|(token, _)| token.clone())
            .collect::<Vec<_>>();
        for token in tokens {
            self.fail_execution(&token, failure.clone());
        }
    }
    /// A replacement or forward-history truncation can dispose a destination
    /// while its parser/publication still owns a receipt. Retire that receipt
    /// now: a late commit must not satisfy an entry no longer in the timeline.
    fn retire_disposed_destinations(&mut self) {
        let disposed = self
            .executions
            .iter()
            .filter(|(_, remaining)| {
                remaining
                    .iter()
                    .any(|(route, pending)| !self.knows(route, &pending.entry))
            })
            .map(|(token, _)| token.clone())
            .collect::<Vec<_>>();
        for token in disposed {
            self.fail_execution(&token, JointTraversalFailure::Retired);
        }
    }
    pub fn cancel_executions(&mut self) {
        let tokens = self.executions.keys().cloned().collect::<Vec<_>>();
        for token in tokens {
            self.fail_execution(&token, JointTraversalFailure::Canceled);
        }
    }
    pub fn fail_execution(&mut self, token: &str, failure: JointTraversalFailure) {
        self.rollback_uncommitted(token);
        self.executions.remove(token);
        settle_completion(token, Err(failure));
        self.release_finished_custody();
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.retained_bytes()?;
        if self
            .execution_origins
            .keys()
            .any(|token| !self.executions.contains_key(token))
        {
            return Err("orphan joint execution origin");
        }
        if self.executions.len() > 128
            || self.executions.iter().any(|(token, routes)| {
                token.len() != 64
                    || !token.bytes().all(|byte| byte.is_ascii_hexdigit())
                    || routes.len() > MAX_ROUTES
            })
        {
            return Err("invalid joint execution metadata");
        }
        if self.executions.values().any(|routes| {
            routes.is_empty()
                || routes
                    .iter()
                    .any(|(route, pending)| !self.knows(route, &pending.entry))
        }) {
            return Err("orphan joint execution destination");
        }
        if self.routes.len() > MAX_ROUTES || self.steps().len() > MAX_STEPS {
            return Err("joint history capacity");
        }
        if self.current > self.next_step
            || !self.routes.is_empty() && !self.steps().contains(&self.current)
        {
            return Err("joint history cursor");
        }
        for (route, entries) in &self.routes {
            if route.is_empty() || entries.is_empty() || entries.len() > MAX_STEPS {
                return Err("joint history route");
            }
            let mut previous = None;
            let mut ids = BTreeSet::new();
            for entry in entries {
                if entry.step > self.next_step
                    || previous.is_some_and(|step| step >= entry.step)
                    || !ids.insert(&entry.snapshot.id)
                    || entry.snapshot.id.is_empty()
                    || entry.snapshot.key.is_empty()
                    || !entry.snapshot.scroll.valid()
                {
                    return Err("joint history entry");
                }
                previous = Some(entry.step);
            }
        }
        Ok(())
    }
    fn steps(&self) -> BTreeSet<u64> {
        self.routes
            .values()
            .flatten()
            .map(|entry| entry.step)
            .collect()
    }
    pub fn retained_bytes(&self) -> Result<usize, &'static str> {
        let mut bytes = std::mem::size_of::<Self>();
        for (route, entries) in &self.routes {
            bytes = bytes
                .checked_add(route.len())
                .ok_or("joint history bytes")?;
            for entry in entries {
                bytes = bytes
                    .checked_add(Self::snapshot_bytes(&entry.snapshot)?)
                    .ok_or("joint history bytes")?;
            }
        }
        for (token, routes) in &self.executions {
            bytes = bytes
                .checked_add(token.len())
                .ok_or("joint history bytes")?;
            for (route, id) in routes {
                bytes = bytes
                    .checked_add(route.len())
                    .and_then(|bytes| bytes.checked_add(id.entry.len()))
                    .and_then(|bytes| bytes.checked_add(std::mem::size_of::<PendingCommit>()))
                    .ok_or("joint history bytes")?;
            }
        }
        for token in self.execution_origins.keys() {
            bytes = bytes
                .checked_add(token.len())
                .and_then(|bytes| bytes.checked_add(std::mem::size_of::<(u64, u64, u64)>()))
                .ok_or("joint history bytes")?;
        }
        if bytes > MAX_RETAINED_BYTES {
            return Err("joint history bytes");
        }
        Ok(bytes)
    }
    fn snapshot_bytes(snapshot: &NavigationHistorySnapshot) -> Result<usize, &'static str> {
        let mut bytes = std::mem::size_of::<Entry>();
        for value in [
            &snapshot.key,
            &snapshot.id,
            &snapshot.url,
            &snapshot.document_id,
        ] {
            bytes = bytes
                .checked_add(value.len())
                .ok_or("joint history bytes")?;
        }
        for value in [&snapshot.state, &snapshot.history_state]
            .into_iter()
            .flatten()
        {
            bytes = bytes
                .checked_add(value.retained_bytes())
                .ok_or("joint history bytes")?;
        }
        Ok(bytes)
    }
    pub fn seed(
        &mut self,
        route: String,
        entries: Vec<NavigationHistorySnapshot>,
        current: usize,
    ) -> Result<(), &'static str> {
        if !self.is_empty()
            || entries.is_empty()
            || current >= entries.len()
            || entries.len() > MAX_STEPS
        {
            return Err("invalid initial joint history");
        }
        let revision = self.next_revision()?;
        let staged = Self {
            executions: BTreeMap::new(),
            execution_origins: BTreeMap::new(),
            completion_leases: BTreeMap::new(),
            revision,
            next_step: entries.len() as u64 - 1,
            current: current as u64,
            routes: [(
                route,
                entries
                    .into_iter()
                    .enumerate()
                    .map(|(step, snapshot)| Entry {
                        step: step as u64,
                        snapshot,
                    })
                    .collect(),
            )]
            .into_iter()
            .collect(),
        };
        staged.validate()?;
        *self = staged;
        Ok(())
    }
    pub fn select(&mut self, route: &str, id: &str) -> Result<bool, &'static str> {
        let Some(step) = self
            .routes
            .get(route)
            .and_then(|entries| entries.iter().find(|entry| entry.snapshot.id == id))
            .map(|entry| entry.step)
        else {
            return Ok(false);
        };
        let revision = self.next_revision()?;
        self.current = step;
        self.revision = revision;
        Ok(true)
    }
    pub fn knows(&self, route: &str, id: &str) -> bool {
        self.routes
            .get(route)
            .is_some_and(|entries| entries.iter().any(|entry| entry.snapshot.id == id))
    }
    pub fn route_count(&self) -> usize {
        self.routes.len()
    }
    pub fn len(&self) -> usize {
        self.steps().len()
    }
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }
    pub fn cursor(&self) -> u64 {
        self.current
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn selected(&self, route: &str) -> Option<&NavigationHistorySnapshot> {
        self.at(route, self.current).map(|entry| &entry.snapshot)
    }
    fn at(&self, route: &str, step: u64) -> Option<&Entry> {
        self.routes
            .get(route)?
            .iter()
            .rev()
            .find(|entry| entry.step <= step)
    }
    fn next_revision(&self) -> Result<u64, &'static str> {
        self.revision
            .checked_add(1)
            .ok_or("joint history revision exhausted")
    }
    /// Initial child documents join the current step. A child completing its
    /// initial load after its parent pushed an entry must replace its own
    /// existing initial entry, not invent another traversal step.
    pub fn attach(
        &mut self,
        route: String,
        snapshot: NavigationHistorySnapshot,
    ) -> Result<(), &'static str> {
        self.attach_at(route, snapshot, self.current)
    }
    pub fn attach_at(
        &mut self,
        route: String,
        snapshot: NavigationHistorySnapshot,
        step: u64,
    ) -> Result<(), &'static str> {
        if step > self.current {
            return Err("future initial joint step");
        }
        if self.routes.contains_key(&route) {
            return self.replace(&route, snapshot);
        }
        if route.is_empty() || self.routes.len() >= MAX_ROUTES {
            return Err("joint history routes exhausted");
        }
        Self::check_snapshot(&snapshot)?;
        let revision = self.next_revision()?;
        if self
            .retained_bytes()?
            .checked_add(route.len())
            .and_then(|bytes| bytes.checked_add(Self::snapshot_bytes(&snapshot).ok()?))
            .is_none_or(|bytes| bytes > MAX_RETAINED_BYTES)
        {
            return Err("joint history bytes");
        }
        self.routes.insert(route, vec![Entry { step, snapshot }]);
        self.revision = revision;
        Ok(())
    }
    fn check_snapshot(snapshot: &NavigationHistorySnapshot) -> Result<(), &'static str> {
        if snapshot.id.is_empty() || snapshot.key.is_empty() || !snapshot.scroll.valid() {
            Err("invalid joint history snapshot")
        } else {
            Ok(())
        }
    }
    pub fn replace(
        &mut self,
        route: &str,
        snapshot: NavigationHistorySnapshot,
    ) -> Result<(), &'static str> {
        Self::check_snapshot(&snapshot)?;
        let revision = self.next_revision()?;
        let current = self.current;
        let old = self
            .at(route, current)
            .ok_or("unselected joint history route")?;
        if self.routes[route]
            .iter()
            .any(|entry| entry.step != old.step && entry.snapshot.id == snapshot.id)
        {
            return Err("duplicate joint history id");
        }
        let retained = self
            .retained_bytes()?
            .saturating_sub(Self::snapshot_bytes(&old.snapshot)?);
        if retained
            .checked_add(Self::snapshot_bytes(&snapshot)?)
            .is_none_or(|bytes| bytes > MAX_RETAINED_BYTES)
        {
            return Err("joint history bytes");
        }
        let entries = self
            .routes
            .get_mut(route)
            .ok_or("unknown joint history route")?;
        let selected = entries
            .iter_mut()
            .rev()
            .find(|entry| entry.step <= current)
            .ok_or("unselected joint history route")?;
        selected.snapshot = snapshot;
        self.revision = revision;
        self.retire_disposed_destinations();
        Ok(())
    }
    pub fn push(
        &mut self,
        route: &str,
        snapshot: NavigationHistorySnapshot,
    ) -> Result<(), &'static str> {
        Self::check_snapshot(&snapshot)?;
        if self.at(route, self.current).is_none() {
            return Err("unselected joint history route");
        }
        if self.routes[route]
            .iter()
            .any(|entry| entry.step <= self.current && entry.snapshot.id == snapshot.id)
        {
            return Err("duplicate joint history id");
        }
        // Include retained execution receipts and rollback metadata as well as
        // entry snapshots. They survive a push when their destinations survive.
        let mut retained = self.retained_bytes()?;
        for (route, entries) in &self.routes {
            if !entries.iter().any(|entry| entry.step <= self.current) {
                retained = retained.saturating_sub(route.len());
            }
            for entry in entries.iter().filter(|entry| entry.step > self.current) {
                retained = retained.saturating_sub(Self::snapshot_bytes(&entry.snapshot)?);
            }
        }
        if retained
            .checked_add(Self::snapshot_bytes(&snapshot)?)
            .is_none_or(|bytes| bytes > MAX_RETAINED_BYTES)
        {
            return Err("joint history bytes");
        }
        let used = self
            .steps()
            .into_iter()
            .filter(|step| *step <= self.current)
            .count();
        if used >= MAX_STEPS {
            return Err("joint history steps exhausted");
        }
        let step = self
            .next_step
            .checked_add(1)
            .ok_or("joint history steps exhausted")?;
        let revision = self.next_revision()?;
        // Prepare capacity before truncating any route's forward entries.
        self.routes
            .get_mut(route)
            .unwrap()
            .try_reserve(1)
            .map_err(|_| "joint history allocation")?;
        for entries in self.routes.values_mut() {
            entries.retain(|entry| entry.step <= self.current);
        }
        self.routes.retain(|_, entries| !entries.is_empty());
        self.routes
            .get_mut(route)
            .ok_or("unselected joint history route")?
            .push(Entry { step, snapshot });
        self.current = step;
        self.next_step = step;
        self.revision = revision;
        self.retire_disposed_destinations();
        Ok(())
    }
    /// A native traversal freezes every affected destination before any
    /// author callback. Callers execute each route under its separate native
    /// lifetime authority; this value is selection evidence only.
    pub fn plan(&self, delta: i64) -> Result<Option<Traversal>, &'static str> {
        if delta == 0 {
            return Ok(None);
        }
        let steps: Vec<_> = self.steps().into_iter().collect();
        let Some(index) = steps.iter().position(|step| *step == self.current) else {
            return Err("joint history cursor missing");
        };
        let Some(next) = (index as i64)
            .checked_add(delta)
            .filter(|next| *next >= 0 && (*next as usize) < steps.len())
        else {
            return Ok(None);
        };
        let to = steps[next as usize];
        let mut destinations = Vec::new();
        destinations
            .try_reserve(self.routes.len())
            .map_err(|_| "joint traversal allocation")?;
        for route in self.routes.keys() {
            let destination = self.at(route, to);
            let previous = self.at(route, self.current);
            if let Some(destination) = destination
                .filter(|entry| previous.is_none_or(|old| old.snapshot.id != entry.snapshot.id))
            {
                destinations.push((route.clone(), destination.snapshot.clone()));
            }
        }
        Ok(Some(Traversal {
            revision: self.revision,
            from: self.current,
            to,
            destinations,
        }))
    }
    pub fn commit(&mut self, traversal: &Traversal) -> Result<bool, &'static str> {
        if self.revision != traversal.revision || self.current != traversal.from {
            return Ok(false);
        }
        if !self.steps().contains(&traversal.to) {
            return Ok(false);
        }
        let revision = self.next_revision()?;
        self.current = traversal.to;
        self.revision = revision;
        Ok(true)
    }
    pub fn discard_tree(&mut self, route: &str) -> Result<(), &'static str> {
        let prefix = format!("{route}/");
        let removed = self
            .routes
            .keys()
            .filter(|candidate| candidate.as_str() == route || candidate.starts_with(&prefix))
            .cloned()
            .collect::<BTreeSet<_>>();
        if removed.is_empty() {
            return Ok(());
        }
        let revision = self.next_revision()?;
        self.executions.retain(|token, entries| {
            if entries.keys().any(|candidate| removed.contains(candidate)) {
                settle_completion(token, Err(JointTraversalFailure::Retired));
                false
            } else {
                true
            }
        });
        self.release_finished_custody();
        self.routes
            .retain(|candidate, _| !removed.contains(candidate));
        let steps = self.steps();
        if !steps.contains(&self.current) {
            self.current = steps
                .range(..=self.current)
                .next_back()
                .copied()
                .or_else(|| steps.first().copied())
                .unwrap_or(0);
        }
        self.revision = revision;
        Ok(())
    }
    pub fn discard(&mut self, route: &str) -> Result<(), &'static str> {
        if !self.routes.contains_key(route) {
            return Ok(());
        }
        let revision = self.next_revision()?;
        self.routes.remove(route);
        let steps = self.steps();
        if !steps.contains(&self.current) {
            self.current = steps
                .range(..=self.current)
                .next_back()
                .copied()
                .or_else(|| steps.first().copied())
                .unwrap_or(0);
        }
        self.revision = revision;
        self.retire_disposed_destinations();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(id: &str) -> NavigationHistorySnapshot {
        NavigationHistorySnapshot {
            scroll: Default::default(),
            key: id.into(),
            id: id.into(),
            url: format!("https://example.test/#{id}"),
            document_id: "document".into(),
            state: None,
            history_state: None,
        }
    }
    #[test]
    fn one_traversal_selects_parent_and_child_steps() {
        let mut history = Timeline::default();
        history.attach("root".into(), entry("a")).unwrap();
        history.attach("child".into(), entry("b")).unwrap();
        history.push("root", entry("a1")).unwrap();
        history.push("child", entry("b1")).unwrap();
        assert_eq!(history.len(), 3);
        let plan = history.plan(-2).unwrap().unwrap();
        assert_eq!(plan.destinations.len(), 2);
        assert!(history.commit(&plan).unwrap());
        assert_eq!(history.selected("root").unwrap().id, "a");
        assert_eq!(history.selected("child").unwrap().id, "b");
    }
    #[test]
    fn initial_child_replacement_does_not_add_or_rebase_a_step() {
        let mut history = Timeline::default();
        history.attach("root".into(), entry("a")).unwrap();
        history.attach("child".into(), entry("blank")).unwrap();
        history.push("root", entry("a1")).unwrap();
        history.replace("child", entry("loaded")).unwrap();
        assert_eq!(history.len(), 2);
        let plan = history.plan(-1).unwrap().unwrap();
        assert_eq!(plan.destinations.len(), 1);
        assert_eq!(plan.destinations[0].0, "root");
    }
    #[test]
    fn push_truncates_forward_steps_across_routes_and_stale_plan_cannot_commit() {
        let mut history = Timeline::default();
        history.attach("root".into(), entry("a")).unwrap();
        history.attach("child".into(), entry("b")).unwrap();
        history.push("child", entry("b1")).unwrap();
        let old = history.plan(-1).unwrap().unwrap();
        history.replace("root", entry("changed")).unwrap();
        assert!(!history.commit(&old).unwrap());
        let back = history.plan(-1).unwrap().unwrap();
        assert!(history.commit(&back).unwrap());
        history.push("root", entry("a2")).unwrap();
        assert!(history.plan(1).unwrap().is_none());
        assert_eq!(history.selected("child").unwrap().id, "b");
        history.validate().unwrap();
    }
    #[test]
    fn rejected_seed_and_revision_exhaustion_leave_timeline_unchanged() {
        let mut history = Timeline::default();
        assert!(
            history
                .seed("root".into(), vec![entry("same"), entry("same")], 0)
                .is_err()
        );
        assert!(history.is_empty());
        history
            .seed("root".into(), vec![entry("a"), entry("b")], 1)
            .unwrap();
        history.revision = u64::MAX;
        assert!(history.select("root", "a").is_err());
        assert_eq!(history.cursor(), 1);
        assert_eq!(history.selected("root").unwrap().id, "b");
    }
    #[test]
    fn oversized_replacement_does_not_dispose_selected_entry() {
        let mut history = Timeline::default();
        history.attach("root".into(), entry("a")).unwrap();
        let revision = history.revision();
        let mut large = entry("b");
        large.url = "x".repeat(MAX_RETAINED_BYTES);
        assert!(history.replace("root", large).is_err());
        assert_eq!(history.selected("root").unwrap().id, "a");
        assert_eq!(history.revision(), revision);
    }

    #[test]
    fn native_receipt_waits_for_all_commits_across_serialized_timeline() {
        let mut history = Timeline::default();
        history.attach("root".into(), entry("a")).unwrap();
        history.attach("child".into(), entry("b")).unwrap();
        history.push("root", entry("a1")).unwrap();
        history.push("child", entry("b1")).unwrap();
        let plan = history.plan(-2).unwrap().unwrap();
        let receipt = history.begin_execution(&plan).unwrap();
        history.commit(&plan).unwrap();
        assert!(receipt.poll().is_none());
        history.record_commit("root", "a");
        history.record_publication("root", "a");
        assert!(receipt.poll().is_none());
        let encoded = serde_json::to_string(&history).unwrap();
        let mut restored: Timeline = serde_json::from_str(&encoded).unwrap();
        restored.validate().unwrap();
        restored.record_commit("child", "b");
        restored.record_publication("child", "b");
        assert_eq!(receipt.poll(), Some(Ok(())));
        receipt.cancel();
        assert_eq!(
            receipt.poll(),
            Some(Ok(())),
            "late cancellation cannot replace native completion"
        );
    }
    #[test]
    fn canceled_native_receipt_cannot_be_completed_by_late_commit() {
        let mut history = Timeline::default();
        history.attach("root".into(), entry("a")).unwrap();
        history.push("root", entry("b")).unwrap();
        let plan = history.plan(-1).unwrap().unwrap();
        let receipt = history.begin_execution(&plan).unwrap();
        history.commit(&plan).unwrap();
        receipt.cancel();
        history.record_commit("root", "a");
        history.record_publication("root", "a");
        assert_eq!(receipt.poll(), Some(Err(JointTraversalFailure::Canceled)));
    }

    #[test]
    fn cross_document_receipt_requires_load_after_commit() {
        let mut history = Timeline::default();
        history.attach("root".into(), entry("a")).unwrap();
        let mut other = entry("b");
        other.document_id = "other document".into();
        history.push("root", other).unwrap();
        let plan = history.plan(-1).unwrap().unwrap();
        let receipt = history.begin_execution(&plan).unwrap();
        history.commit(&plan).unwrap();
        history.record_commit("root", "a");
        history.record_publication("root", "a");
        assert!(receipt.poll().is_none());
        history.record_load("root", "different");
        assert!(receipt.poll().is_none());
        history.record_load("root", "a");
        assert_eq!(receipt.poll(), Some(Ok(())));
    }

    #[test]
    fn removal_prunes_child_steps_and_fails_only_owned_receipt() {
        let mut history = Timeline::default();
        history.attach("root".into(), entry("a")).unwrap();
        history.attach("root/doc/frame".into(), entry("b")).unwrap();
        history.push("root/doc/frame", entry("b1")).unwrap();
        let plan = history.plan(-1).unwrap().unwrap();
        let receipt = history.begin_execution(&plan).unwrap();
        history.commit(&plan).unwrap();
        history.discard_tree("root/doc/frame").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(receipt.poll(), Some(Err(JointTraversalFailure::Retired)));
        history.validate().unwrap();
    }
    #[test]
    fn canceled_uncommitted_plan_restores_cursor_but_keeps_author_changes() {
        let mut history = Timeline::default();
        history.attach("root".into(), entry("a")).unwrap();
        history.push("root", entry("b")).unwrap();
        let plan = history.plan(-1).unwrap().unwrap();
        let receipt = history.begin_execution(&plan).unwrap();
        history.commit(&plan).unwrap();
        receipt.cancel();
        history.reconcile_executions();
        assert_eq!(history.selected("root").unwrap().id, "b");
        let plan = history.plan(-1).unwrap().unwrap();
        let receipt = history.begin_execution(&plan).unwrap();
        history.commit(&plan).unwrap();
        history.push("root", entry("author")).unwrap();
        receipt.cancel();
        history.reconcile_executions();
        assert_eq!(history.selected("root").unwrap().id, "author");
    }
    #[test]
    fn pending_script_traversal_lease_survives_native_restore_without_ui_holder() {
        let mut history = Timeline::default();
        history.attach("root".into(), entry("a")).unwrap();
        let mut next = entry("b");
        next.document_id = "new".into();
        history.push("root", next).unwrap();
        let plan = history.plan(-1).unwrap().unwrap();
        let receipt = history.begin_execution(&plan).unwrap();
        let token = receipt.token.clone();
        history.commit(&plan).unwrap();
        drop(receipt);
        let encoded = serde_json::to_string(&history).unwrap();
        let mut restored: Timeline = serde_json::from_str(&encoded).unwrap();
        restored.rebind_execution_leases();
        let receipt = restored.completion_leases[&token].clone();
        drop(history);
        restored.reconcile_executions();
        assert!(receipt.poll().is_none());
        restored.record_commit("root", "a");
        restored.record_publication("root", "a");
        restored.record_load("root", "a");
        assert_eq!(receipt.poll(), Some(Ok(())));
    }

    #[test]
    fn load_and_commit_are_not_publication_completion() {
        let mut history = Timeline::default();
        history.attach("root".into(), entry("a")).unwrap();
        let mut next = entry("b");
        next.document_id = "other".into();
        history.push("root", next).unwrap();
        let plan = history.plan(-1).unwrap().unwrap();
        let receipt = history.begin_execution(&plan).unwrap();
        history.commit(&plan).unwrap();
        history.record_commit("root", "a");
        history.record_load("root", "a");
        assert!(receipt.poll().is_none());
        history.record_publication("root", "a");
        assert_eq!(receipt.poll(), Some(Ok(())));
    }

    #[test]
    fn interactive_waits_for_every_selected_document_and_survives_restore() {
        let mut history = Timeline::default();
        history.attach("root".into(), entry("a")).unwrap();
        history.attach("child".into(), entry("c")).unwrap();
        let mut b = entry("b");
        b.document_id = "other-root".into();
        history.push("root", b).unwrap();
        let mut d = entry("d");
        d.document_id = "other-child".into();
        history.push("child", d).unwrap();
        let plan = history.plan(-2).unwrap().unwrap();
        let receipt = history.begin_execution(&plan).unwrap();
        history.commit(&plan).unwrap();
        history.record_commit("root", "a");
        history.record_commit("child", "c");
        history.record_interactive("root", "a");
        assert!(receipt.poll_interactive().is_none());
        let encoded = serde_json::to_string(&history).unwrap();
        let mut restored: Timeline = serde_json::from_str(&encoded).unwrap();
        restored.rebind_execution_leases();
        drop(history);
        restored.record_interactive("child", "wrong");
        assert!(receipt.poll_interactive().is_none());
        restored.record_interactive("child", "c");
        assert_eq!(receipt.poll_interactive(), Some(Ok(())));
        assert!(receipt.poll().is_none());
        restored.record_publication("root", "a");
        restored.record_load("root", "a");
        assert!(receipt.poll().is_none());
        restored.record_publication("child", "c");
        restored.record_load("child", "c");
        assert_eq!(receipt.poll(), Some(Ok(())));
    }
    #[test]
    fn interactive_same_document_requires_publication_and_terminal_failure_remains_visible() {
        let mut history = Timeline::default();
        history.attach("root".into(), entry("a")).unwrap();
        history.push("root", entry("b")).unwrap();
        let plan = history.plan(-1).unwrap().unwrap();
        let receipt = history.begin_execution(&plan).unwrap();
        history.commit(&plan).unwrap();
        history.record_commit("root", "a");
        history.record_interactive("root", "a");
        assert!(receipt.poll_interactive().is_none());
        history.record_publication("root", "a");
        assert_eq!(receipt.poll_interactive(), Some(Ok(())));
        let mut next = entry("c");
        next.document_id = "new".into();
        history.push("root", next).unwrap();
        let plan = history.plan(-1).unwrap().unwrap();
        let receipt = history.begin_execution(&plan).unwrap();
        history.commit(&plan).unwrap();
        history.record_commit("root", "a");
        history.record_interactive("root", "a");
        assert_eq!(receipt.poll_interactive(), Some(Ok(())));
        receipt.cancel();
        assert_eq!(
            receipt.poll_interactive(),
            Some(Err(JointTraversalFailure::Canceled))
        );
        assert_eq!(receipt.poll(), Some(Err(JointTraversalFailure::Canceled)));
    }

    #[test]
    fn disposing_one_route_retires_only_its_receipt_and_late_publication_cannot_complete_it() {
        let mut history = Timeline::default();
        history.attach("root".into(), entry("a")).unwrap();
        history
            .attach("root/doc/child".into(), entry("child-a"))
            .unwrap();
        history.push("root", entry("b")).unwrap();
        let root_plan = history.plan(-1).unwrap().unwrap();
        let root_receipt = history.begin_execution(&root_plan).unwrap();
        history.push("root/doc/child", entry("child-b")).unwrap();
        let child_plan = history.plan(-1).unwrap().unwrap();
        let child_receipt = history.begin_execution(&child_plan).unwrap();
        history.discard("root/doc/child").unwrap();
        assert_eq!(
            child_receipt.poll(),
            Some(Err(JointTraversalFailure::Retired))
        );
        assert!(root_receipt.poll().is_none());
        history.record_commit("root/doc/child", "child-a");
        history.record_publication("root/doc/child", "child-a");
        assert_eq!(
            child_receipt.poll(),
            Some(Err(JointTraversalFailure::Retired))
        );
        history.record_commit("root", "a");
        history.record_publication("root", "a");
        assert_eq!(root_receipt.poll(), Some(Ok(())));
        history.validate().unwrap();
    }
    #[test]
    fn forward_truncation_retires_disposed_destination_without_rolling_back_author_push() {
        let mut history = Timeline::default();
        history.attach("root".into(), entry("a")).unwrap();
        history.push("root", entry("b")).unwrap();
        let back = history.plan(-1).unwrap().unwrap();
        history.commit(&back).unwrap();
        let forward = history.plan(1).unwrap().unwrap();
        let receipt = history.begin_execution(&forward).unwrap();
        history.push("root", entry("authored")).unwrap();
        assert_eq!(receipt.poll(), Some(Err(JointTraversalFailure::Retired)));
        assert_eq!(history.selected("root").unwrap().id, "authored");
        history.record_commit("root", "b");
        history.record_publication("root", "b");
        assert_eq!(receipt.poll(), Some(Err(JointTraversalFailure::Retired)));
        history.validate().unwrap();
    }
    #[test]
    fn replacement_disposes_old_destination_but_failed_replacement_preserves_receipt() {
        let mut history = Timeline::default();
        history.attach("root".into(), entry("a")).unwrap();
        history.push("root", entry("b")).unwrap();
        let back = history.plan(-1).unwrap().unwrap();
        let receipt = history.begin_execution(&back).unwrap();
        history.commit(&back).unwrap();
        let mut oversized = entry("oversized");
        oversized.url = "x".repeat(MAX_RETAINED_BYTES);
        assert!(history.replace("root", oversized).is_err());
        assert!(receipt.poll().is_none());
        history.replace("root", entry("replacement")).unwrap();
        assert_eq!(receipt.poll(), Some(Err(JointTraversalFailure::Retired)));
        assert_eq!(history.selected("root").unwrap().id, "replacement");
        history.validate().unwrap();
    }

    #[test]
    fn serialized_execution_rejects_missing_route_or_destination() {
        let mut history = Timeline::default();
        history.attach("root".into(), entry("a")).unwrap();
        history.push("root", entry("b")).unwrap();
        let plan = history.plan(-1).unwrap().unwrap();
        let receipt = history.begin_execution(&plan).unwrap();
        let wire = serde_json::to_value(&history).unwrap();
        for route in ["root", "missing"] {
            let mut malformed = wire.clone();
            let pending = malformed["executions"][receipt.token()]["root"].take();
            malformed["executions"][receipt.token()] =
                serde_json::Value::Object([(route.to_owned(), pending)].into_iter().collect());
            if route == "root" {
                malformed["executions"][receipt.token()]["root"]["entry"] =
                    serde_json::json!("missing-entry");
            }
            let restored: Timeline = serde_json::from_value(malformed).unwrap();
            assert_eq!(
                restored.validate(),
                Err("orphan joint execution destination")
            );
        }
        assert!(
            receipt.poll().is_none(),
            "invalid import does not alter live completion authority"
        );
        history.validate().unwrap();
    }

    #[test]
    fn push_budget_includes_retained_execution_metadata_before_mutation() {
        let mut history = Timeline::default();
        history.attach("root".into(), entry("a")).unwrap();
        history.push("root", entry("b")).unwrap();
        let plan = history.plan(-1).unwrap().unwrap();
        let receipt = history.begin_execution(&plan).unwrap();
        let revision = history.revision();
        let mut next = entry("large");
        next.url.clear();
        next.url = "x".repeat(
            MAX_RETAINED_BYTES
                - history.retained_bytes().unwrap()
                - Timeline::snapshot_bytes(&next).unwrap()
                + 1,
        );
        assert_eq!(history.push("root", next), Err("joint history bytes"));
        assert_eq!(history.revision(), revision);
        assert_eq!(history.selected("root").unwrap().id, "b");
        assert!(receipt.poll().is_none());
        history.validate().unwrap();
    }
}
