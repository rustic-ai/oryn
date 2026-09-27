//! Diagnostic-only correlation. No field is used for admission or authorization.
use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
const MAX_GROUPS: usize = 256;
const MAX_OPERATIONS: usize = 64;
const MAX_WAITING: usize = 1024;
const SEGMENT_CALLS: u64 = 4096;
const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_FRAME: usize = 32 * 1024;
const MAX_RECORDS: u64 = 4096;
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Command {
    Goto,
    GotoResource,
    ControlledNavigation,
    Execute,
    ExecuteBatch,
    Evaluate,
    Close,
    Oil,
    Initialize,
    Observe,
    ObserveWith,
    ObserveSemantic,
    ObserveSemanticV6,
    ObserveDelta,
    ObserveSemanticDelta,
    ResolveTarget,
    Other,
}
impl Command {
    fn from_label(label: &str) -> Self {
        match label {
            "goto" => Self::Goto,
            "goto_resource" => Self::GotoResource,
            "controlled_navigation" => Self::ControlledNavigation,
            "execute" => Self::Execute,
            "execute_batch" => Self::ExecuteBatch,
            "evaluate" => Self::Evaluate,
            "close" => Self::Close,
            "oil" => Self::Oil,
            "initialize" => Self::Initialize,
            "observe" => Self::Observe,
            "observe_with" => Self::ObserveWith,
            "observe_semantic" => Self::ObserveSemantic,
            "observe_semantic_v6" => Self::ObserveSemanticV6,
            "observe_delta" => Self::ObserveDelta,
            "observe_semantic_delta" => Self::ObserveSemanticDelta,
            "resolve_target" => Self::ResolveTarget,
            _ => Self::Other,
        }
    }
}
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(deny_unknown_fields)]
pub(crate) struct Stamp {
    process_session: u64,
    page_id: u64,
    route_generation: u64,
    message_id: u64,
    command: Command,
}
impl Stamp {
    pub(super) fn from_identity(id: Identity) -> Option<Self> {
        Some(Self {
            process_session: id.process_session,
            page_id: id.page_id,
            route_generation: id.route_generation,
            message_id: id.message_id?,
            command: Command::from_label(id.command?),
        })
    }
    fn matches(&self, route: &crate::worker_route::WorkerRoute) -> bool {
        self.process_session == route.process_session
            && self.page_id == route.environment
            && self.route_generation == route.generation
    }
}
// Version2 records are DELTAS. Sum distinct sequence numbers, never treat them as
// cumulative snapshots. In-flight counts are gauges and must not be summed.
type GroupKey = (u64, Stamp);
type ReceiptKey = (u64, u64, u64, u64, u64);
#[derive(Clone, Copy, Default, serde::Serialize)]
struct Totals {
    calls: u64,
    departed: u64,
    failed: u64,
    stages_ns: [u128; 6],
    receipts: u64,
    write_to_receipt_ns: u128,
    abandoned_receipts: u64,
}
#[derive(Default)]
struct Group {
    rows: BTreeMap<&'static str, Totals>,
    dispatches: u64,
    receipts: u64,
    calls_since_flush: u64,
    sealed: bool,
}
struct Waiting {
    group: GroupKey,
    operation: &'static str,
    started: Instant,
}
#[derive(Default)]
struct Store {
    groups: BTreeMap<GroupKey, Group>,
    waiting: BTreeMap<ReceiptKey, Waiting>,
    sequence: u64,
    bytes: usize,
    exhausted: bool,
    #[cfg(test)]
    emitted: Vec<Vec<u8>>,
}
#[derive(serde::Serialize)]
struct Event<'a> {
    kind: &'static str,
    version: u8,
    pid: u32,
    sequence: u64,
    channel: u64,
    identity: Option<Stamp>,
    phase: &'static str,
    aggregation: &'static str,
    incomplete: bool,
    pending_dispatches: u64,
    pending_receipts: u64,
    operations: &'a BTreeMap<&'static str, Totals>,
}
impl Store {
    // Output under this mutex preserves segment sequence order. Regular-file
    // one-write sink only; this never takes a native broker/authority lock.
    fn output(
        &mut self,
        key: Option<GroupKey>,
        phase: &'static str,
        incomplete: bool,
        dispatches: u64,
        receipts: u64,
        rows: &BTreeMap<&'static str, Totals>,
    ) {
        if self.exhausted {
            return;
        }
        let encode = |sequence, phase, incomplete, rows| {
            serde_json::to_vec(&Event {
                kind: "oryn_context_parent_summary",
                version: 2,
                pid: std::process::id(),
                sequence,
                channel: key.map(|k| k.0).unwrap_or(0),
                identity: key.map(|k| k.1),
                phase,
                aggregation: "delta",
                incomplete,
                pending_dispatches: dispatches,
                pending_receipts: receipts,
                operations: rows,
            })
        };
        let mut bytes = encode(self.sequence, phase, incomplete, rows).ok();
        let overflow = self.sequence >= MAX_RECORDS
            || bytes.as_ref().is_none_or(|b| {
                b.len() + 1 > MAX_FRAME || self.bytes.saturating_add(b.len() + 1) > MAX_BYTES - 1024
            });
        if overflow {
            self.exhausted = true;
            bytes = encode(self.sequence, "truncated", true, &BTreeMap::new()).ok();
        }
        self.sequence = self.sequence.saturating_add(1);
        if let Some(mut bytes) = bytes {
            bytes.push(b'\n');
            self.bytes = self.bytes.saturating_add(bytes.len());
            #[cfg(test)]
            self.emitted.push(bytes.clone());
            if regular_file(libc::STDERR_FILENO) {
                let _ = write_once(libc::STDERR_FILENO, &bytes);
            }
            crate::native_timing::aggregate(&bytes);
        }
    }
    fn overflow(&mut self, key: GroupKey, reason: &'static str) {
        if self.exhausted {
            return;
        }
        // Emit existing deltas first where the remaining output budget allows.
        let keys = self.groups.keys().copied().collect::<Vec<_>>();
        for key in keys {
            self.flush(key, "capacity_flush", true);
        }
        self.output(Some(key), reason, true, 0, 0, &BTreeMap::new());
        self.exhausted = true;
    }
    fn flush(&mut self, key: GroupKey, reason: &'static str, incomplete: bool) {
        let Some(group) = self.groups.get_mut(&key) else {
            return;
        };
        let rows = std::mem::take(&mut group.rows);
        group.calls_since_flush = 0;
        // Abandonment may arrive after the reader's first close segment, or on
        // a failed write before a later command transition. Every emitting path
        // must carry this evidence, not only the transport-close call site.
        let incomplete = incomplete || rows.values().any(|row| row.abandoned_receipts != 0);
        let (dispatches, receipts) = (group.dispatches, group.receipts);
        if !rows.is_empty() {
            self.output(Some(key), reason, incomplete, dispatches, receipts, &rows);
        }
        if dispatches == 0 && receipts == 0 {
            self.groups.remove(&key);
        }
    }
    fn ensure_row(&mut self, key: GroupKey, operation: &'static str) -> bool {
        if self.exhausted {
            return false;
        }
        if !self.groups.contains_key(&key) && self.groups.len() >= MAX_GROUPS {
            self.overflow(key, "group_capacity_reached");
            return false;
        }
        let group = self.groups.entry(key).or_default();
        if !group.rows.contains_key(operation) && group.rows.len() >= MAX_OPERATIONS {
            self.overflow(key, "operation_capacity_reached");
            return false;
        }
        self.groups
            .get_mut(&key)
            .unwrap()
            .rows
            .entry(operation)
            .or_default();
        true
    }
    fn admit(&mut self, key: GroupKey, operation: &'static str) -> bool {
        if self.exhausted {
            return false;
        }
        // A later command cannot steal an earlier command's receipts/counters.
        let prior = self
            .groups
            .keys()
            .copied()
            .filter(|old| {
                old.0 == key.0
                    && old.1 != key.1
                    && old.1.process_session == key.1.process_session
                    && old.1.page_id == key.1.page_id
                    && old.1.route_generation == key.1.route_generation
            })
            .collect::<Vec<_>>();
        for old in prior {
            self.groups.get_mut(&old).unwrap().sealed = true;
            self.flush(old, "command_transition", false);
        }
        if !self.ensure_row(key, operation) {
            return false;
        }
        let group = self.groups.get_mut(&key).unwrap();
        group.rows.get_mut(operation).unwrap().calls += 1;
        group.dispatches += 1;
        group.calls_since_flush += 1;
        if group.calls_since_flush >= SEGMENT_CALLS {
            self.flush(key, "rolling_segment", false);
        }
        true
    }
    fn finish(
        &mut self,
        key: GroupKey,
        operation: &'static str,
        times: [u128; 6],
        success: bool,
        closed: bool,
    ) {
        if !self.ensure_row(key, operation) {
            return;
        }
        let group = self.groups.get_mut(&key).unwrap();
        group.dispatches = group.dispatches.saturating_sub(1);
        let row = group.rows.get_mut(operation).unwrap();
        row.departed += 1;
        row.failed += u64::from(!success);
        for (sum, n) in row.stages_ns.iter_mut().zip(times) {
            *sum = sum.saturating_add(n);
        }
        if closed || (group.sealed && group.dispatches == 0 && group.receipts == 0) {
            self.flush(key, "settled_segment", false);
        }
    }
    fn await_receipt(
        &mut self,
        key: GroupKey,
        operation: &'static str,
        message: u64,
        closed: bool,
    ) {
        if self.exhausted {
            return;
        }
        if self.waiting.len() >= MAX_WAITING {
            self.overflow(key, "receipt_capacity_reached");
            return;
        }
        if !self.ensure_row(key, operation) {
            return;
        }
        if closed {
            self.groups
                .get_mut(&key)
                .unwrap()
                .rows
                .get_mut(operation)
                .unwrap()
                .abandoned_receipts += 1;
            return;
        }
        self.waiting.insert(
            receipt_key(key.0, key.1, message),
            Waiting {
                group: key,
                operation,
                started: Instant::now(),
            },
        );
        self.groups.get_mut(&key).unwrap().receipts += 1;
    }
    fn received(&mut self, key: ReceiptKey, now: Instant) {
        let Some(pending) = self.waiting.remove(&key) else {
            return;
        };
        if !self.ensure_row(pending.group, pending.operation) {
            return;
        }
        let group = self.groups.get_mut(&pending.group).unwrap();
        group.receipts = group.receipts.saturating_sub(1);
        let row = group.rows.get_mut(pending.operation).unwrap();
        row.receipts += 1;
        row.write_to_receipt_ns = row
            .write_to_receipt_ns
            .saturating_add(now.saturating_duration_since(pending.started).as_nanos());
        if group.sealed && group.dispatches == 0 && group.receipts == 0 {
            self.flush(pending.group, "settled_segment", false);
        }
    }
    fn abandon(&mut self, key: ReceiptKey) {
        let Some(p) = self.waiting.remove(&key) else {
            return;
        };
        if !self.ensure_row(p.group, p.operation) {
            return;
        }
        let g = self.groups.get_mut(&p.group).unwrap();
        g.receipts = g.receipts.saturating_sub(1);
        g.rows.get_mut(p.operation).unwrap().abandoned_receipts += 1;
    }
    fn close(&mut self, channel: u64) {
        let pending = self
            .waiting
            .keys()
            .copied()
            .filter(|k| k.0 == channel)
            .collect::<Vec<_>>();
        for key in pending {
            self.abandon(key);
        }
        let groups = self
            .groups
            .keys()
            .copied()
            .filter(|k| k.0 == channel)
            .collect::<Vec<_>>();
        for key in groups {
            self.groups.get_mut(&key).unwrap().sealed = true;
            let incomplete = self.groups[&key]
                .rows
                .values()
                .any(|r| r.abandoned_receipts != 0);
            self.flush(key, "transport_closed", incomplete);
        }
    }
}
fn store() -> &'static Mutex<Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    STORE.get_or_init(Default::default)
}
struct Channel {
    id: u64,
    closed: AtomicBool,
}
/// Only the reader owns this guard. Dispatch tasks own its state, not the guard;
/// close flushes buffered deltas, and late dispatch exits publish their own segment.
pub(crate) struct Diagnostics(Arc<Channel>);
impl Diagnostics {
    pub(crate) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(Arc::new(Channel {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            closed: AtomicBool::new(false),
        }))
    }
}
impl Drop for Diagnostics {
    fn drop(&mut self) {
        self.0.closed.store(true, Ordering::Release);
        store()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .close(self.0.id);
    }
}
fn receipt_key(channel: u64, stamp: Stamp, message: u64) -> ReceiptKey {
    (
        channel,
        stamp.process_session,
        stamp.page_id,
        stamp.route_generation,
        message,
    )
}
pub(crate) fn receipt(
    diagnostics: &Diagnostics,
    route: Option<&crate::worker_route::WorkerRoute>,
    message: u64,
) {
    let Some(route) = route else {
        return;
    };
    store().lock().unwrap_or_else(|e| e.into_inner()).received(
        (
            diagnostics.0.id,
            route.process_session,
            route.environment,
            route.generation,
            message,
        ),
        Instant::now(),
    );
}
struct Pending {
    key: GroupKey,
    message: u64,
    operation: &'static str,
    channel: Arc<Channel>,
    boundary: Instant,
    stage: usize,
    times: [u128; 6],
    success: bool,
}
pub(crate) struct Scope(Option<Pending>);
impl Scope {
    pub(crate) fn start(
        diagnostics: &Diagnostics,
        stamp: Option<Stamp>,
        route: Option<&crate::worker_route::WorkerRoute>,
        message: u64,
        operation: &'static str,
        decode_started: Instant,
    ) -> Self {
        if !regular_file(libc::STDERR_FILENO) && !crate::native_timing::enabled() {
            return Self(None);
        }
        let Some(stamp) = stamp.filter(|s| route.is_some_and(|r| s.matches(r))) else {
            return Self(None);
        };
        let key = (diagnostics.0.id, stamp);
        if !store()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .admit(key, operation)
        {
            return Self(None);
        }
        let now = Instant::now();
        Self(Some(Pending {
            key,
            message,
            operation,
            channel: diagnostics.0.clone(),
            boundary: now,
            stage: 1,
            times: [now.duration_since(decode_started).as_nanos(), 0, 0, 0, 0, 0],
            success: false,
        }))
    }
    pub(crate) fn advance(&mut self) {
        if let Some(p) = &mut self.0 {
            let now = Instant::now();
            p.times[p.stage] =
                p.times[p.stage].saturating_add(now.duration_since(p.boundary).as_nanos());
            p.stage = (p.stage + 1).min(5);
            p.boundary = now;
        }
    }
    pub(crate) fn await_receipt(&mut self) {
        let Some(p) = &self.0 else {
            return;
        };
        let mut s = store().lock().unwrap_or_else(|e| e.into_inner());
        // Reader closure prevents any later validated receipt. Record abandonment,
        // not an outstanding entry which nobody can ever remove.
        s.await_receipt(
            p.key,
            p.operation,
            p.message,
            p.channel.closed.load(Ordering::Acquire),
        );
    }
    pub(crate) fn finished(&mut self, success: bool) {
        if let Some(p) = &mut self.0 {
            p.success = success;
        }
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        let Some(mut p) = self.0.take() else {
            return;
        };
        p.times[p.stage] = p.times[p.stage].saturating_add(p.boundary.elapsed().as_nanos());
        let mut s = store().lock().unwrap_or_else(|e| e.into_inner());
        if !p.success {
            s.abandon(receipt_key(p.key.0, p.key.1, p.message));
        }
        s.finish(
            p.key,
            p.operation,
            p.times,
            p.success,
            p.channel.closed.load(Ordering::Acquire),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stamp(page: u64, message: u64) -> Stamp {
        Stamp {
            process_session: 1,
            page_id: page,
            route_generation: 3,
            message_id: message,
            command: Command::Execute,
        }
    }
    fn events(s: &Store) -> Vec<serde_json::Value> {
        s.emitted
            .iter()
            .map(|b| serde_json::from_slice(b).unwrap())
            .collect()
    }
    fn total(s: &Store, field: &str) -> u64 {
        events(s)
            .iter()
            .flat_map(|e| e["operations"].as_object().unwrap().values())
            .map(|r| r[field].as_u64().unwrap())
            .sum()
    }
    #[test]
    fn ipc_parent_totals_count_unsampled_calls_without_aliasing_commands_or_subtypes() {
        let mut s = Store::default();
        let key = (1, stamp(2, 4));
        for n in 0..9 {
            assert!(s.admit(key, "WindowState"));
            s.finish(key, "WindowState", [1, 2, 3, 4, 5, 6], n != 8, false);
        }
        assert!(s.admit(key, "CspEvents"));
        s.finish(key, "CspEvents", [0; 6], true, false);
        s.close(1);
        assert_eq!(total(&s, "calls"), 10);
        assert_eq!(total(&s, "departed"), 10);
        assert_eq!(total(&s, "failed"), 1);
        assert_eq!(
            events(&s)[0]["operations"]["WindowState"]["stages_ns"],
            serde_json::json!([9, 18, 27, 36, 45, 54])
        );
    }
    #[test]
    fn ipc_parent_stamp_is_bounded_and_not_an_authority_payload() {
        let text = serde_json::to_string(&stamp(2, 4)).unwrap();
        assert!(!text.contains("grant"));
        assert!(
            serde_json::from_str::<Stamp>(&text.replace("execute", "arbitrary_payload")).is_err()
        );
        let mut s = Store::default();
        for page in 0..MAX_GROUPS {
            assert!(s.admit((1, stamp(page as u64, 1)), "Read"));
        }
        assert!(!s.admit((1, stamp(9999, 1)), "Read"));
        assert!(s.exhausted);
        assert!(events(&s).last().unwrap()["incomplete"].as_bool().unwrap());
        assert!(s.groups.len() <= MAX_GROUPS);
    }
    #[test]
    fn ipc_parent_route_mismatch_does_not_acquire_correlation() {
        let route = crate::worker_route::WorkerRoute::issue(1, 2, 3).unwrap();
        assert!(stamp(2, 4).matches(&route));
        assert!(!stamp(9, 4).matches(&route));
        assert_ne!(
            receipt_key(1, stamp(2, 4), 8),
            receipt_key(2, stamp(2, 4), 8)
        );
    }
    #[test]
    fn ipc_parent_receipt_duration_is_distinct_from_response_departure() {
        let mut s = Store::default();
        let key = (1, stamp(2, 4));
        s.admit(key, "Read");
        s.finish(key, "Read", [1; 6], true, false);
        s.close(1);
        assert_eq!(total(&s, "departed"), 1);
        assert_eq!(total(&s, "receipts"), 0);
    }
    #[test]
    fn ipc_parent_late_execute_survives_twenty_context_startup() {
        let mut s = Store::default();
        for message in 1..=10 {
            for page in 1..=20 {
                let key = (page, stamp(page, message));
                for _ in 0..16 {
                    for operation in [
                        "WindowState",
                        "CspEvents",
                        "StorageEvents",
                        "WindowPosition",
                    ] {
                        assert!(s.admit(key, operation));
                        s.finish(key, operation, [1; 6], true, false);
                    }
                }
            }
        }
        for channel in 1..=20 {
            s.close(channel);
        }
        assert!(!s.exhausted);
        let rows = events(&s);
        assert!(rows.len() < MAX_RECORDS as usize);
        for page in 1..=20 {
            assert!(
                rows.iter()
                    .any(|e| e["identity"]["page_id"] == page && e["identity"]["message_id"] == 10)
            );
        }
        assert_eq!(total(&s, "calls"), 20 * 10 * 16 * 4);
        assert_eq!(total(&s, "departed"), 20 * 10 * 16 * 4);
    }
    #[test]
    fn ipc_parent_rolling_segments_preserve_inflight_delta_without_double_counting() {
        let mut s = Store::default();
        let key = (1, stamp(2, 10));
        for _ in 0..=SEGMENT_CALLS {
            assert!(s.admit(key, "WindowState"));
            s.finish(key, "WindowState", [1; 6], true, false);
        }
        assert!(!s.emitted.is_empty());
        s.close(1);
        assert_eq!(total(&s, "calls"), SEGMENT_CALLS + 1);
        assert_eq!(total(&s, "departed"), SEGMENT_CALLS + 1);
        assert_eq!(events(&s)[0]["pending_dispatches"], 1);
        assert!(s.groups.is_empty());
    }
    #[test]
    fn ipc_parent_late_receipt_stays_with_original_command() {
        let mut s = Store::default();
        let old = (1, stamp(2, 4));
        let new = (1, stamp(2, 5));
        let receipt = receipt_key(1, old.1, 33);
        s.admit(old, "Read");
        s.groups.get_mut(&old).unwrap().receipts = 1;
        s.waiting.insert(
            receipt,
            Waiting {
                group: old,
                operation: "Read",
                started: Instant::now(),
            },
        );
        s.finish(old, "Read", [1; 6], true, false);
        s.admit(new, "Other");
        assert!(s.groups.contains_key(&old));
        s.received(receipt, Instant::now());
        s.received(receipt, Instant::now());
        s.finish(new, "Other", [1; 6], true, false);
        s.close(1);
        assert_eq!(total(&s, "receipts"), 1);
        assert!(
            events(&s)
                .iter()
                .filter(|e| e["identity"]["message_id"] == 5)
                .all(|e| e["operations"]
                    .as_object()
                    .unwrap()
                    .values()
                    .all(|v| v["receipts"] == 0))
        );
    }
    #[test]
    fn ipc_parent_transport_close_preserves_pending_and_late_departure() {
        let mut s = Store::default();
        let key = (1, stamp(2, 4));
        s.admit(key, "Read");
        s.groups.get_mut(&key).unwrap().receipts = 1;
        s.waiting.insert(
            receipt_key(1, key.1, 7),
            Waiting {
                group: key,
                operation: "Read",
                started: Instant::now(),
            },
        );
        s.close(1);
        assert_eq!(events(&s)[0]["pending_dispatches"], 1);
        assert_eq!(total(&s, "abandoned_receipts"), 1);
        s.finish(key, "Read", [1; 6], false, true);
        assert_eq!(total(&s, "departed"), 1);
        assert_eq!(total(&s, "failed"), 1);
        assert!(s.waiting.is_empty());
        assert!(s.groups.is_empty());
    }
    #[test]
    fn ipc_parent_record_and_byte_caps_emit_one_explicit_incomplete_terminal() {
        for byte_cap in [false, true] {
            let mut s = Store::default();
            if byte_cap {
                s.bytes = MAX_BYTES - 1024;
            } else {
                s.sequence = MAX_RECORDS;
            }
            s.output(
                Some((1, stamp(2, 4))),
                "segment",
                false,
                0,
                0,
                &BTreeMap::new(),
            );
            s.output(None, "ignored", false, 0, 0, &BTreeMap::new());
            assert_eq!(s.emitted.len(), 1);
            assert_eq!(events(&s)[0]["phase"], "truncated");
            assert_eq!(events(&s)[0]["incomplete"], true);
            assert!(s.bytes <= MAX_BYTES);
        }
    }
    #[test]
    fn ipc_parent_idle_transport_flush_does_not_invent_incompleteness() {
        let mut s = Store::default();
        let key = (1, stamp(2, 4));
        s.admit(key, "Read");
        s.finish(key, "Read", [0; 6], true, false);
        s.close(1);
        assert_eq!(events(&s).len(), 1);
        assert_eq!(events(&s)[0]["incomplete"], false);
        assert_eq!(events(&s)[0]["pending_dispatches"], 0);
        assert!(s.groups.is_empty());
    }
    #[test]
    fn ipc_parent_close_before_receipt_admission_marks_late_segment_incomplete() {
        let mut s = Store::default();
        let key = (1, stamp(2, 10));
        s.admit(key, "Read");
        s.close(1);
        assert_eq!(events(&s)[0]["pending_dispatches"], 1);
        // Exercise the production admission branch after Channel.closed.
        s.await_receipt(key, "Read", 88, true);
        s.finish(key, "Read", [1; 6], true, true);
        let records = events(&s);
        let last = records.last().unwrap();
        assert_eq!(last["phase"], "settled_segment");
        assert_eq!(last["incomplete"], true);
        assert_eq!(total(&s, "abandoned_receipts"), 1);
        assert_eq!(total(&s, "receipts"), 0);
        assert!(s.waiting.is_empty());
        assert!(s.groups.is_empty());
    }
    #[test]
    fn ipc_parent_failed_write_abandonment_survives_command_transition() {
        let mut s = Store::default();
        let old = (1, stamp(2, 10));
        let next = (1, stamp(2, 11));
        s.admit(old, "Read");
        s.await_receipt(old, "Read", 88, false);
        let receipt = receipt_key(1, old.1, 88);
        s.abandon(receipt);
        s.finish(old, "Read", [1; 6], false, false);
        s.admit(next, "Other");
        let records = events(&s);
        assert_eq!(records[0]["phase"], "command_transition");
        assert_eq!(records[0]["incomplete"], true);
        assert_eq!(total(&s, "failed"), 1);
        assert_eq!(total(&s, "abandoned_receipts"), 1);
        assert_eq!(total(&s, "receipts"), 0);
        s.finish(next, "Other", [0; 6], true, false);
        s.close(1);
    }
}
